//! Interactive file operations. The synchronous Lisp functions remain
//! available; interactive-function selects these counterparts for commands.
//! Design: `doc/architettura-editor.typ, <file-su-disco>`.
use super::*;
use crate::buffer::io_status::{FileIssue, IoCause};
use crate::buffer::{
    Buffer, FileStamp, coding,
    disk::{self, WriteFailure},
};
use crate::editor::file_io::{self, Completion, ERROR, Handle, Lane};
use crate::jobs::autosave::{auto_save_directory, auto_save_path};
use std::sync::Arc;

enum Read {
    File(String, crate::buffer::coding::Coding, FileStamp),
    Missing,
    Directory,
}

fn read(path: &str) -> std::io::Result<Read> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(why) if crate::buffer::io_status::is_missing(&why) => return Ok(Read::Missing),
        Err(why) => return Err(why),
    };
    if metadata.is_dir() {
        return Ok(Read::Directory);
    }
    coding::read_stable(path).map(|(text, coding, stamp)| Read::File(text, coding, stamp))
}

fn visiting<B: BufferTrait>(ctx: &EditorState<B>, path: &str) -> Option<String> {
    ctx.buffer_names().into_iter().find(|name| {
        ctx.with_buffer(name, |buf| buf.file_path.as_deref() == Some(path)) == Some(true)
    })
}

fn after<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    function: &ELispExp<B>,
    values: &[ELispExp<B>],
) {
    if function.is_truthy() {
        if let Err(why) = risp::call_callable(function, values, env.clone(), ctx) {
            ctx.log_diagnostic(&format!("[ERROR] file completion callback: {why:?}"));
        }
    }
}

pub const FIND_ASYNC_DOC: &str = "(find-file-async PATH &optional AFTER): Start opening PATH \
    without blocking the command thread. Return t when accepted, nil when refused. AFTER is \
    called with the buffer name on success, nil on failure or a directory. The ordinary find-file remains synchronous.\n\n\
    Example:\n\
    (find-file-async \"/mnt/share/notes.txt\")";
primitive!(find_file_async, args, env, ctx, {
    let Some(ELispExp::String(path)) = args.first() else {
        return Err(EvalError::RuntimeMessage(
            "find-file-async needs a path".into(),
        ));
    };
    let path = ctx.expand_path(path);
    let callback = args.get(1).cloned().unwrap_or_else(ELispExp::nil);
    if let Some(name) = visiting(ctx, &path) {
        ctx.switch_to_buffer(&name);
        after(ctx, &env, &callback, &[ELispExp::string(name)]);
        return Ok(ELispExp::t());
    }
    let window = ctx.get_focused_window_id();
    let origin_name = ctx.get_current_buffer_name();
    let origin = ctx.file_handle(&origin_name).expect("current buffer");
    let origin_version = origin
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .version;
    let target = path.clone();
    let recovery = auto_save_directory(&env)
        .ok()
        .map(|directory| auto_save_path(&path, directory.as_deref()));
    let marker = origin.clone();
    let accepted = ctx
        .start_file_io(
            Lane::Read,
            format!("file:{path}"),
            "Opening",
            Some(marker),
            file_io::timeout(&env),
            move || read(&target).map(|read| (read, recovery.is_some_and(|copy| copy.exists()))),
            move |state, env, result| {
                let may_show = state.get_focused_window_id() == window
                    && state.windows(|windows| windows.buffer_of(window))
                        == Some(origin_name.clone())
                    && state.file_handle_name(&origin).is_some()
                    && origin
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .version
                        == origin_version;
                let (result, recovery_exists) = match result {
                    Ok(result) => result,
                    Err(why) => {
                        let message = format!("Cannot open {path}: {why}");
                        state.set_echo_message(&message);
                        state.log_diagnostic(&message);
                        if state.file_handle_name(&origin).is_some() {
                            let mut issue = FileIssue::from_io(&why);
                            issue.message = format!("opening {path}: {why}");
                            origin
                                .write()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .put_data(ERROR, issue);
                        }
                        after(state, env, &callback, &[ELispExp::nil()]);
                        return;
                    }
                };
                if matches!(result, Read::Directory) {
                    if may_show {
                        state.select_window(window);
                        if let Some(function) =
                            env.get_variable(crate::editor::OPEN_DIRECTORY_CALLBACK)
                            && function.is_truthy()
                        {
                            after(state, env, &function, &[ELispExp::string(path)]);
                        }
                    } else {
                        state.set_echo_message(&format!(
                            "Directory ready: {path}; open it again to show it"
                        ));
                    }
                    after(state, env, &callback, &[ELispExp::nil()]);
                    return;
                }
                let candidate = visiting(state, &path)
                    .map_or_else(|| super::files::buffer_name_for(state, &path), Err);
                let created = candidate.is_ok();
                let name = match candidate {
                    Ok(name) => {
                        let mut buf = match result {
                            Read::File(text, coding, stamp) => {
                                let mut buf = Buffer::from_text(&name, &text);
                                buf.coding = coding;
                                buf.file_stamp = Some(stamp);
                                buf
                            }
                            _ => {
                                let mut buf = Buffer::new(&name);
                                buf.put_data(
                                    ERROR,
                                    FileIssue::from_io(&std::io::Error::from(
                                        std::io::ErrorKind::NotFound,
                                    )),
                                );
                                buf
                            }
                        };
                        buf.file_path = Some(path.clone());
                        buf.current_mode = state
                            .auto_mode_for(&path)
                            .unwrap_or_else(|| "fundamental-mode".into());
                        if let Some(place) = state.place_for(&path, env) {
                            super::movement::goto_offset(&mut buf.text, place);
                        }
                        state.buffers_mut(|buffers| buffers.insert(&name, buf));
                        name
                    }
                    Err(name) => name,
                };
                if may_show {
                    state.select_window(window);
                    state.switch_to_buffer(&name);
                }
                if created {
                    state.initialize_buffer(&name, true, env);
                }
                state.record_visit(&path, env);
                let coding = state
                    .with_buffer(&name, |buf| buf.coding)
                    .unwrap_or_default();
                let mut message = format!("Opened {path}");
                if coding == coding::Coding::Windows1252 {
                    message.push_str("; not UTF-8: read as windows-1252");
                }
                if recovery_exists {
                    message.push_str("; an auto-save copy exists -- M-x recover-file");
                }
                state.set_echo_message(&message);
                after(state, env, &callback, &[ELispExp::string(name)]);
            },
        )
        .is_some();
    if accepted {
        ctx.set_echo_message("Opening file...");
    } else {
        ctx.set_echo_message("File I/O busy: an operation is pending or the read limit is reached");
    }
    Ok(ELispExp::boolean(accepted))
});

#[derive(Clone)]
struct Save {
    path: String,
    source: Option<String>,
    text: Vec<u8>,
    recovery_text: String,
    version: u64,
    expected: Option<FileStamp>,
    stale: bool,
    confirmed: bool,
    recovery: Result<Option<String>, String>,
}

fn start_save<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    handle: Handle<B>,
    save: Save,
    callback: ELispExp<B>,
) -> bool {
    let target = save.path.clone();
    let work = save.clone();
    let origin = handle.clone();
    let recovery_handle = handle.clone();
    let recovery_worker = ctx.clone();
    let accepted = ctx.start_file_io(
        Lane::Write, format!("file:{target}"), "Saving", Some(origin), file_io::timeout(env),
        move || {
            // A local recovery copy precedes the remote write. A cache error
            // is reported, never redirected to the remote file's directory.
            let recovery = recovery_worker.prepare_recovery(&recovery_handle,
                work.source.as_deref().unwrap_or(&work.path), work.recovery.clone(), work.version, &work.recovery_text);
            let result = if work.stale && !work.confirmed {
                FileStamp::try_of(&work.path).map_err(WriteFailure::Io).and_then(|found| Err(WriteFailure::Conflict(found)))
            } else {
                disk::write_checked(&work.path, &work.text, work.expected)
            };
            (result, recovery)
        },
        move |state, env, (result, recovery)| {
            let Some(name) = state.file_handle_name(&handle) else {
                state.log_diagnostic(&format!("[WARNING] File operation finished after buffer removal: {}", save.path));
                return;
            };
            let current = handle.read().unwrap_or_else(std::sync::PoisonError::into_inner).file_path.clone();
            if current != save.source {
                state.set_echo_message(&format!("File operation finished for {}; buffer now visits another file", save.path));
                return;
            }
            if let Err(why) = &recovery {
                state.log_diagnostic(&format!("[ERROR] Recovery copy unavailable: {why}"));
            }
            match result {
                Ok(stamp) => {
                    let candidate = visiting(state, &save.path).map_or_else(|| super::files::buffer_name_for(state, &save.path), Err);
                    let new_name = match candidate {
                        Ok(name) => name,
                        Err(other) if other == name => name.clone(),
                        Err(_) => {
                            handle.write().unwrap_or_else(std::sync::PoisonError::into_inner).is_modified = true;
                            state.set_echo_message("The saved destination was opened in another buffer; buffer kept unsaved");
                            return;
                        }
                    };
                    let changed_path = save.source.as_deref() != Some(save.path.as_str());
                    state.with_buffer_mut(&name, |buf| { buf.file_path = Some(save.path.clone()); buf.forget_data(ERROR); });
                    let all = super::files::mark_written(state, &name, save.version, Some(stamp));
                    if all && let Ok(copy) = recovery { state.queue_recovery_cleanup(handle.clone(), copy, save.version, file_io::timeout(env)); }
                    if new_name != name { state.rename_buffer(&name, &new_name); }
                    if changed_path {
                        if let Some(mode) = state.auto_mode_for(&save.path) { state.change_major_mode(&new_name, &mode, env); }
                        state.record_visit(&save.path, env);
                        state.run_buffer_hook(&new_name, "after-set-visited-file-name-hook", env);
                    }
                    let message = format!("Wrote {}{}", save.path, if all { "" } else { "; newer edits remain unsaved" });
                    state.set_echo_message(&message);
                    state.log_diagnostic(&message);
                    after(state, env, &callback, &[ELispExp::t()]);
                }
                Err(WriteFailure::Conflict(found)) => {
                    {
                        let mut buf = handle.write().unwrap_or_else(std::sync::PoisonError::into_inner);
                        buf.stale = true; buf.forget_data(ERROR);
                    }
                    let key = save.path.clone();
                    let path = save.path.clone();
                    let version = save.version;
                    let mut retry = save;
                    retry.expected = found;
                    retry.confirmed = true;
                    let action: Completion<B> = Box::new(move |state, env| {
                        let valid = state.file_handle_name(&handle).is_some()
                            && {
                                let buf = handle.read().unwrap_or_else(std::sync::PoisonError::into_inner);
                                buf.version == version && buf.file_path == retry.source
                            };
                        if valid { start_save(state, env, handle, retry, callback); }
                        else { state.set_echo_message("Buffer changed while confirmation was pending; save again"); }
                    });
                    if let Err(why) = ask_confirmation(state, env, key,
                        format!("{path} changed or disappeared. Overwrite/recreate it?"), action) {
                        state.log_diagnostic(&format!("[ERROR] save confirmation: {why:?}"));
                    }
                }
                Err(why) => {
                    let reason = match why {
                        WriteFailure::Io(why) => FileIssue::from_io(&why),
                        WriteFailure::Uncertain(why) => FileIssue::new(IoCause::Uncertain, why),
                        _ => unreachable!(),
                    };
                    {
                        let mut buf = handle.write().unwrap_or_else(std::sync::PoisonError::into_inner);
                        buf.put_data(ERROR, reason.clone());
                        buf.is_modified = true;
                    }
                    let message = format!("Cannot save {}: {reason}", save.path);
                    state.set_echo_message(&message);
                    state.log_diagnostic(&message);
                    after(state, env, &callback, &[ELispExp::nil()]);
                }
            }
        },
    ).is_some();
    if accepted {
        ctx.set_echo_message(&format!("Saving {target}..."));
    } else {
        ctx.set_echo_message("File I/O busy: no overlapping save was started");
    }
    accepted
}

pub(super) fn save_named<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    name: &str,
    path: Option<String>,
    callback: ELispExp<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let Some(handle) = ctx.file_handle(name) else {
        return Ok(ELispExp::nil());
    };
    let source = handle
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .file_path
        .clone();
    let Some(path) = path.or_else(|| source.clone()) else {
        ctx.set_echo_message("No file associated with this buffer");
        return Ok(ELispExp::nil());
    };
    if visiting(ctx, &path).is_some_and(|other| other != name) {
        ctx.set_echo_message("Another buffer is already visiting this destination");
        return Ok(ELispExp::nil());
    }
    if ctx.file_io_busy(&format!("file:{path}")) {
        ctx.set_echo_message("File operation already pending; no overlapping save was started");
        return Ok(ELispExp::nil());
    }
    if !super::files::before_save(ctx, env, name) {
        return Ok(ELispExp::nil());
    }
    let request = {
        let buf = handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = coding::encode(&buf.text.to_string(), buf.coding).map_err(|why| {
            EvalError::RuntimeMessage(format!("Cannot encode file character: {why:?}"))
        })?;
        let same_target = source.as_deref() == Some(path.as_str());
        Save {
            expected: if source.as_deref() == Some(path.as_str()) {
                buf.file_stamp
            } else {
                None
            },
            path,
            source,
            text: bytes,
            recovery_text: buf.text.to_string(),
            version: buf.version,
            stale: same_target && buf.stale,
            confirmed: false,
            recovery: auto_save_directory(env),
        }
    };
    Ok(ELispExp::boolean(start_save(
        ctx, env, handle, request, callback,
    )))
}

pub const SAVE_ASYNC_DOC: &str = "(save-buffer-async &optional AFTER): Start saving this buffer \
    in the background. Return t when accepted. AFTER receives t on success or nil on an I/O \
    failure. A timeout warns but does not cancel the write. Newer edits stay unsaved.\n\n\
    Example:\n\
    (save-buffer-async)";
primitive!(save_buffer_async, args, env, ctx, {
    save_named(
        ctx,
        &env,
        &ctx.get_current_buffer_name(),
        None,
        args.first().cloned().unwrap_or_else(ELispExp::nil),
    )
});

pub const WRITE_ASYNC_DOC: &str = "(write-file-async PATH &optional AFTER): Save to PATH in the \
    background and visit it only after success. Return t when accepted. AFTER receives t or nil, \
    as for save-buffer-async. Existing destinations require confirmation.\n\n\
    Example:\n\
    (write-file-async \"~/local-copy.txt\")";
primitive!(write_file_async, args, env, ctx, {
    let Some(ELispExp::String(path)) = args.first() else {
        return Err(EvalError::RuntimeMessage(
            "write-file-async needs a path".into(),
        ));
    };
    save_named(
        ctx,
        &env,
        &ctx.get_current_buffer_name(),
        Some(ctx.expand_path(path)),
        args.get(1).cloned().unwrap_or_else(ELispExp::nil),
    )
});

fn start_revert<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    handle: Handle<B>,
    path: String,
    version: u64,
) -> bool {
    let target = path.clone();
    ctx.start_file_io(
        Lane::Read,
        format!("file:{path}"),
        "Reverting",
        Some(handle.clone()),
        file_io::timeout(env),
        move || read(&target),
        move |state, _, result| {
            let Some(name) = state.file_handle_name(&handle) else {
                return;
            };
            let valid = {
                let buf = handle
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                buf.version == version && buf.file_path.as_deref() == Some(path.as_str())
            };
            if !valid {
                state.set_echo_message("Buffer changed during the read; revert discarded");
                return;
            }
            match result {
                Ok(Read::File(text, coding, stamp)) => {
                    state.with_buffer_mut(&name, |buf| {
                        buf.adopt_text(&text);
                        buf.coding = coding;
                        buf.file_stamp = Some(stamp);
                        buf.forget_data(ERROR);
                    });
                    state.set_echo_message(&format!("Reverted to {path}"));
                }
                other => {
                    let why = match other {
                        Err(why) => FileIssue::from_io(&why),
                        Ok(Read::Missing) => {
                            FileIssue::from_io(&std::io::Error::from(std::io::ErrorKind::NotFound))
                        }
                        _ => FileIssue::from_io(&std::io::Error::from(
                            std::io::ErrorKind::IsADirectory,
                        )),
                    };
                    handle
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .put_data(ERROR, why.clone());
                    state.set_echo_message(&format!(
                        "Cannot revert {path}: {why}; buffer preserved"
                    ));
                }
            }
        },
    )
    .is_some()
}

pub const REVERT_ASYNC_DOC: &str = "(revert-buffer-async): Reread the visited file in the background, \
    asking before discarding unsaved changes. Edits made while the read is pending are preserved. \
    Return t when a read or confirmation was accepted, nil otherwise.";
primitive!(revert_buffer_async, _args, env, ctx, {
    let name = ctx.get_current_buffer_name();
    let Some(handle) = ctx.file_handle(&name) else {
        return Ok(ELispExp::nil());
    };
    let (path, version, modified) = {
        let buf = handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (buf.file_path.clone(), buf.version, buf.is_modified)
    };
    let Some(path) = path else {
        ctx.set_echo_message("No file associated with this buffer");
        return Ok(ELispExp::nil());
    };
    if ctx.file_io_busy(&format!("file:{path}")) {
        ctx.set_echo_message("File operation already pending; revert refused");
        return Ok(ELispExp::nil());
    }
    if modified {
        return ask_confirmation(
            ctx,
            &env,
            format!("revert:{path}"),
            format!("Discard your changes and reread {path}?"),
            Box::new(move |state, env| {
                if state.file_handle_name(&handle).is_some()
                    && handle
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .version
                        == version
                {
                    start_revert(state, env, handle, path, version);
                } else {
                    state.set_echo_message(
                        "Buffer changed while confirmation was pending; revert again",
                    );
                }
            }),
        );
    }
    Ok(ELispExp::boolean(start_revert(
        ctx, &env, handle, path, version,
    )))
});

pub const CONFIRM_DOC: &str = "(file-save--confirm ID): Continue a save after its conflict \
    question was answered. Private callback; a stale confirmation writes nothing.";
primitive!(file_save_confirm, args, env, ctx, {
    if let Some(ELispExp::Number(id)) = args.first() {
        ctx.confirm_file_io(*id as u64, &env);
    }
    Ok(ELispExp::nil())
});

pub const CANCEL_DOC: &str =
    "(file-save--cancel ID): Forget a cancelled file confirmation. Private callback.";
primitive!(file_save_cancel, args, _env, ctx, {
    if let Some(ELispExp::Number(id)) = args.first() {
        ctx.cancel_file_confirmation(*id as u64);
    }
    Ok(ELispExp::nil())
});

pub(super) fn ask_confirmation<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    key: String,
    prompt: String,
    action: Completion<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let id = ctx.remember_file_confirmation(key, action);
    let answer = |function: &str| {
        ELispExp::lambda(risp::Lambda {
            params: Vec::new(),
            optionals: Vec::new(),
            rest: None,
            env: env.clone(),
            doc: None,
            body: vec![ELispExp::form(vec![
                ELispExp::symbol(function.into()),
                ELispExp::number(id as f64),
            ])],
        })
    };
    super::ask::yes_or_no(
        &[
            ELispExp::string(prompt),
            answer("file-save--confirm"),
            answer("file-save--cancel"),
        ],
        env.clone(),
        ctx,
    )
}

pub const FILE_STATUS_DOC: &str = "(file-io-status): Show and return this buffer's recorded file \
    error, its OS details and the last recovery attempt. This performs no filesystem I/O.";
primitive!(file_io_status, _args, _env, ctx, {
    let text = ctx.with_current_buffer(|buf| buf.file_io_detail());
    ctx.set_echo_message(&text);
    ctx.log_diagnostic(&text);
    Ok(ELispExp::string(text))
});

pub const AUTO_STATUS_DOC: &str = "(auto-save-status): Show and return the last recovery attempt \
    for this buffer: UTC date, outcome, destination, version and last success, or never attempted. \
    Manual saves' recovery copies count too. This performs no filesystem I/O.";
primitive!(auto_save_status, _args, _env, ctx, {
    let text = ctx.with_current_buffer(|buf| buf.auto_save_status());
    ctx.set_echo_message(&text);
    ctx.log_diagnostic(&text);
    Ok(ELispExp::string(text))
});

pub fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("file-io-status", file_io_status, &[], FILE_STATUS_DOC);
    into.command("auto-save-status", auto_save_status, &[], AUTO_STATUS_DOC);
    into.command(
        "revert-buffer-async",
        revert_buffer_async,
        &[],
        REVERT_ASYNC_DOC,
    );
    into.command(
        "find-file-async",
        find_file_async,
        &["fFind file: "],
        FIND_ASYNC_DOC,
    );
    into.command("save-buffer-async", save_buffer_async, &[], SAVE_ASYNC_DOC);
    into.command(
        "write-file-async",
        write_file_async,
        &["fWrite file: "],
        WRITE_ASYNC_DOC,
    );
    into.function("file-save--cancel", file_save_cancel, CANCEL_DOC);
    into.function("file-save--confirm", file_save_confirm, CONFIRM_DOC);
    for (sync, asynchronous) in [
        ("revert-buffer", "revert-buffer-async"),
        ("find-file", "find-file-async"),
        ("save-buffer", "save-buffer-async"),
        ("write-file", "write-file-async"),
    ] {
        into.env.put_property(
            sync,
            "interactive-function",
            ELispExp::symbol(asynchronous.into()),
        );
    }
}
