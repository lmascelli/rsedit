//! Visiting a file and writing it back: `find-file`, `save-buffer`,
//! `write-file`, `revert-buffer` and `recover-file`, with the halves their
//! questions call once answered.
//!
//! Design: `doc/architettura-editor.typ, <file-su-disco>`.
use super::*;
use crate::jobs::autosave::{auto_save_directory, auto_save_path};
use crate::primitives::args;
use crate::primitives::file_names::expand_path;
use risp::eval;

// TODO(FIL-1): a failed write is said in the log only.
// TODO(FIL-2): the file is written in place, as by `save-buffer`.
// TODO(FIL-4): an edit made during the write is marked saved with the rest.
/// Writes NAME's buffer to the file it is visiting, and answers false when it
/// has none or the write failed. What `save-some-buffers` saves with.
pub(super) fn write_buffer<B: BufferTrait>(
    ctx: &EditorState<B>,
    name: &str,
    directory: Option<&str>,
) -> bool {
    // Taken under the lock; written with the lock given back, because a slow
    // disk must not stop every other thread reading the buffer.
    let Some(Some((path, content))) = ctx.with_buffer(name, |buf| {
        buf.file_path
            .as_ref()
            .map(|path| (path.to_string(), buf.text.to_string()))
    }) else {
        return false;
    };
    match std::fs::write(&path, content) {
        Ok(()) => {
            let stamp = crate::buffer::FileStamp::of(&path);
            ctx.with_buffer_mut(name, |buf| {
                buf.is_modified = false;
                buf.file_stamp = stamp;
                buf.stale = false;
            });
            ctx.discard_auto_save(name, directory);
            ctx.log_diagnostic(&format!("Wrote {path}"));
            true
        }
        Err(why) => {
            ctx.log_diagnostic(&format!("Failed to save {path}: {why}"));
            false
        }
    }
}

pub const FIND_FILE_DOC: &str = "(find-file PATH): Open the file at PATH in a buffer, make it \
         current, and return the buffer's name. A file open already is gone back to; a name \
         another file's buffer has gets a suffix, as in name<2>. A PATH that does not exist gives \
         an empty buffer, which saving creates. A directory is handed to \
         `*open-directory-callback*', and its answer returned. A file that cannot be read gives \
         nil.\n\n\
         Example:\n\
         (find-file \"~/notes.txt\")";

/// Returns the buffer name for PATH: its file name, made unique.
///
/// The plain file name, with `<2>`, `<3>`... appended while that name is taken
/// by another buffer, so that two `mod.rs` from different directories can
/// both be open. A buffer already visiting this exact file comes back as
/// `Err`: there is nothing to open, only a buffer to go to.
fn buffer_name_for<B: BufferTrait>(ctx: &EditorState<B>, path: &str) -> Result<String, String> {
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
    let path_str = expand_path(path_str);
    let path = std::path::Path::new(&path_str);

    // A directory is somebody else's job. The editor knows only that there is
    // a variable to ask; what answers it -- `dired', or something else
    // entirely -- it never learns.
    if path.is_dir() {
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

    let buf_name = match buffer_name_for(ctx, &path_str) {
        // Already open. Going back to it is what was meant, and reading the
        // file again would throw away whatever had been typed into it.
        Err(existing) => {
            ctx.switch_to_buffer(&existing);
            return Ok(ELispExp::string(existing));
        }
        Ok(name) => name,
    };

    if !path.exists() {
        let name = ctx.new_file_buffer(&buf_name, &path_str, None, &env);
        // Said out loud, because an empty buffer for a file that does not
        // exist looks exactly like an empty buffer for one that does -- and
        // the difference matters when the reason is a typo in the path.
        ctx.set_echo_message("(New file)");
        return Ok(ELispExp::string(name));
    }

    match ctx.new_buffer(&buf_name, Some(&path_str), None, &env) {
        Some(buf_name) => {
            // Said on the way in, because nothing else would say it. A
            // recovery copy nobody is told about is a recovery copy nobody
            // uses, and the moment it matters is exactly the moment somebody
            // reopens the file after a crash.
            if auto_save_path(&path_str, auto_save_directory(&env).as_deref()).exists() {
                ctx.set_echo_message(
                    "An auto-save file exists for this -- M-x recover-file to see it",
                );
            }
            Ok(ELispExp::string(buf_name))
        }
        // TODO(FIL-3): a file that cannot be read, one not in UTF-8 among
        // them, gives nil, and only the log says why.
        None => Ok(ELispExp::nil()),
    }
});

pub const SAVE_BUFFER_DOC: &str = "(save-buffer): Write the current buffer to its file, asking \
         first when the file has changed on disk since it was read. A buffer with no file, or a \
         write that fails, writes nothing and says why in the log.";

// TODO(FIL-1): a buffer with no file and a failed write are said in the log
// only; the echo area stays as it was.
// TODO(FIL-2): the file is written in place, and a write that stops halfway
// leaves it cut short.
// TODO(FIL-4): an edit made while the text is being written is marked saved
// with the rest.
primitive!(save_buffer, _args, env, ctx, {
    // What to write is taken under the lock; the write itself happens with
    // the lock given back, because a slow disk must not stop every other
    // thread reading the buffer.
    let Some((path, content, stale)) = ctx.with_current_buffer(|buf| {
        buf.file_path
            .as_ref()
            .map(|path| (path.to_string(), buf.text.to_string(), buf.stale))
    }) else {
        ctx.log_diagnostic("No file associated with this buffer");
        return Ok(ELispExp::nil());
    };

    // The moment the watcher's flag is for. Somebody else wrote this file
    // since it was read, so writing now replaces their work with a version
    // that never saw it -- and the save is where that can still be stopped,
    // which is why noticing the change did not prompt at the time.
    if stale {
        return eval(
            &ELispExp::form(vec![
                ELispExp::symbol("yes-or-no".into()),
                ELispExp::string(format!(
                    "{path} changed on disk since you read it. Overwrite it?"
                )),
                args::quoted(ELispExp::symbol("save-buffer--overwrite".into())),
            ]),
            env,
            ctx,
        );
    }
    match std::fs::write(&path, content) {
        Ok(_) => {
            // Stamped after the write, or the file we just made would differ
            // from what we remember of it and the editor would report a
            // conflict with itself on the next check.
            let stamp = crate::buffer::FileStamp::of(&path);
            ctx.with_current_buffer_mut(|buf| {
                buf.is_modified = false;
                buf.file_stamp = stamp;
                buf.stale = false;
            });
            // The recovery copy has done its job: what it held is now in the
            // file it was standing in for.
            let name = ctx.get_current_buffer_name();
            ctx.discard_auto_save(&name, auto_save_directory(&env).as_deref());
            ctx.log_diagnostic(&format!("Wrote {}", path));
            Ok(ELispExp::nil())
        }
        Err(e) => {
            ctx.log_diagnostic(&format!("Failed to save: {}", e));
            Ok(ELispExp::nil())
        }
    }
});

pub const SAVE_BUFFER_OVERWRITE_DOC: &str = "(save-buffer--overwrite): Write the current buffer \
         over its file, asking nothing. Not called directly -- it is what `save-buffer' does once \
         a conflict has been confirmed.";

// TODO(FIL-1): a failed write is said in the log only, as by `save-buffer`.
// TODO(FIL-2): the file is written in place, as by `save-buffer`.
// TODO(FIL-4): an edit made during the write is marked saved, as by
// `save-buffer`.
primitive!(save_buffer_overwrite, _args, env, ctx, {
    let Some((path, content)) = ctx.with_current_buffer(|buf| {
        buf.file_path
            .as_ref()
            .map(|path| (path.to_string(), buf.text.to_string()))
    }) else {
        return Ok(ELispExp::nil());
    };
    match std::fs::write(&path, content) {
        Ok(()) => {
            let stamp = crate::buffer::FileStamp::of(&path);
            ctx.with_current_buffer_mut(|buf| {
                buf.is_modified = false;
                buf.file_stamp = stamp;
                buf.stale = false;
            });
            let name = ctx.get_current_buffer_name();
            ctx.discard_auto_save(&name, auto_save_directory(&env).as_deref());
            ctx.log_diagnostic(&format!("Wrote {path}"));
        }
        Err(why) => ctx.log_diagnostic(&format!("Failed to save: {why}")),
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
    let copy = auto_save_path(&path, auto_save_directory(&env).as_deref());
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
    let copy = auto_save_path(&path, auto_save_directory(&env).as_deref());
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
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(why) => {
            ctx.set_echo_message(&format!("Cannot read {path}: {why}"));
            return false;
        }
    };
    let stamp = crate::buffer::FileStamp::of(path);
    ctx.with_current_buffer_mut(|buf| {
        buf.adopt_text(&text);
        buf.file_stamp = stamp;
    });
    ctx.set_echo_message(&format!("Reverted to {path}"));
    true
}

pub const WRITE_FILE_DOC: &str = "(write-file PATH): Write the current buffer to PATH and visit \
         that file from then on; return PATH, or nil when nothing was written. The buffer takes \
         the new file's name and its mode is chosen again, and the old file is left as it was. A \
         PATH another buffer is visiting is refused.\n\n\
         Example:\n\
         (write-file \"~/notes-v2.txt\")";

// TODO(FIL-2): the file is written in place, as by `save-buffer`.
// TODO(FIL-4): an edit made during the write is marked saved, as by
// `save-buffer`.
primitive!(write_file, args, env, ctx, {
    let Some(ELispExp::String(path_str)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let path = expand_path(path_str);
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

    // Taken under the lock, written with the lock given back: a slow disk must
    // not stop every other thread reading the buffer.
    let content = ctx.with_current_buffer(|buf| buf.text.to_string());
    if let Err(why) = std::fs::write(&path, content) {
        // Nothing has been changed yet, deliberately: a failed write leaves
        // the buffer visiting whatever it was visiting before, so the next
        // `C-x C-s` still goes somewhere that works.
        ctx.set_echo_message(&format!("Failed to write {path}: {why}"));
        ctx.log_diagnostic(&format!("write-file: {path}: {why}"));
        return Ok(ELispExp::nil());
    }

    // Only now, with the bytes safely down.
    let mode = ctx.auto_mode_for(&path);
    let visited_file_changed =
        ctx.with_current_buffer(|buf| buf.file_path.as_deref() != Some(path.as_str()));
    let stamp = crate::buffer::FileStamp::of(&path);
    // Discarded against the *old* path, before the buffer is repointed: the
    // recovery copy that exists is the one named after where it used to live.
    ctx.discard_auto_save(&current, auto_save_directory(&env).as_deref());
    ctx.with_current_buffer_mut(|buf| {
        buf.file_path = Some(path.clone());
        buf.is_modified = false;
        buf.file_stamp = stamp;
        buf.stale = false;
    });
    if new_name != current {
        ctx.rename_buffer(&current, &new_name);
    }
    if let Some(mode) = mode {
        ctx.change_major_mode(&new_name, &mode, &env);
    }
    if visited_file_changed {
        ctx.run_buffer_hook(&new_name, "after-set-visited-file-name-hook", &env);
    }
    ctx.set_echo_message(&format!("Wrote {path}"));
    ctx.log_diagnostic(&format!("Wrote {path}"));
    Ok(ELispExp::string(path))
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
}
