use super::*;
use crate::lisp::{call_callable, eval};
use crate::primitives::ask;

pub const FIND_FILE_DOC: &str = "(find-file PATH): Open PATH into a buffer, make it current, and \
         return the buffer's name.\n\n\
         The buffer is named after PATH's file name, or after PATH itself when it has none. A \
         name already taken by a buffer visiting a *different* file gets a suffix -- \
         `mod.rs<2>' -- so that two files of the same name in different directories can both be \
         open. Opening a file that is already open returns to its buffer rather than reading it \
         again.\n\n\
         A PATH that does not exist yet is not an error: an empty buffer is made that remembers \
         where it goes, and `save-buffer' creates the file. It is *not* marked modified, since \
         nothing has been written -- so leaving without saving loses nothing and asks nothing. \
         The echo area says \"(New file)\", which is the only thing distinguishing it from an \
         empty file that does exist.\n\n\
         A PATH that is a directory is handed to `*open-directory-callback*' -- the function \
         `dired' installs when it loads -- and its answer is returned. With nothing installed, \
         this reports that PATH is a directory rather than failing obscurely on the read. That \
         variable is the whole of what the editor knows about file managers: nothing here \
         mentions `dired', and a different one can be installed instead.\n\n\
         Returns nil (logging a diagnostic) if the file exists but cannot be read.\n\n\
         Example:\n\
         (find-file \"/home/me/notes.txt\") => \"notes.txt\"";

/// The buffer name to use for PATH: its file name, made unique.
///
/// A buffer already visiting this exact file is returned as `Err`, meaning
/// "there is nothing to open, go there". Otherwise the plain file name, with
/// `<2>`, `<3>`... appended if that name is taken by something else -- which is
/// what lets two `mod.rs` from different directories both be open, the case
/// that made this worth doing at all.
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
        return match env.get_variable("*open-directory-callback*") {
            Some(callback) if callback.is_truthy() => crate::lisp::call_callable(
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
        let name = ctx.new_file_buffer(&buf_name, &path_str, None);
        // Said out loud, because an empty buffer for a file that does not
        // exist looks exactly like an empty buffer for one that does -- and
        // the difference matters when the reason is a typo in the path.
        ctx.set_echo_message("(New file)");
        return Ok(ELispExp::string(name));
    }

    match ctx.new_buffer(&buf_name, Some(&path_str), None) {
        Some(buf_name) => Ok(ELispExp::string(buf_name)),
        None => Ok(ELispExp::nil()),
    }
});

pub const SAVE_BUFFER_DOC: &str = "(save-buffer): Write the current buffer's contents to the file it \
         was visiting. Returns nil in every case (logging a diagnostic \
         either way); if the buffer has no associated file, or the write \
         fails, nothing is written.\n\n\
         Example:\n\
         (define-key nil \"C-x C-s\" 'save-buffer)";

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
                quoted(ELispExp::symbol("save-buffer--overwrite".into())),
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
            ctx.log_diagnostic(&format!("Wrote {}", path));
            Ok(ELispExp::nil())
        }
        Err(e) => {
            ctx.log_diagnostic(&format!("Failed to save: {}", e));
            Ok(ELispExp::nil())
        }
    }
});

pub const REVERT_BUFFER_DOC: &str = "(revert-buffer): Read the current buffer's file again, \
         throwing away anything unsaved. Returns t if it was read, nil otherwise.\n\n\
         Asks first when there is something to lose -- typing `yes' -- and simply reads when \
         there is not. The undo history, the mark and any overlays go with the old text: all \
         three describe positions in text that no longer exists, and undoing back into a state \
         the file never had would be worse than not being able to undo. Point stays roughly where \
         it was, clamped into the new contents.\n\n\
         Most of the time nothing has to run this. A buffer with no unsaved edits is reloaded on \
         its own when the file changes -- see `watch-files' -- and this is the way to do it \
         deliberately, or to abandon edits in favour of what is on disk.\n\n\
         Example:\n\
         (revert-buffer)";

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
            quoted(ELispExp::symbol("revert-buffer--reread".into())),
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

/// Replace the current buffer's text with what is in PATH.
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
         it from now on. Returns PATH, or nil if nothing was written.\n\n\
         What \"save as\" means: the text goes to the new file, later `save-buffer\'s go there \
         too, and the buffer is renamed after it -- with `<2>\' appended if that name is taken, \
         the same way `find-file\' makes two `mod.rs\' tell each other apart. The mode is \
         reconsidered from the new name, so saving a scratch buffer as `x.rs\' colours it.\n\n\
         The old file, if there was one, is left exactly as it is. This does not move a file; it \
         writes a copy and stops looking at the original.\n\n\
         Refused, writing nothing, when another buffer is already visiting PATH -- two buffers \
         over one file is how one of them silently loses its work.\n\n\
         Example:\n\
         (write-file \"/home/me/notes-v2.txt\")";

primitive!(write_file, args, _env, ctx, {
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
    let stamp = crate::buffer::FileStamp::of(&path);
    ctx.with_current_buffer_mut(|buf| {
        buf.file_path = Some(path.clone());
        buf.is_modified = false;
        buf.file_stamp = stamp;
        buf.stale = false;
        if let Some(mode) = mode {
            buf.current_mode = mode;
        }
    });
    if new_name != current {
        ctx.rename_buffer(&current, &new_name);
    }
    ctx.set_echo_message(&format!("Wrote {path}"));
    ctx.log_diagnostic(&format!("Wrote {path}"));
    Ok(ELispExp::string(path))
});

// ---------------------------------------------------------------------------
// Paths, listings, and what file-name completion is built out of
// ---------------------------------------------------------------------------

/// PATH as an absolute, normalised path: `~` expanded, resolved against the
/// working directory if it is relative, and `.`/`..` collapsed.
///
/// # Why all three and not just the tilde
///
/// This used to expand `~` and nothing else, so a relative path stayed
/// relative -- and a relative path has fewer components than it looks like it
/// has. Opening a listing of `.` gave a header of `./`, and asking for the
/// directory above *that* ran off the front of a one-component path and landed
/// at the filesystem root. One keystroke from the directory you were in to `/`.
///
/// The repair is not to teach the caller to count more carefully. It is that a
/// path is ambiguous until it is absolute, and the one place that should be
/// dealing in ambiguous paths is the moment a user types one.
///
/// # Lexical, not `canonicalize`
///
/// `..` is collapsed by dropping the previous component rather than by asking
/// the filesystem. The two differ through a symlink -- `/a/link/..` is
/// lexically `/a`, and physically the parent of whatever `link` points at --
/// and lexical is the answer a file manager wants: pressing `^` in a listing of
/// `/a/link/` should show `/a/`, which is where the name came from. It is also
/// the only answer available for a path that does not exist yet, which is
/// exactly what the destination of a rename is.
pub(crate) fn expand_path(path: &str) -> String {
    expand_path_in(path, &working_directory())
}

/// PATH made absolute against BASE rather than against the working directory.
///
/// What `expand-file-name`'s second argument is for: a relative name typed into
/// a directory listing means "in the directory being listed", which is not
/// where the editor happens to have been started.
pub(crate) fn expand_path_in(path: &str, base: &std::path::Path) -> String {
    // The base is made absolute too, which is what lets everything below
    // assume it is working on an absolute path. A relative base would hand
    // back a relative answer -- from a function whose whole purpose is that
    // what comes out is absolute -- and the caller would be no better off than
    // before.
    let base = if base.is_absolute() {
        base.to_path_buf()
    } else {
        working_directory().join(base)
    };
    let expanded = expand_tilde(path);
    let joined = if std::path::Path::new(&expanded).is_absolute() {
        std::path::PathBuf::from(expanded)
    } else {
        base.join(expanded)
    };
    normalise(&joined)
}

/// Where the editor is running, or the root if even that cannot be had.
///
/// A failure here means the working directory was deleted out from under the
/// process. The root is a poor answer but a usable one, and better than
/// refusing to expand anything for the rest of the session.
fn working_directory() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from(std::path::MAIN_SEPARATOR_STR))
}

/// Expand a leading `~`, leaving anything else alone.
///
/// Only `~` and `~/...`, deliberately: `~other-user` needs the password
/// database, which is a platform question this editor has no business
/// answering. A path that cannot be expanded is returned unchanged rather than
/// rejected -- a file really can be called `~weird`, and failing to open it
/// would be worse than trying.
fn expand_tilde(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_string();
    };
    if !(rest.is_empty() || rest.starts_with(std::path::is_separator)) {
        return path.to_string();
    }
    // `USERPROFILE` as well as `HOME`, because Windows sets that one and
    // frequently not the other -- and a `~` that silently stayed a `~` would
    // make every path built from it name a directory called `~`.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    match home {
        Some(home) => format!("{home}{rest}"),
        None => path.to_string(),
    }
}

/// Collapse `.` and `..` without touching the filesystem.
///
/// Walks `std::path::Component`s rather than splitting on a character, which is
/// what makes this right on Windows: a `Prefix` (`C:`) and a `RootDir` are
/// components the platform's own parser produced, so a drive letter is never
/// mistaken for a directory name and a backslash is recognised as a separator.
///
/// PATH is always absolute here -- its only caller makes it so -- which is what
/// lets the result be returned without a case for having nothing left. An
/// absolute path keeps its root through every step below: `..` refuses to
/// remove one, and that is the only thing that removes anything.
fn normalise(path: &std::path::Path) -> String {
    use std::path::Component;
    let mut kept: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            // `.` says nothing about where the path goes. Mostly redundant:
            // `Components` already drops these, except one at the very start of
            // a relative path -- and by here the path is absolute, so there is
            // no start for one to be at. Kept so that this function is correct
            // on its own terms rather than on a neighbour's.
            Component::CurDir => {}
            Component::ParentDir => match kept.last() {
                // The only thing `..` may remove is a name.
                Some(Component::Normal(_)) => {
                    kept.pop();
                }
                // Above a root there is nothing, so the root is its own parent
                // -- which is what stops `^` held down from producing a path
                // made of `..`.
                Some(_) => {}
                None => kept.push(component),
            },
            other => kept.push(other),
        }
    }
    let out: std::path::PathBuf = kept.iter().collect();
    out.to_string_lossy().to_string()
}

/// PATH with a trailing separator, so a name can be joined onto it.
pub(crate) fn as_directory(path: &str) -> String {
    if path.ends_with(std::path::is_separator) {
        return path.to_string();
    }
    format!("{path}{}", std::path::MAIN_SEPARATOR)
}

/// PATH without its trailing separator -- the directory named as a *file*,
/// which is the form whose parent can be asked for.
///
/// A root keeps its separator: `/` without it is the empty string, and `C:\`
/// without it is `C:`, which names the drive's working directory rather than
/// its root.
pub(crate) fn without_trailing_separator(path: &str) -> String {
    let trimmed = path.trim_end_matches(std::path::is_separator);
    if trimmed.is_empty() || std::path::Path::new(trimmed).parent().is_none() {
        return path.to_string();
    }
    trimmed.to_string()
}

/// The directory part of PATH -- everything up to and including its last
/// separator -- or nothing when PATH names no directory at all.
///
/// Lexical, like Emacs's `file-name-directory`: `"/a/b"` gives `"/a/"` and
/// `"/a/b/"` gives itself, because the question is about the *name*, not about
/// what is on disk.
pub(crate) fn directory_part(path: &str) -> Option<String> {
    let cut = path.rfind(std::path::is_separator)?;
    Some(path[..=cut].to_string())
}

/// The last component of PATH: everything after its last separator.
pub(crate) fn last_component(path: &str) -> String {
    match path.rfind(std::path::is_separator) {
        Some(cut) => path[cut + 1..].to_string(),
        None => path.to_string(),
    }
}

/// Everything in DIRECTORY, sorted, with a `/` after each subdirectory.
///
/// The trailing slash is the whole reason this is useful for completion: it is
/// what tells the caller -- and the user reading a cycled candidate -- that
/// pressing Tab again will descend rather than confirm, and it is what makes a
/// completed directory name a legal prefix for the next component.
///
/// `.` and `..` are not listed. They are always there, they are never what
/// somebody is looking for, and they would sit at the top of every listing.
pub(crate) fn directory_entries(directory: &str) -> Result<Vec<String>, std::io::Error> {
    directory_entries_ordered(directory, false)
}

/// The same, with directories gathered before files when DIRS_FIRST.
///
/// Ordered here rather than by the caller because the answer is already in
/// hand: `file_type` has been asked for anyway, to decide the trailing
/// separator. A caller sorting afterwards would have to work out what each
/// entry is all over again -- a `stat` and an evaluation apiece from Lisp,
/// which is the cost that made `find-file-recursive` unusable on a large tree.
///
/// Reading the separator back off the name would be cheaper still and is
/// wrong: which character it is depends on the platform, and `dired--visit`
/// already declines to do it for that reason. The separator is there to be
/// read by a person.
pub(crate) fn directory_entries_ordered(
    directory: &str,
    dirs_first: bool,
) -> Result<Vec<String>, std::io::Error> {
    let mut names: Vec<(bool, String)> = Vec::new();
    for entry in std::fs::read_dir(expand_path(directory))? {
        let entry = entry?;
        let mut name = entry.file_name().to_string_lossy().to_string();
        // `file_type` rather than `metadata`, so a symlink to a directory is
        // reported as the link it is instead of failing when it dangles.
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            name.push(std::path::MAIN_SEPARATOR);
        }
        // `false` sorts before `true`, so the flag is "is this a file" when
        // directories are wanted first, and a constant when they are not.
        names.push((dirs_first && !is_dir, name));
    }
    names.sort();
    Ok(names.into_iter().map(|(_, name)| name).collect())
}

pub const LIST_DIR_DOC: &str = "(list-dir &optional DIRECTORY ORDER): Return the names of \
         everything in DIRECTORY -- the current directory if it is omitted -- as a sorted list of \
         strings, with a \"/\" after each subdirectory. `.' and `..' are not included, and a \
         leading \"~\" is expanded to the home directory.\n\n\
         ORDER is 'name for one alphabetical run, which is the default, or 'type for the \
         subdirectories first and then the files, each run alphabetical. Ordering is done here \
         rather than by the caller because what each entry is has already been asked of the \
         filesystem; sorting afterwards means asking again, once per entry.\n\n\
         Returns nil (logging a diagnostic) if the directory can't be read.\n\n\
         Example:\n\
         (list-dir \"/etc\" 'type) => (\"ssh/\" \"hosts\" ...)";

primitive!(list_dir, args, _env, ctx, {
    if args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let directory = match args.first() {
        None => ".".to_string(),
        Some(exp) if exp.is_nil() => ".".to_string(),
        Some(ELispExp::String(path)) => path.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let dirs_first = match args.get(1) {
        None => false,
        Some(exp) if exp.is_nil() => false,
        Some(ELispExp::Symbol(order)) if order.as_str() == "type" => true,
        Some(ELispExp::Symbol(order)) if order.as_str() == "name" => false,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "'name or 'type".into(),
                got: other.clone(),
            });
        }
    };
    match directory_entries_ordered(&directory, dirs_first) {
        Ok(names) => Ok(ELispExp::proper_list(
            names.into_iter().map(ELispExp::string).collect(),
        )),
        Err(why) => {
            ctx.log_diagnostic(&format!("Cannot list {directory}: {why}"));
            Ok(ELispExp::nil())
        }
    }
});

pub const MATCH_LIST_DOC: &str = "(match-list LIST PATTERN): Return the elements of LIST that \
         contain PATTERN, in the order they appear. Non-string elements are skipped, so a list \
         that is not all strings narrows rather than signalling.\n\n\
         Containment, not prefix: this is for narrowing a listing to what is interesting, which \
         is a different question from completing a prefix.\n\n\
         Example:\n\
         (match-list (list-dir \"/etc\") \".conf\")";

primitive!(match_list, args, _env, _ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let ELispExp::String(pattern) = &args[1] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[1].clone(),
        });
    };
    Ok(ELispExp::proper_list(
        args[0]
            .iter()
            .filter(|item| match item {
                ELispExp::String(s) => s.contains(pattern.as_str()),
                _ => false,
            })
            .collect(),
    ))
});

pub const EXPAND_FILE_NAME_DOC: &str = "(expand-file-name PATH &optional DIRECTORY): Return PATH as \
         an absolute path: a leading \"~\" replaced by the home directory, a relative PATH \
         resolved against DIRECTORY (the editor's working directory if it is omitted), and \
         \".\" and \"..\" components collapsed.\n\n\
         An absolute PATH is returned normalised but otherwise as given, so passing one \
         through costs nothing and DIRECTORY is ignored for it.\n\n\
         The collapsing is done on the name, not on the disk: \"/a/link/..\" becomes \"/a\" \
         even when `link' is a symlink pointing elsewhere, and a path that does not exist \
         yet -- the destination of a rename, say -- is expanded like any other.\n\n\
         Example:\n\
         (expand-file-name \"~/notes.txt\")      => \"/home/me/notes.txt\"\n\
         (expand-file-name \"..\" \"/a/b/c/\")     => \"/a/b\"\n\
         (expand-file-name \"draft.md\" \"/a/b/\") => \"/a/b/draft.md\"";

primitive!(expand_file_name, args, _env, _ctx, {
    let path = match args.first() {
        Some(ELispExp::String(path)) => path.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    Ok(ELispExp::string(match args.get(1) {
        None => expand_path(&path),
        Some(exp) if exp.is_nil() => expand_path(&path),
        Some(ELispExp::String(base)) => {
            // The base is expanded too, so that a relative or `~`-prefixed
            // DIRECTORY does not quietly produce a relative answer -- the
            // whole point of this function is that what comes out is absolute.
            expand_path_in(&path, std::path::Path::new(&expand_path(base)))
        }
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    }))
});

pub const FILE_NAME_AS_DIRECTORY_DOC: &str = "(file-name-as-directory PATH): Return PATH with a \
         trailing separator, adding one only if it has none. This is the form a file name can \
         be joined onto with `concat'.\n\n\
         Example:\n\
         (concat (file-name-as-directory \"/etc\") \"hosts\") => \"/etc/hosts\"";

primitive!(file_name_as_directory, args, _env, _ctx, {
    let path = path_string(args.first())?;
    Ok(ELispExp::string(as_directory(&path)))
});

pub const DIRECTORY_FILE_NAME_DOC: &str = "(directory-file-name PATH): Return PATH without its \
         trailing separator -- the directory named as a file, which is the form whose own \
         directory can be asked for.\n\n\
         A root is returned unchanged: it is its own parent, and stripping its separator \
         would leave the empty string.\n\n\
         Example:\n\
         (file-name-directory (directory-file-name \"/a/b/\")) => \"/a/\"   ; the parent";

primitive!(directory_file_name, args, _env, _ctx, {
    let path = path_string(args.first())?;
    Ok(ELispExp::string(without_trailing_separator(&path)))
});

pub const FILE_NAME_DIRECTORY_DOC: &str = "(file-name-directory PATH): Return the directory part of \
         PATH -- everything up to and including its last separator -- or nil if PATH contains \
         no separator at all.\n\n\
         About the name, not about the disk: \"/a/b\" gives \"/a/\" whether or not `b' is a \
         directory, and \"/a/b/\" gives itself. To go *up* from a directory, take its \
         `directory-file-name' first.\n\n\
         Example:\n\
         (file-name-directory \"/a/b/c\") => \"/a/b/\"\n\
         (file-name-directory \"notes\")  => nil";

primitive!(file_name_directory, args, _env, _ctx, {
    let path = path_string(args.first())?;
    Ok(match directory_part(&path) {
        Some(directory) => ELispExp::string(directory),
        None => ELispExp::nil(),
    })
});

pub const FILE_NAME_NONDIRECTORY_DOC: &str = "(file-name-nondirectory PATH): Return the last \
         component of PATH -- everything after its last separator, or all of PATH when it has \
         none.\n\n\
         Example:\n\
         (file-name-nondirectory \"/a/b/notes.txt\") => \"notes.txt\"";

primitive!(file_name_nondirectory, args, _env, _ctx, {
    let path = path_string(args.first())?;
    Ok(ELispExp::string(last_component(&path)))
});

/// A string argument, unexpanded: the name-shaping functions above work on the
/// name they are given and must not quietly make it absolute.
fn path_string<B: BufferTrait>(
    arg: Option<&ELispExp<B>>,
) -> Result<String, EvalError<EditorState<B>>> {
    match arg {
        Some(ELispExp::String(path)) => Ok(path.to_string()),
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

/// Split a part-typed path into the directory to list and the name to match
/// inside it.
///
/// `"/etc/ho"` lists `/etc` looking for `ho`; `"/etc/"` lists `/etc` looking
/// for everything; `"ho"` lists the current directory. The split is on the last
/// `/` rather than on `Path::parent`, because a path that ends in a separator
/// means "inside this directory" and `parent` would climb out of it.
pub(crate) fn split_for_completion(prefix: &str) -> (String, &str) {
    // Any separator the platform recognises, not the character `/`: on Windows
    // a path typed as `C:\\Users\\me\\doc` has no `/` in it at all, and
    // splitting on one would offer completions from the working directory for
    // every path the user typed.
    match prefix.rfind(std::path::is_separator) {
        // Keeping the separator matters at the root: the directory part of
        // `/us` is `/`, and dropping the slash would list the working
        // directory instead.
        Some(cut) => (prefix[..=cut].to_string(), &prefix[cut + 1..]),
        None => (String::new(), prefix),
    }
}

/// File names that could complete PREFIX, as whole paths ready to be confirmed.
///
/// Whole paths rather than bare names because the minibuffer replaces its
/// entire contents with the candidate it is showing: handing back `hosts` for
/// `/etc/ho` would complete the prompt to `hosts` and open the wrong file.
///
/// Hidden files are offered only once the typed name starts with a dot, which
/// is the convention every shell uses and the reason a listing of a home
/// directory is readable at all.
pub(crate) fn file_completions(prefix: &str) -> Vec<String> {
    let (directory, name) = split_for_completion(prefix);
    let listed = directory_entries(if directory.is_empty() {
        "."
    } else {
        &directory
    });
    listed
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.starts_with(name))
        .filter(|entry| name.starts_with('.') || !entry.starts_with('.'))
        .map(|entry| format!("{directory}{entry}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Changing the filesystem
// ---------------------------------------------------------------------------
//
// These are the primitives a file manager is built out of, and they are the
// only ones in the editor that destroy something outside it. Two rules follow
// from that and are worth stating once rather than repeating in each doc
// string.
//
// A failure is reported, never signalled. Deleting six files should not stop
// at the second because one of them was denied, leaving four still there and
// no listing refreshed; the caller wants to know which failed, and to carry
// on. So each returns nil and logs, exactly as `find-file` does for a file it
// cannot read.
//
// And nothing here asks the user anything. Confirmation is a question about
// what the user wants, which depends on the editor's state -- whether a region
// is active, whether the file is open, what the last answer was -- and so
// belongs in Lisp with the rest of the interface. A primitive that prompted
// could not be called from a script, and a script that could not be stopped is
// the more dangerous of the two mistakes.

pub const FILE_EXISTS_P_DOC: &str = "(file-exists-p PATH): Return t if something exists at PATH, nil \
         otherwise. A leading \"~\" is expanded.\n\n\
         A broken symlink exists: the link is there, which is what matters to \
         `rename-file' and `delete-file'.\n\n\
         Example:\n\
         (file-exists-p \"~/.config\") => t";

primitive!(file_exists_p, args, _env, _ctx, {
    let path = path_arg(args.first())?;
    // `symlink_metadata` rather than `exists`, which follows the link and so
    // calls a dangling symlink absent -- when in fact it is there, and is
    // exactly the kind of thing a file manager is asked to delete.
    Ok(bool_exp(
        std::fs::symlink_metadata(&path).is_ok() || std::path::Path::new(&path).exists(),
    ))
});

pub const FILE_DIRECTORY_P_DOC: &str = "(file-directory-p PATH): Return t if PATH is a directory, nil \
         if it is anything else or does not exist. A leading \"~\" is expanded, and a \
         symlink to a directory counts as one.\n\n\
         Example:\n\
         (file-directory-p \"/etc\") => t";

primitive!(file_directory_p, args, _env, _ctx, {
    let path = path_arg(args.first())?;
    Ok(bool_exp(std::path::Path::new(&path).is_dir()))
});

pub const DIRECTORY_ENTRY_COUNT_DOC: &str = "(directory-entry-count PATH): Return how many entries \
         PATH contains, not counting `.' and `..' and not descending into subdirectories. \
         Returns nil (logging a diagnostic) if PATH is not a readable directory.\n\n\
         What it is for: telling the user how much a recursive delete would take with them. \
         \"Delete src/ (34 entries)?\" is a question somebody can answer; \"Delete src/?\" \
         is one they can only guess at.\n\n\
         Example:\n\
         (directory-entry-count \"/etc\") => 214";

primitive!(directory_entry_count, args, _env, ctx, {
    let path = path_arg(args.first())?;
    match std::fs::read_dir(&path) {
        Ok(entries) => Ok(ELispExp::number(entries.count() as f64)),
        Err(why) => {
            ctx.log_diagnostic(&format!("Cannot count {path}: {why}"));
            Ok(ELispExp::nil())
        }
    }
});

pub const DELETE_FILE_DOC: &str = "(delete-file PATH &optional RECURSIVE): Delete PATH. Returns t on \
         success, or nil (logging a diagnostic) if it could not be deleted.\n\n\
         A file, a symlink or an *empty* directory is removed. A directory with anything in \
         it is refused unless RECURSIVE is non-nil, in which case everything under it goes \
         too.\n\n\
         Refusing by default is the point: a file manager binds this to a single keystroke, \
         and one keystroke must not be able to lose a tree. Asking for RECURSIVE is how a \
         caller says it has confirmed with the user that this is what they meant -- which is \
         also why this does no asking of its own.\n\n\
         A leading \"~\" is expanded. A symlink to a directory is unlinked, not followed: \
         deleting a link never deletes what it points at, RECURSIVE or not.\n\n\
         Example:\n\
         (delete-file \"~/notes.txt\")\n\
         (delete-file \"~/build\" t)      ; and everything under it";

primitive!(delete_file, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let path = path_arg(args.first())?;
    let recursive = args.get(1).is_some_and(|flag| flag.is_truthy());
    let as_given = std::path::Path::new(&path);
    // The link itself, not its target: `is_dir` follows symlinks, and a link
    // to a directory removed with `remove_dir_all` would take the target's
    // contents with it. `symlink_metadata` is what distinguishes them.
    let is_symlink = std::fs::symlink_metadata(as_given)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false);
    let outcome = if as_given.is_dir() && !is_symlink {
        if recursive {
            std::fs::remove_dir_all(as_given)
        } else {
            // Not `remove_dir_all` with a pre-flight emptiness check: between
            // the check and the call the directory could have gained a file.
            // `remove_dir` refuses a non-empty directory itself, so the
            // decision is made by the one call that acts on it.
            std::fs::remove_dir(as_given)
        }
    } else {
        std::fs::remove_file(as_given)
    };
    match outcome {
        Ok(()) => Ok(ELispExp::t()),
        Err(why) => {
            ctx.log_diagnostic(&format!("Cannot delete {path}: {why}"));
            Ok(ELispExp::nil())
        }
    }
});

pub const RENAME_FILE_DOC: &str = "(rename-file FROM TO): Rename FROM to TO, which may move it into \
         another directory. Returns t on success, or nil (logging a diagnostic) if it \
         failed.\n\n\
         An existing TO is *not* overwritten -- the rename is refused instead. The \
         underlying system call would replace it silently, which for a file manager means \
         one mistyped name destroys an unrelated file with nothing said. Renaming \
         something onto itself succeeds and changes nothing.\n\n\
         A leading \"~\" is expanded in both. Both must be on the same filesystem: this is \
         a rename, not a copy, and crossing a mount point is reported as a failure rather \
         than silently costing a full copy of a large tree.\n\n\
         Example:\n\
         (rename-file \"~/notes.txt\" \"~/notes.md\")\n\
         (rename-file \"draft.txt\" \"archive/draft.txt\")";

primitive!(rename_file, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let from = path_arg(args.first())?;
    let to = path_arg(args.get(1))?;
    let from_path = std::path::Path::new(&from);
    let to_path = std::path::Path::new(&to);
    // Same file, so there is nothing to refuse and nothing to do. Checked
    // before the existence test below, which would otherwise reject `(rename-file
    // "a" "a")` as clobbering a file that is the one being renamed.
    let same = std::fs::canonicalize(from_path)
        .ok()
        .zip(std::fs::canonicalize(to_path).ok())
        .is_some_and(|(a, b)| a == b);
    if same {
        return Ok(ELispExp::t());
    }
    if std::fs::symlink_metadata(to_path).is_ok() {
        ctx.log_diagnostic(&format!("Cannot rename {from} to {to}: it already exists"));
        return Ok(ELispExp::nil());
    }
    match std::fs::rename(from_path, to_path) {
        Ok(()) => Ok(ELispExp::t()),
        Err(why) => {
            ctx.log_diagnostic(&format!("Cannot rename {from} to {to}: {why}"));
            Ok(ELispExp::nil())
        }
    }
});

/// A path argument: a string, expanded, so that every one of these takes `~`.
fn path_arg<B: BufferTrait>(
    arg: Option<&ELispExp<B>>,
) -> Result<String, EvalError<EditorState<B>>> {
    match arg {
        Some(ELispExp::String(path)) => Ok(expand_path(path)),
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

/// `t` or `nil`, since Lisp has no booleans of its own.
fn bool_exp<B: BufferTrait>(yes: bool) -> ELispExp<B> {
    if yes { ELispExp::t() } else { ELispExp::nil() }
}

// ---------------------------------------------------------------------------
// Walking a tree
// ---------------------------------------------------------------------------

/// Collect every file under `root`, depth-first, as paths relative to it.
///
/// `prune` is a set of directory *names* -- not paths -- that the walk never
/// descends into, and `limit` is how many files it will collect before giving
/// up. Returns the paths and whether the limit stopped it early.
///
/// # Why pruning is here and not in the caller
///
/// Everything else about which files are interesting is policy, and policy
/// lives in Lisp. Not descending is the exception, because it cannot be done
/// afterwards: `.git` in a working repository holds thousands of objects, and
/// a walk that collected them would spend its whole budget before reaching any
/// source file. Filtering that result gives an empty list, not a slow one.
///
/// So the *mechanism* is here because it has to be, and the *list* arrives
/// from Lisp on every call. The caller still decides what is uninteresting;
/// this only knows how to not look.
fn walk_files(
    root: &std::path::Path,
    prune: &std::collections::HashSet<String>,
    suffixes: &[String],
    limit: usize,
) -> (Vec<String>, bool) {
    let mut found = Vec::new();
    // Explicit stack rather than recursion: a deep tree -- or a symlink loop
    // that `read_dir` happens to follow -- would otherwise overflow, and an
    // editor should not be able to be crashed by a directory.
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            // Unreadable directories are skipped rather than reported: a walk
            // from the home directory crosses several, and a permission error
            // for one of them is not a failure of the search.
            continue;
        };
        // Sorted per directory rather than at the end, so the order is stable
        // across filesystems -- `read_dir` promises nothing about it.
        let mut names: Vec<_> = entries
            .flatten()
            .map(|entry| (entry.file_name().to_string_lossy().to_string(), entry))
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, entry) in names {
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            // `file_type` rather than `metadata`: it does not follow symlinks,
            // so a link to a parent directory is listed as the link it is
            // instead of sending the walk round in a circle.
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !prune.contains(&name) {
                    pending.push((entry.path(), relative));
                }
            } else {
                // Dropped here rather than by the caller. A caller filtering
                // in Lisp pays an evaluation per file per suffix, which is
                // what made a 5,000-file tree with forty `*.ext` patterns in
                // its .gitignore exhaust a whole command's fuel budget and
                // return nothing at all.
                if suffixes.iter().any(|suffix| relative.ends_with(suffix)) {
                    continue;
                }
                // Counted against LIMIT only once it is kept, so the limit
                // bounds what the caller is offered rather than what the walk
                // happened to step over.
                if found.len() >= limit {
                    return (found, true);
                }
                found.push(relative);
            }
        }
    }
    found.sort();
    (found, false)
}

pub const DIRECTORY_FILES_RECURSIVE_DOC: &str = "(directory-files-recursive &optional DIRECTORY \
         LIMIT PRUNE SUFFIXES): Every file under DIRECTORY -- the current directory if it is omitted -- \
         as a list of paths relative to it, sorted. Directories themselves are not listed, only \
         the files in them.\n\n\
         Returns (TRUNCATED PATHS): TRUNCATED is t if LIMIT stopped the walk before it \
         finished, nil if the list is everything there is. A caller that ignores it shows a \
         partial list as though it were complete, which is how a search comes to quietly not \
         find a file that is there.\n\n\
         LIMIT defaults to 10000. PRUNE is a list of directory *names* -- not paths -- that the \
         walk does not descend into; every directory so named is skipped wherever it appears. \
         Pruning is not the same as filtering the result: \".git\" alone holds thousands of \
         files, and a walk that collected them would reach LIMIT before reaching any source.\n\n\
         SUFFIXES is a list of endings -- \".o\", \".lock\" -- that a file must not have to be \
         listed. Dropped during the walk for the same reason PRUNE is: filtering afterwards in \
         Lisp costs an evaluation per file per suffix, and on a large tree with a long \
         .gitignore that alone can exhaust a command's whole fuel budget. A file dropped this \
         way is not counted against LIMIT either, so LIMIT bounds what you are offered rather \
         than what the walk stepped over.\n\n\
         Unreadable directories are skipped rather than reported -- a walk of a home directory \
         crosses several, and one refusal is not a failed search. Symlinks are listed as the \
         links they are and never followed, so a link pointing at an ancestor cannot send the \
         walk round in a circle.\n\n\
         Example:\n\
         (directory-files-recursive \".\" 5000 '(\".git\" \"target\") '(\".o\"))\n\
         => (nil (\"Cargo.toml\" \"src/main.rs\" ...))";

primitive!(directory_files_recursive, args, _env, ctx, {
    if args.len() > 4 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 4,
            got: args.len(),
        });
    }
    let directory = match args.first() {
        None => ".".to_string(),
        Some(exp) if exp.is_nil() => ".".to_string(),
        Some(ELispExp::String(path)) => path.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let limit = match args.get(1) {
        None => 10000,
        Some(exp) if exp.is_nil() => 10000,
        Some(ELispExp::Number(n)) if *n >= 0.0 => *n as usize,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "a non-negative Number".into(),
                got: other.clone(),
            });
        }
    };
    let mut prune = std::collections::HashSet::new();
    if let Some(list) = args.get(2)
        && !list.is_nil()
    {
        for name in list.iter() {
            match name {
                ELispExp::String(s) => {
                    prune.insert(s.to_string());
                }
                other => {
                    return Err(EvalError::WrongArgumentType {
                        expected: "String".into(),
                        got: other.clone(),
                    });
                }
            }
        }
    }

    let mut suffixes: Vec<String> = Vec::new();
    if let Some(list) = args.get(3)
        && !list.is_nil()
    {
        for suffix in list.iter() {
            match suffix {
                ELispExp::String(s) => suffixes.push(s.to_string()),
                other => {
                    return Err(EvalError::WrongArgumentType {
                        expected: "String".into(),
                        got: other.clone(),
                    });
                }
            }
        }
    }

    let root = expand_path(&directory);
    let root = std::path::Path::new(&root);
    if !root.is_dir() {
        ctx.log_diagnostic(&format!("Cannot walk {directory}: not a directory"));
        return Ok(ELispExp::nil());
    }
    let (paths, truncated) = walk_files(root, &prune, &suffixes, limit);
    // Charged for what it found, not for the one call it was.
    //
    // The evaluator prices a primitive at one unit per call, which is right
    // for a step-shaped interpreter and wrong for anything returning a list
    // whose length it chose: this hands back twenty thousand paths for three
    // units. The budget is meant to bound *time* -- see `expect_list` in
    // `base_env`, where the rule is written down -- and a walk that costs
    // nothing to the meter is a loop of walks that the guard never stops.
    ctx.consume_fuel(u32::try_from(paths.len()).unwrap_or(u32::MAX))?;
    Ok(ELispExp::proper_list(vec![
        bool_exp(truncated),
        ELispExp::proper_list(paths.into_iter().map(ELispExp::string).collect()),
    ]))
});

pub const READ_FILE_TO_STRING_DOC: &str = "(read-file-to-string PATH): The contents of PATH as a \
         string, or nil (logging a diagnostic) if it cannot be read.\n\n\
         No buffer is involved: nothing is displayed, no mode is chosen, nothing is added to \
         the buffer list, and there is nothing to close afterwards. That is the point -- it is \
         for the files a *module* reads rather than the ones a user edits, such as the \
         `.gitignore' that tells a recursive find what to leave out.\n\n\
         A leading \"~\" is expanded. A file that is not valid UTF-8 is refused rather than \
         mangled.\n\n\
         Example:\n\
         (read-file-to-string \"~/.gitignore\")";

primitive!(read_file_to_string, args, _env, ctx, {
    let path = path_arg(args.first())?;
    match std::fs::read_to_string(&path) {
        Ok(contents) => Ok(ELispExp::string(contents)),
        Err(why) => {
            ctx.log_diagnostic(&format!("Cannot read {path}: {why}"));
            Ok(ELispExp::nil())
        }
    }
});

// ---------------------------------------------------------------------------
// The environment
// ---------------------------------------------------------------------------

pub const GETENV_DOC: &str = "(getenv NAME): The value of the environment variable NAME, or nil \
         if it is not set.\n\n\
         nil is also the answer for a variable whose value is not valid Unicode, which cannot be \
         told apart from unset here. The alternative is a second kind of nil, and nothing in \
         this editor would do anything different with it.\n\n\
         Example:\n\
         (getenv \"HOME\") => \"/home/user\"\n\
         (getenv \"MANPATH\")";

primitive!(getenv, args, _env, _ctx, {
    let Some(ELispExp::String(name)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    Ok(match std::env::var(name.as_str()) {
        Ok(value) => ELispExp::string(value),
        Err(_) => ELispExp::nil(),
    })
});

pub const SETENV_DOC: &str = "(setenv NAME &optional VALUE): Set the environment variable NAME to \
         VALUE, or remove it when VALUE is omitted or nil. Returns VALUE.\n\n\
         The change applies to this editor and to every process it starts afterwards -- which is \
         the point: `(setenv \"MANPATH\" ...)' before running `man' is how `manpage-mode' \
         decides where pages are looked for. It does not reach the shell that started the \
         editor, because no process can change its parent's environment.\n\n\
         Cross-platform: this is `std::env`, not a shell builtin, so it means the same thing \
         everywhere.\n\n\
         Example:\n\
         (setenv \"MANPATH\" \"/usr/share/man:/usr/local/share/man\")\n\
         (setenv \"PAGER\")        ; remove it";

primitive!(setenv, args, _env, _ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let ELispExp::String(name) = &args[0] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[0].clone(),
        });
    };
    // An empty or otherwise impossible name would panic inside `set_var`, and
    // a typo in a configuration file should not take the editor down.
    if name.is_empty() || name.contains('=') || name.contains('\0') {
        return Err(EvalError::RuntimeMessage(format!(
            "{name:?} is not a usable environment variable name"
        )));
    }
    match args.get(1) {
        None => {
            // SAFETY: single-threaded with respect to the environment -- see
            // the note on `set_var` below.
            unsafe { std::env::remove_var(name.as_str()) };
            Ok(ELispExp::nil())
        }
        Some(exp) if exp.is_nil() => {
            unsafe { std::env::remove_var(name.as_str()) };
            Ok(ELispExp::nil())
        }
        Some(ELispExp::String(value)) => {
            if value.contains('\0') {
                return Err(EvalError::RuntimeMessage(
                    "an environment variable's value cannot contain a null byte".into(),
                ));
            }
            // SAFETY: `set_var` is unsafe because another thread reading the
            // environment at the same moment is undefined behaviour. This
            // editor's other threads -- the background worker -- read it only
            // when spawning a process, which happens on the command thread
            // that is running this. Nothing else touches it.
            unsafe { std::env::set_var(name.as_str(), value.as_str()) };
            Ok(ELispExp::string(value.to_string()))
        }
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.clone(),
        }),
    }
});

pub const PATH_SEPARATOR_DOC: &str = "(path-separator): The character this platform puts between \
         paths in a variable like PATH or MANPATH, as a string: \":\" on Unix, \";\" on \
         Windows.\n\n\
         Asked rather than assumed, for the reason `dired--parent' asks what a directory \
         separator is: writing \":\" is one platform's rule spelled as though it were the rule, \
         and the resulting MANPATH is one long nonsense entry on the other.\n\n\
         Example:\n\
         (setenv \"MANPATH\" (string-join manpage-path (path-separator)))";

primitive!(path_separator, _args, _env, _ctx, {
    Ok(ELispExp::string(
        if cfg!(windows) { ";" } else { ":" }.to_string(),
    ))
});

pub const DATA_DIRECTORY_DOC: &str = "(data-directory &optional KIND): The directory the editor's \
         own data was installed into, or nil if it cannot be worked out.\n\n\
         With KIND -- a string like \"man\" or \"lisp\" -- the subdirectory of it. These sit \
         beside the executable, put there at build time, because they are read at runtime and \
         the source tree they came from may not be present.\n\n\
         This is what `lisp-path' is built from, and what `rsedit-man' looks in for the \
         editor's own manual pages.\n\n\
         Example:\n\
         (data-directory \"man\") => \"/path/to/target/debug/data/man\"";

primitive!(data_directory, args, _env, ctx, {
    let kind = match args.first() {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(ELispExp::String(kind)) => Some(kind.to_string()),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let Ok(exe) = std::env::current_exe() else {
        ctx.log_diagnostic("Cannot find the rsedit executable, so nor its data directory");
        return Ok(ELispExp::nil());
    };
    let Some(parent) = exe.parent() else {
        return Ok(ELispExp::nil());
    };
    // Beside the executable normally. One level up as well, because a test
    // binary lives in `target/<profile>/deps/` while the data was put in
    // `target/<profile>/data/` -- so without this the editor's own pages are
    // unreachable from every test that looks for them, which is exactly the
    // code that needs testing.
    let candidates = [parent.join("data"), parent.join("..").join("data")];
    let base = candidates
        .iter()
        .find(|path| path.is_dir())
        // Nothing installed. The first candidate is still the right answer to
        // report: a caller wants to be told where it looked.
        .unwrap_or(&candidates[0]);
    let path = match kind {
        Some(kind) => base.join(kind),
        None => base.clone(),
    };
    Ok(ELispExp::string(path.display().to_string()))
});

pub const SAVE_BUFFER_OVERWRITE_DOC: &str = "(save-buffer--overwrite): Write the current buffer \
         over its file, asking nothing. Not called directly -- it is what `save-buffer' does once \
         a conflict has been confirmed.";

primitive!(save_buffer_overwrite, _args, _env, ctx, {
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
            ctx.log_diagnostic(&format!("Wrote {path}"));
        }
        Err(why) => ctx.log_diagnostic(&format!("Failed to save: {why}")),
    }
    Ok(ELispExp::nil())
});

// ---------------------------------------------------------------------------
// Leaving without losing anything
// ---------------------------------------------------------------------------

/// The buffers a quit has to account for: modified, visiting a file, and not
/// one of the editor's own read-only views.
///
/// # Why only file-visiting ones
///
/// Because those are the ones with somewhere to be saved to. Typing a note in
/// `*scratch*` marks it modified and there is no file it belongs in, so
/// counting it would mean the editor could never be quit without a prompt
/// about something the prompt cannot fix. A listing, a backtrace and a manual
/// page are modified in the same meaningless way and are read-only besides.
fn unsaved_file_buffers<B: BufferTrait>(ctx: &EditorState<B>) -> Vec<String> {
    ctx.buffer_names()
        .into_iter()
        .filter(|name| {
            ctx.with_buffer(name, |buf| {
                buf.is_modified && !buf.read_only && buf.file_path.is_some()
            })
            .unwrap_or(false)
        })
        .collect()
}

/// VALUE as `(quote VALUE)`, so that a form built here hands it back whole.
fn quoted<B: BufferTrait>(value: ELispExp<B>) -> ELispExp<B> {
    ELispExp::form(vec![ELispExp::symbol("quote".into()), value])
}

fn names_list<B: BufferTrait>(names: &[String]) -> ELispExp<B> {
    ELispExp::proper_list(names.iter().map(|n| ELispExp::string(n.clone())).collect())
}

fn strings_from<B: BufferTrait>(value: Option<&ELispExp<B>>) -> Vec<String> {
    value
        .map(|list| {
            list.iter()
                .filter_map(|item| match item {
                    ELispExp::String(text) => Some(text.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Write NAME's buffer to the file it is visiting. False when it has none or
/// the write failed.
fn write_buffer<B: BufferTrait>(ctx: &EditorState<B>, name: &str) -> bool {
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
            ctx.log_diagnostic(&format!("Wrote {path}"));
            true
        }
        Err(why) => {
            ctx.log_diagnostic(&format!("Failed to save {path}: {why}"));
            false
        }
    }
}

/// The call that answers one buffer's question.
fn answer_form<B: BufferTrait>(
    action: &str,
    name: &str,
    rest: &[String],
    after: &ELispExp<B>,
) -> ELispExp<B> {
    ELispExp::form(vec![
        ELispExp::symbol("save-some-buffers--answer".into()),
        ELispExp::string(action.to_string()),
        ELispExp::string(name.to_string()),
        quoted(names_list(rest)),
        quoted(after.clone()),
    ])
}

pub const SAVE_SOME_BUFFERS_DOC: &str = "(save-some-buffers &optional AFTER): Offer to save each \
         modified buffer that is visiting a file, one at a time, then call AFTER with no \
         arguments.\n\n\
         `y' saves this one, `n' leaves it, `!' saves this one and every one left, and `q' stops \
         asking. Any other key does nothing, so the question cannot be answered by accident.\n\n\
         Buffers not visiting a file are passed over: typing in `*scratch*' marks it modified and \
         there is nowhere to put it, so asking would be a question with no useful answer.\n\n\
         AFTER runs whether anything was saved or not, and whether or not the questions were \
         stopped early -- it is where the caller decides what to do about what is left, which is \
         what `quit' uses it for.\n\n\
         Example:\n\
         (save-some-buffers 'quit--confirm)";

primitive!(save_some_buffers, args, env, ctx, {
    let after = args.first().cloned().unwrap_or_else(ELispExp::nil);
    let names = unsaved_file_buffers(ctx);
    ask_about(ctx, env, &names, &after)
});

/// Ask about the first of NAMES, or run AFTER when there are none left.
fn ask_about<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: std::sync::Arc<Env<EditorState<B>>>,
    names: &[String],
    after: &ELispExp<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let Some((name, rest)) = names.split_first() else {
        if after.is_truthy() {
            call_callable(after, &[], env, ctx)?;
        }
        return Ok(ELispExp::nil());
    };
    // Re-checked rather than trusted: the list was taken before the first
    // question, and answering one runs arbitrary Lisp -- a hook, a callback --
    // which may have saved or killed this one in the meantime.
    let still_unsaved = ctx
        .with_buffer(name, |buf| buf.is_modified && buf.file_path.is_some())
        .unwrap_or(false);
    if !still_unsaved {
        return ask_about(ctx, env, rest, after);
    }

    let path = ctx
        .with_buffer(name, |buf| buf.file_path.clone())
        .flatten()
        .unwrap_or_else(|| name.clone());
    ask::ask_with_keys(
        ctx,
        format!("Save {path}? (y, n, ! = all, q = stop)"),
        vec![
            ("y", answer_form("save", name, rest, after)),
            ("n", answer_form("skip", name, rest, after)),
            ("!", answer_form("all", name, rest, after)),
            ("q", answer_form("stop", name, rest, after)),
        ],
    );
    Ok(ELispExp::t())
}

pub const SAVE_SOME_BUFFERS_ANSWER_DOC: &str = "(save-some-buffers--answer ACTION NAME REST AFTER): \
         Act on one answer and move on. Not called directly -- it is what `save-some-buffers' \
         binds its keys to.";

primitive!(save_some_buffers_answer, args, env, ctx, {
    // Before anything else, so that a callback which asks its own question
    // installs a keymap that this does not then take down again.
    ctx.clear_transient_keymap();

    let action = match args.first() {
        Some(ELispExp::String(text)) => text.to_string(),
        _ => return Ok(ELispExp::nil()),
    };
    let name = match args.get(1) {
        Some(ELispExp::String(text)) => text.to_string(),
        _ => return Ok(ELispExp::nil()),
    };
    let rest: Vec<String> = strings_from(args.get(2));
    let after = args.get(3).cloned().unwrap_or_else(ELispExp::nil);

    match action.as_str() {
        "save" => {
            write_buffer(ctx, &name);
        }
        "all" => {
            // This one and every one left, with no further questions.
            write_buffer(ctx, &name);
            for other in &rest {
                write_buffer(ctx, other);
            }
            return ask_about(ctx, env, &[], &after);
        }
        "stop" => return ask_about(ctx, env, &[], &after),
        // "skip", and anything else, leaves it alone.
        _ => {}
    }
    ask_about(ctx, env, &rest, &after)
});

pub const QUIT_DOC: &str = "(quit): Leave the editor, offering to save anything modified \
         first.\n\n\
         Each modified file buffer is offered in turn -- see `save-some-buffers' -- and if any \
         are still unsaved afterwards, leaving has to be confirmed by typing `yes'. With nothing \
         modified, it simply quits.\n\n\
         The confirmation guards the act that cannot be undone rather than each step towards it: \
         declining to save one buffer is recoverable right up until the editor closes, and that \
         is the moment worth being sure about. `quit-without-saving' skips all of it.\n\n\
         Example:\n\
         (define-key nil \"C-x C-c\" 'quit)";

primitive!(quit, _args, env, ctx, {
    let names = unsaved_file_buffers(ctx);
    let after = ELispExp::symbol("quit--confirm".into());
    ask_about(ctx, env, &names, &after)
});

pub const QUIT_CONFIRM_DOC: &str = "(quit--confirm): Quit, or ask first if anything is still \
         unsaved. Not called directly -- `quit' runs it once the offers to save are done.";

primitive!(quit_confirm, _args, env, ctx, {
    let left = unsaved_file_buffers(ctx);
    if left.is_empty() {
        ctx.quit();
        return Ok(ELispExp::nil());
    }
    // Typed in full, because this is the step that loses the work. The
    // per-buffer questions above are single keys precisely so that the budget
    // for care can be spent here.
    let prompt = match left.len() {
        1 => format!("{} is unsaved. Quit anyway?", left[0]),
        n => format!("{n} buffers are unsaved. Quit anyway?"),
    };
    eval(
        &ELispExp::form(vec![
            ELispExp::symbol("yes-or-no".into()),
            ELispExp::string(prompt),
            quoted(ELispExp::symbol("quit-without-saving".into())),
        ]),
        env,
        ctx,
    )
});

pub const QUIT_WITHOUT_SAVING_DOC: &str = "(quit-without-saving): Stop the editor's main loop at \
         once, asking nothing and saving nothing.\n\n\
         What `quit' eventually calls, and the way out for anything that has already done the \
         asking itself.";

primitive!(quit_without_saving, _args, _env, ctx, {
    ctx.quit();
    Ok(ELispExp::nil())
});
