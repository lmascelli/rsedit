//! Visiting a file and writing it back: `find-file`, `save-buffer`,
//! `write-file`, `revert-buffer` and `recover-file`, with the halves their
//! questions call once answered, and the files visited before.
//!
//! Design: `doc/architettura-editor.typ, <file-su-disco>`.
use super::*;
use crate::buffer::coding::{self, Coding};
use crate::buffer::io_status::{FileIssue, IoCause};
use crate::jobs::autosave::{auto_save_directory, auto_save_path};
use crate::primitives::args;
use risp::eval;
use std::sync::Arc;

/// Writes NAME's buffer to the file it is visiting, and answers false when it
/// has none or the write failed. Either way the echo area says what became of
/// it: a failed save that only the log heard about is work the user believes
/// is safe. What `save-buffer` and `save-some-buffers` save with.
///
/// Runs before-save-hook before taking the text snapshot.
pub(super) fn write_buffer<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    name: &str,
) -> bool {
    write_buffer_expected(ctx, env, name, None)
}

fn write_buffer_expected<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    name: &str,
    confirmed: Option<Option<crate::buffer::FileStamp>>,
) -> bool {
    use crate::buffer::disk::{self, WriteFailure};
    let Some(handle) = ctx.file_handle(name) else {
        return false;
    };
    let Some(path) = ctx.with_buffer(name, |buf| buf.file_path.clone()).flatten() else {
        return false;
    };
    let Some(_reservation) = ctx.reserve_file_write(handle.clone(), &path) else {
        ctx.set_echo_message("File operation already pending; no overlapping save was started");
        return false;
    };
    if !before_save(ctx, env, name) {
        return false;
    }
    let (content, coding, version, stamp, stale) = ctx
        .with_buffer(name, |buf| {
            (
                buf.text.to_string(),
                buf.coding,
                buf.version,
                buf.file_stamp,
                buf.stale,
            )
        })
        .expect("reserved buffer");
    let Some(bytes) = encode_for(ctx, &path, &content, coding) else {
        return false;
    };
    // Even synchronous scripts get a recovery copy before a potentially slow write.
    let recovery =
        ctx.prepare_recovery(&handle, &path, auto_save_directory(env), version, &content);
    if let Err(why) = &recovery {
        ctx.log_diagnostic(&format!("[ERROR] Recovery copy unavailable: {why}"));
    }
    let written = if stale && confirmed.is_none() {
        Err(WriteFailure::Conflict(stamp))
    } else {
        disk::write_checked(&path, &bytes, confirmed.unwrap_or(stamp))
    };
    match written {
        Ok(stamp) => {
            let all = mark_written(ctx, name, version, Some(stamp));
            if all && let Ok(copy) = recovery {
                ctx.queue_recovery_cleanup(
                    handle,
                    copy,
                    version,
                    crate::editor::file_io::timeout(env),
                );
            }
            say_written(ctx, &path, all);
            true
        }
        Err(why) => {
            report_write_failure(ctx, name, &path, why);
            false
        }
    }
}

fn report_write_failure<B: BufferTrait>(
    ctx: &EditorState<B>,
    name: &str,
    path: &str,
    why: crate::buffer::disk::WriteFailure,
) {
    use crate::buffer::disk::WriteFailure;
    let conflict = matches!(&why, WriteFailure::Conflict(_));
    let reason = match why {
        WriteFailure::Conflict(_) => {
            ctx.with_buffer_mut(name, |buf| buf.stale = true);
            FileIssue::new(
                IoCause::Other,
                "file changed or disappeared; save again to confirm",
            )
        }
        WriteFailure::Io(why) => FileIssue::from_io(&why),
        WriteFailure::Uncertain(why) => FileIssue::new(IoCause::Uncertain, why),
    };
    ctx.with_buffer_mut(name, |buf| {
        buf.is_modified = true;
        if conflict {
            buf.forget_data(crate::editor::file_io::ERROR);
        } else {
            buf.put_data(crate::editor::file_io::ERROR, reason.clone());
        }
    });
    let message = format!("Cannot save {path}: {reason}");
    ctx.set_echo_message(&message);
    ctx.log_diagnostic(&message);
}

const PREPARING_SAVE: &str = "file-io:before-save";

struct SaveHookGuard<B: BufferTrait>(crate::editor::file_io::Handle<B>);
impl<B: BufferTrait> Drop for SaveHookGuard<B> {
    fn drop(&mut self) {
        self.0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .forget_data(PREPARING_SAVE);
    }
}

/// Runs the target buffer's save hooks without locks, before its snapshot.
/// Errors abort the save; a hook cannot recursively prepare the same buffer.
pub(crate) fn before_save<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    name: &str,
) -> bool {
    let Some(handle) = ctx.file_handle(name) else {
        return false;
    };
    let state = {
        let mut buf = handle
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if buf.data::<bool>(PREPARING_SAVE).copied().unwrap_or(false) {
            None
        } else {
            buf.put_data(PREPARING_SAVE, true);
            Some((
                buf.current_mode.clone(),
                buf.minor_modes
                    .iter()
                    .map(|on| on.name.clone())
                    .collect::<Vec<_>>(),
                buf.file_path.clone(),
            ))
        }
    };
    let Some((mode, minors, path)) = state else {
        ctx.set_echo_message("Save already being prepared in this buffer");
        return false;
    };
    let _preparing = SaveHookGuard(handle.clone());
    let Some(previous) = ctx.set_current_buffer(name) else {
        return false;
    };
    let hooks = ctx.modes(|modes| modes.hooks_for(&mode, &minors, "before-save-hook"));
    let target_intact = || {
        ctx.file_handle(name)
            .is_some_and(|current| Arc::ptr_eq(&current, &handle))
            && handle
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .file_path
                == path
    };
    let result = (|| {
        for hook in hooks {
            ctx.set_current_buffer(name);
            let _command = ctx.begin_command();
            eval(&ELispExp::form(vec![hook]), env.clone(), ctx)
                .map_err(|why| format!("{why:?}"))?;
            if !target_intact() {
                return Err("the hook removed, renamed or repointed the buffer".to_string());
            }
        }
        Ok(())
    })();
    ctx.set_current_buffer(&previous);
    if let Err(why) = result {
        let message = format!("Before-save hook failed: {why}; save aborted");
        ctx.set_echo_message(&message);
        ctx.log_diagnostic(&message);
        return false;
    }
    true
}

/// Records that NAME's text as it was at VERSION is now in its file, stamped
/// STAMP, and returns whether that is all of it.
///
/// The text is copied, then written with no lock held, and an edit can come
/// in between, from a job or a callback: the buffer then holds more than the
/// file, and stays modified. Unmodified only when the version is still the
/// one written.
pub(crate) fn mark_written<B: BufferTrait>(
    ctx: &EditorState<B>,
    name: &str,
    version: u64,
    stamp: Option<crate::buffer::FileStamp>,
) -> bool {
    ctx.with_buffer_mut(name, |buf| {
        buf.file_stamp = stamp;
        buf.stale = false;
        buf.forget_data(crate::editor::file_io::ERROR);
        let all_of_it = buf.version == version;
        if all_of_it {
            buf.is_modified = false;
        }
        all_of_it
    })
    .unwrap_or(false)
}

/// Says that PATH was written, and, when ALL_OF_IT is false, that the buffer
/// changed while it was and is still modified.
fn say_written<B: BufferTrait>(ctx: &EditorState<B>, path: &str, all_of_it: bool) {
    let said = if all_of_it {
        format!("Wrote {path}")
    } else {
        format!("Wrote {path}; the buffer changed meanwhile, and is still modified")
    };
    ctx.set_echo_message(&said);
    ctx.log_diagnostic(&said);
}

/// Returns CONTENT as CODING writes it to PATH, or `None`, saying why, when
/// CODING has no bytes for one of its characters: the file is left as it was,
/// since a character written as something else would not come back.
fn encode_for<B: BufferTrait>(
    ctx: &EditorState<B>,
    path: &str,
    content: &str,
    coding: Coding,
) -> Option<Vec<u8>> {
    match coding::encode(content, coding) {
        Ok(bytes) => Some(bytes),
        Err(missing) => {
            let why = format!(
                "Cannot save {path} in {}: it has no {missing:?}; M-x set-buffer-file-coding to \
                 write it in another",
                coding.name()
            );
            ctx.set_echo_message(&why);
            ctx.log_diagnostic(&why);
            None
        }
    }
}

pub const FIND_FILE_DOC: &str = "(find-file PATH): Open the file at PATH in a buffer, make it \
         current, and return the buffer's name. A file open already is gone back to; a name \
         another file's buffer has gets a suffix, as in name<2>. A PATH that does not exist gives \
         an empty buffer, which saving creates. A directory is handed to \
         `*open-directory-callback*', and its answer returned. A file that is not UTF-8 is read \
         as windows-1252, and saved in it; one that cannot be read gives nil. Point goes where \
         the file was last left, unless `save-place' is nil.\n\n\
         Example:\n\
         (find-file \"~/notes.txt\")";

/// Returns the buffer name for PATH: its file name, made unique.
///
/// The plain file name, with `<2>`, `<3>`... appended while that name is taken
/// by another buffer, so that two `mod.rs` from different directories can
/// both be open. A buffer already visiting this exact file comes back as
/// `Err`: there is nothing to open, only a buffer to go to.
pub(super) fn buffer_name_for<B: BufferTrait>(
    ctx: &EditorState<B>,
    path: &str,
) -> Result<String, String> {
    let file_name = std::path::Path::new(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let base = if file_name.is_empty() {
        path.to_string()
    } else {
        file_name
    };
    let visiting = |name: &str| -> Option<String> {
        ctx.with_buffer(name, |buf| buf.file_path.clone()).flatten()
    };
    if !ctx.has_buffer(&base) {
        return Ok(base);
    }
    if visiting(&base).as_deref() == Some(path) {
        return Err(base);
    }
    for n in 2.. {
        let candidate = format!("{base}<{n}>");
        if !ctx.has_buffer(&candidate) {
            return Ok(candidate);
        }
        if visiting(&candidate).as_deref() == Some(path) {
            return Err(candidate);
        }
    }
    unreachable!("the loop above returns")
}

primitive!(find_file, args, env, ctx, {
    let Some(ELispExp::String(path_str)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    // Expanded before anything else looks at it, so that the `~/...` a
    // completion candidate offers is a path that can actually be opened.
    let path_str = ctx.expand_path(path_str);
    let path = std::path::Path::new(&path_str);
    let buf_name = match buffer_name_for(ctx, &path_str) {
        // Already open. Going back to it is what was meant, and reading the
        // file again would throw away whatever had been typed into it.
        Err(existing) => {
            ctx.switch_to_buffer(&existing);
            ctx.record_visit(&path_str, &env);
            return Ok(ELispExp::string(existing));
        }
        Ok(name) => name,
    };

    // A directory is somebody else's job. The editor knows only that there is
    // a variable to ask; what answers it -- `dired', or something else
    // entirely -- it never learns.
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(why) if crate::buffer::io_status::is_missing(&why) => None,
        Err(why) => {
            ctx.with_current_buffer_mut(|buf| {
                buf.put_data(crate::editor::file_io::ERROR, FileIssue::from_io(&why))
            });
            ctx.set_echo_message(&format!("Cannot read {path_str}: {why}"));
            ctx.log_diagnostic(&format!("Cannot read {path_str}: {why}"));
            return Ok(ELispExp::nil());
        }
    };
    if metadata.as_ref().is_some_and(|metadata| metadata.is_dir()) {
        return match env.get_variable(crate::editor::OPEN_DIRECTORY_CALLBACK) {
            Some(callback) if callback.is_truthy() => risp::call_callable(
                &callback,
                &[ELispExp::string(path_str.clone())],
                env.clone(),
                ctx,
            ),
            _ => {
                ctx.set_echo_message(&format!("{path_str} is a directory"));
                Ok(ELispExp::nil())
            }
        };
    }

    if metadata.is_none() {
        let name = ctx.new_file_buffer(&buf_name, &path_str, None, &env);
        ctx.record_visit(&path_str, &env);
        // Said out loud, because an empty buffer for a file that does not
        // exist looks exactly like an empty buffer for one that does -- and
        // the difference matters when the reason is a typo in the path.
        ctx.set_echo_message("(New file)");
        return Ok(ELispExp::string(name));
    }

    match ctx.new_buffer(&buf_name, Some(&path_str), None, &env) {
        Some(buf_name) => {
            ctx.record_visit(&path_str, &env);
            let mut said = Vec::new();
            // A file that is not UTF-8 opens all the same, and says so: its
            // characters are this coding's guess, and saving writes it back
            // in the same coding.
            let coding = ctx
                .with_buffer(&buf_name, |buf| buf.coding)
                .unwrap_or_default();
            if coding == Coding::Windows1252 {
                said.push(format!("Not UTF-8: read as {}", coding.name()));
            }
            // Said on the way in, because nothing else would say it. A
            // recovery copy nobody is told about is a recovery copy nobody
            // uses, and the moment it matters is exactly the moment somebody
            // reopens the file after a crash.
            if auto_save_directory(&env)
                .ok()
                .is_some_and(|directory| auto_save_path(&path_str, directory.as_deref()).exists())
            {
                said.push(
                    "An auto-save file exists for this -- M-x recover-file to see it".to_string(),
                );
            }
            if !said.is_empty() {
                ctx.set_echo_message(&said.join("; "));
            }
            Ok(ELispExp::string(buf_name))
        }
        // `new_buffer` has said why.
        None => Ok(ELispExp::nil()),
    }
});

pub const SAVE_BUFFER_DOC: &str = "(save-buffer): Write the current buffer to its file, asking \
         first when the file has changed on disk since it was read. The echo area says whether it \
         was written; a buffer with no file writes nothing, and says so.";

primitive!(save_buffer, _args, env, ctx, {
    let name = ctx.get_current_buffer_name();
    let Some(handle) = ctx.file_handle(&name) else {
        return Ok(ELispExp::nil());
    };
    let Some((path, stamp, stale, version)) = ctx
        .with_buffer(&name, |buf| {
            buf.file_path
                .clone()
                .map(|path| (path, buf.file_stamp, buf.stale, buf.version))
        })
        .flatten()
    else {
        ctx.set_echo_message("No file associated with this buffer");
        return Ok(ELispExp::nil());
    };
    if ctx.file_io_busy(&format!("file:{path}")) {
        ctx.set_echo_message("File operation already pending; no overlapping save was started");
        return Ok(ELispExp::nil());
    }
    let found = match crate::buffer::FileStamp::try_of(&path) {
        Ok(found) => found,
        Err(why) => {
            ctx.with_buffer_mut(&name, |buf| {
                buf.put_data(crate::editor::file_io::ERROR, FileIssue::from_io(&why))
            });
            ctx.set_echo_message(&format!("Cannot check {path}: {why}"));
            return Ok(ELispExp::nil());
        }
    };
    if stale || found != stamp {
        let prompt = format!("{path} changed or disappeared. Overwrite/recreate it?");
        return super::file_io::ask_confirmation(
            ctx,
            &env,
            path.clone(),
            prompt,
            Box::new(move |state, env| {
                if state.file_handle_name(&handle).is_some()
                    && state.with_buffer(&name, |buf| {
                        buf.version == version && buf.file_path.as_deref() == Some(path.as_str())
                    }) == Some(true)
                {
                    write_buffer_expected(state, env, &name, Some(found));
                } else {
                    state.set_echo_message(
                        "Buffer changed while confirmation was pending; save again",
                    );
                }
            }),
        );
    }
    write_buffer(ctx, &env, &name);
    Ok(ELispExp::nil())
});

pub const SAVE_BUFFER_OVERWRITE_DOC: &str = "(save-buffer--overwrite): Write the current buffer \
         over its file, asking nothing. Not called directly -- it is what `save-buffer' does once \
         a conflict has been confirmed.";

primitive!(save_buffer_overwrite, _args, env, ctx, {
    let name = ctx.get_current_buffer_name();
    if let Some(path) = ctx
        .with_buffer(&name, |buf| buf.file_path.clone())
        .flatten()
    {
        match crate::buffer::FileStamp::try_of(&path) {
            Ok(found) => {
                write_buffer_expected(ctx, &env, &name, Some(found));
            }
            Err(why) => ctx.set_echo_message(&format!("Cannot check {path}: {why}")),
        }
    }
    Ok(ELispExp::nil())
});

pub const RECOVER_FILE_DOC: &str = "(recover-file): Put the auto-saved copy of the current \
         buffer's file into the buffer, after asking, and leave it unsaved: saving keeps it, and \
         `revert-buffer' goes back to the file.";

primitive!(recover_file, _args, env, ctx, {
    let Some(path) = ctx.with_current_buffer(|buf| buf.file_path.clone()) else {
        ctx.set_echo_message("No file associated with this buffer");
        return Ok(ELispExp::nil());
    };
    let directory = auto_save_directory(&env).map_err(EvalError::RuntimeMessage)?;
    let copy = auto_save_path(&path, directory.as_deref());
    if !copy.exists() {
        ctx.set_echo_message(&format!("No auto-save file for {path}"));
        return Ok(ELispExp::nil());
    }
    eval(
        &ELispExp::form(vec![
            ELispExp::symbol("yes-or-no".into()),
            ELispExp::string(format!(
                "Replace this buffer with the auto-saved {}?",
                copy.display()
            )),
            args::quoted(ELispExp::symbol("recover-file--adopt".into())),
        ]),
        env,
        ctx,
    )
});

pub const RECOVER_FILE_ADOPT_DOC: &str = "(recover-file--adopt): Read the auto-save file into the \
         current buffer with no questions. Not called directly -- it is what `recover-file' does \
         once its question has been answered.";

primitive!(recover_file_adopt, _args, env, ctx, {
    let Some(path) = ctx.with_current_buffer(|buf| buf.file_path.clone()) else {
        return Ok(ELispExp::nil());
    };
    let directory = auto_save_directory(&env).map_err(EvalError::RuntimeMessage)?;
    let copy = auto_save_path(&path, directory.as_deref());
    let text = match std::fs::read_to_string(&copy) {
        Ok(text) => text,
        Err(why) => {
            ctx.set_echo_message(&format!("Cannot read {}: {why}", copy.display()));
            return Ok(ELispExp::nil());
        }
    };
    ctx.with_current_buffer_mut(|buf| {
        buf.adopt_text(&text);
        // Modified on purpose. What is in the buffer now is *not* what is in
        // the file, and saying otherwise would let it be quit away silently --
        // which is the exact loss the auto-save file exists to undo.
        buf.is_modified = true;
        // The stamp still describes the real file, which has not been touched,
        // so the watcher goes on comparing against the right thing.
    });
    ctx.set_echo_message(&format!(
        "Recovered from {} -- save to keep it, revert-buffer to discard it",
        copy.display()
    ));
    Ok(ELispExp::t())
});

pub const REVERT_BUFFER_DOC: &str = "(revert-buffer): Read the current buffer's file again, \
         asking first when there are unsaved changes to lose. The undo history, the mark and the \
         overlays go with the old text.";

primitive!(revert_buffer, _args, env, ctx, {
    let Some((path, modified)) =
        ctx.with_current_buffer(|buf| buf.file_path.clone().map(|path| (path, buf.is_modified)))
    else {
        ctx.set_echo_message("No file associated with this buffer");
        return Ok(ELispExp::nil());
    };
    if !modified {
        return Ok(ELispExp::boolean(reread(ctx, &path)));
    }
    // There is something to lose, so the answer has to be spelled out. The
    // read happens in the callback, not here -- a question does not return an
    // answer, it says what to do with each one.
    eval(
        &ELispExp::form(vec![
            ELispExp::symbol("yes-or-no".into()),
            ELispExp::string(format!("Discard your changes and reread {path}?")),
            args::quoted(ELispExp::symbol("revert-buffer--reread".into())),
        ]),
        env,
        ctx,
    )
});

pub const REVERT_BUFFER_REREAD_DOC: &str = "(revert-buffer--reread): Read the current buffer's \
         file again with no questions. Not called directly -- it is what `revert-buffer' does \
         once its question has been answered.";

primitive!(revert_buffer_reread, _args, _env, ctx, {
    let Some(path) = ctx.with_current_buffer(|buf| buf.file_path.clone()) else {
        return Ok(ELispExp::nil());
    };
    Ok(ELispExp::boolean(reread(ctx, &path)))
});

/// Replaces the current buffer's text with what is in PATH, and answers
/// false, saying why, when the file cannot be read.
fn reread<B: BufferTrait>(ctx: &EditorState<B>, path: &str) -> bool {
    // Read with no lock held, for the reason every other file operation here
    // does it: a slow disk must not stop the threads walking the buffer.
    let (text, coding, stamp) = match coding::read_stable(path) {
        Ok(read) => read,
        Err(why) => {
            ctx.with_current_buffer_mut(|buf| {
                buf.put_data(crate::editor::file_io::ERROR, FileIssue::from_io(&why))
            });
            ctx.set_echo_message(&format!("Cannot read {path}: {why}"));
            return false;
        }
    };
    ctx.with_current_buffer_mut(|buf| {
        buf.adopt_text(&text);
        buf.coding = coding;
        buf.file_stamp = Some(stamp);
        buf.forget_data(crate::editor::file_io::ERROR);
    });
    if coding == Coding::Windows1252 {
        ctx.set_echo_message(&format!(
            "Reverted to {path}; not UTF-8: read as {}",
            coding.name()
        ));
    } else {
        ctx.set_echo_message(&format!("Reverted to {path}"));
    }
    true
}

pub const WRITE_FILE_DOC: &str = "(write-file PATH): Write the current buffer to PATH and visit \
         that file from then on; return PATH, or nil when nothing was written. The buffer takes \
         the new file's name and its mode is chosen again, and the old file is left as it was. A \
         PATH another buffer is visiting is refused.\n\n\
         Example:\n\
         (write-file \"~/notes-v2.txt\")";

primitive!(write_file, args, env, ctx, {
    let Some(ELispExp::String(path_str)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let path = ctx.expand_path(path_str);
    let current = ctx.get_current_buffer_name();

    // What the buffer should be called once it lives at this path -- and, on
    // the way, whether anybody is already there.
    let new_name = match buffer_name_for(ctx, &path) {
        // A buffer is already visiting this exact file. If it is this one,
        // `write-file` to the path it already has is just a save, and there is
        // nothing to rename.
        Err(existing) if existing == current => current.clone(),
        Err(existing) => {
            ctx.set_echo_message(&format!("{existing} is already visiting {path}"));
            return Ok(ELispExp::nil());
        }
        Ok(name) => name,
    };

    let handle = ctx.file_handle(&current).expect("current buffer");
    let Some(_reservation) = ctx.reserve_file_write(handle.clone(), &path) else {
        ctx.set_echo_message("File operation already pending; no overlapping save was started");
        return Ok(ELispExp::nil());
    };
    let expected = if ctx.with_current_buffer(|buf| buf.file_path.as_deref() == Some(path.as_str()))
    {
        ctx.with_current_buffer(|buf| buf.file_stamp)
    } else {
        match crate::buffer::FileStamp::try_of(&path) {
            Ok(stamp) => stamp,
            Err(why) => {
                ctx.set_echo_message(&format!("Cannot check {path}: {why}"));
                return Ok(ELispExp::nil());
            }
        }
    };
    if !before_save(ctx, &env, &current) {
        return Ok(ELispExp::nil());
    }
    // Taken under the lock, written with the lock given back: a slow disk must
    // not stop every other thread reading the buffer.
    let (content, coding, version) =
        ctx.with_current_buffer(|buf| (buf.text.to_string(), buf.coding, buf.version));
    let Some(bytes) = encode_for(ctx, &path, &content, coding) else {
        return Ok(ELispExp::nil());
    };
    let source = ctx
        .with_current_buffer(|buf| buf.file_path.clone())
        .unwrap_or_else(|| path.clone());
    let recovery = ctx.prepare_recovery(
        &handle,
        &source,
        auto_save_directory(&env),
        version,
        &content,
    );
    if let Err(why) = &recovery {
        ctx.log_diagnostic(&format!("[ERROR] Recovery copy unavailable: {why}"));
    }
    let stamp = match crate::buffer::disk::write_checked(&path, &bytes, expected) {
        Ok(stamp) => Some(stamp),
        Err(why) => {
            report_write_failure(ctx, &current, &path, why);
            return Ok(ELispExp::nil());
        }
    };

    // Only now, with the bytes safely down.
    let mode = ctx.auto_mode_for(&path);
    let visited_file_changed =
        ctx.with_current_buffer(|buf| buf.file_path.as_deref() != Some(path.as_str()));
    // Whether the buffer is still what was written: an edit that came in
    // during the write keeps it modified, and its recovery copy with it.
    ctx.with_current_buffer_mut(|buf| buf.file_path = Some(path.clone()));
    let all_of_it = mark_written(ctx, &current, version, stamp);
    if all_of_it && let Ok(copy) = recovery {
        ctx.queue_recovery_cleanup(handle, copy, version, crate::editor::file_io::timeout(&env));
    }
    if new_name != current {
        ctx.rename_buffer(&current, &new_name);
    }
    if let Some(mode) = mode {
        ctx.change_major_mode(&new_name, &mode, &env);
    }
    if visited_file_changed {
        ctx.record_visit(&path, &env);
        ctx.run_buffer_hook(&new_name, "after-set-visited-file-name-hook", &env);
    }
    say_written(ctx, &path, all_of_it);
    Ok(ELispExp::string(path))
});

pub const BUFFER_FILE_CODING_DOC: &str = "(buffer-file-coding): Say in the echo area which \
         coding the current buffer's file is read and written in, and return its name: utf-8, \
         utf-8-with-signature, or windows-1252, which a file that is not UTF-8 is read in.";

primitive!(buffer_file_coding, _args, _env, ctx, {
    let coding = ctx.with_current_buffer(|buf| buf.coding);
    ctx.set_echo_message(&format!("Coding: {}", coding.name()));
    Ok(ELispExp::string(coding.name().to_string()))
});

pub const SET_BUFFER_FILE_CODING_DOC: &str = "(set-buffer-file-coding CODING): Write the current \
         buffer's file in CODING from the next save on, and mark the buffer modified so that \
         there is a save to make: utf-8, utf-8-with-signature or windows-1252. Return the \
         coding's name, or nil, and a message, for a name that is none of them.\n\n\
         Example:\n\
         (set-buffer-file-coding \"utf-8\")";

primitive!(set_buffer_file_coding, args, _env, ctx, {
    let name = match args.first() {
        Some(ELispExp::String(name)) | Some(ELispExp::Symbol(name)) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String or Symbol".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    let Some(coding) = Coding::named(name.trim()) else {
        let known: Vec<&str> = Coding::ALL.iter().map(|coding| coding.name()).collect();
        ctx.set_echo_message(&format!("No coding called {name}: {}", known.join(", ")));
        return Ok(ELispExp::nil());
    };
    ctx.with_current_buffer_mut(|buf| {
        buf.coding = coding;
        buf.is_modified = true;
    });
    ctx.set_echo_message(&format!("Will be written in {}", coding.name()));
    Ok(ELispExp::string(coding.name().to_string()))
});

pub const RECENT_FILES_DOC: &str = "(recent-files): Return the files opened with `find-file' \
         or written with `write-file', newest first, as a list of absolute paths: at most \
         `recent-files-length' of them, kept from one run to the next.\n\n\
         Example:\n\
         (car (recent-files))";

primitive!(recent_files, _args, _env, ctx, {
    let paths = ctx.visits(|visits| visits.recent());
    Ok(ELispExp::proper_list(
        paths.into_iter().map(ELispExp::string).collect(),
    ))
});

pub const CLEAR_RECENT_FILES_DOC: &str = "(clear-recent-files): Forget every file `recent-files' \
         lists, and return t when there was one. Where point was in each file is kept.";

primitive!(clear_recent_files, _args, _env, ctx, {
    if ctx.visits_mut(|visits| visits.forget_recent()) {
        ctx.set_echo_message("Recent files forgotten");
        Ok(ELispExp::t())
    } else {
        Ok(ELispExp::nil())
    }
});

/// Registers the primitives that visit a file and write it back.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("find-file", find_file, &["fFind file: "], FIND_FILE_DOC);
    into.command("save-buffer", save_buffer, &[], SAVE_BUFFER_DOC);
    into.function(
        "save-buffer--overwrite",
        save_buffer_overwrite,
        SAVE_BUFFER_OVERWRITE_DOC,
    );
    into.command("recover-file", recover_file, &[], RECOVER_FILE_DOC);
    into.command(
        "buffer-file-coding",
        buffer_file_coding,
        &[],
        BUFFER_FILE_CODING_DOC,
    );
    into.command(
        "set-buffer-file-coding",
        set_buffer_file_coding,
        &["sCoding (utf-8, utf-8-with-signature, windows-1252): "],
        SET_BUFFER_FILE_CODING_DOC,
    );
    into.function(
        "recover-file--adopt",
        recover_file_adopt,
        RECOVER_FILE_ADOPT_DOC,
    );
    into.command("revert-buffer", revert_buffer, &[], REVERT_BUFFER_DOC);
    into.function(
        "revert-buffer--reread",
        revert_buffer_reread,
        REVERT_BUFFER_REREAD_DOC,
    );
    into.command("write-file", write_file, &["fWrite file: "], WRITE_FILE_DOC);
    into.function("recent-files", recent_files, RECENT_FILES_DOC);
    into.command(
        "clear-recent-files",
        clear_recent_files,
        &[],
        CLEAR_RECENT_FILES_DOC,
    );
}
