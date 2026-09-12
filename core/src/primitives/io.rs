use super::*;

pub const FIND_FILE_DOC: &str = "(find-file PATH): Open the file at PATH into a new buffer named \
         after PATH's file name (or PATH itself if it has none), make it the \
         current buffer, and return its buffer name. Returns nil (logging a \
         diagnostic) if the file can't be read.\n\n\
         Example:\n\
         (find-file \"/home/me/notes.txt\") => \"notes.txt\"";

primitive!(find_file, args, _env, ctx, {
    if let Some(ELispExp::String(path_str)) = args.first() {
        // Expanded before anything else looks at it, so that the `~/...` a
        // completion candidate offers is a path that can actually be opened.
        let path_str = expand_path(path_str);
        let path = std::path::Path::new(&path_str);
        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let buf_name = if file_name.is_empty() {
            path_str.to_string()
        } else {
            file_name
        };
        match ctx.new_buffer(&buf_name, Some(&path_str), None) {
            Some(buf_name) => Ok(ELispExp::string(buf_name)),
            None => Ok(ELispExp::nil()),
        }
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        })
    }
});

pub const SAVE_BUFFER_DOC: &str = "(save-buffer): Write the current buffer's contents to the file it \
         was visiting. Returns nil in every case (logging a diagnostic \
         either way); if the buffer has no associated file, or the write \
         fails, nothing is written.\n\n\
         Example:\n\
         (define-key nil \"C-x C-s\" 'save-buffer)";

primitive!(save_buffer, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("Failed to acquire write lock on buffer");
    let path = if let Some(path) = &buf.file_path {
        path.to_string()
    } else {
        ctx.log_diagnostic("No file associated with this buffer");
        return Ok(ELispExp::nil());
    };
    let content = buf.text.to_string();
    match std::fs::write(&path, content) {
        Ok(_) => {
            buf.is_modified = false;
            ctx.log_diagnostic(&format!("Wrote {}", path));
            Ok(ELispExp::nil())
        }
        Err(e) => {
            ctx.log_diagnostic(&format!("Failed to save: {}", e));
            Ok(ELispExp::nil())
        }
    }
});

// ---------------------------------------------------------------------------
// Paths, listings, and what file-name completion is built out of
// ---------------------------------------------------------------------------

/// Expand `~` to the user's home directory.
///
/// Only `~` and `~/...`, deliberately: `~other-user` needs the password
/// database, which is a platform question this editor has no business
/// answering. A path that cannot be expanded is returned unchanged rather than
/// rejected -- a file really can be called `~weird`, and failing to open it
/// would be worse than trying.
pub(crate) fn expand_path(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_string();
    };
    if !(rest.is_empty() || rest.starts_with('/')) {
        return path.to_string();
    }
    match std::env::var("HOME") {
        Ok(home) => format!("{home}{rest}"),
        Err(_) => path.to_string(),
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
    let mut names = Vec::new();
    for entry in std::fs::read_dir(expand_path(directory))? {
        let entry = entry?;
        let mut name = entry.file_name().to_string_lossy().to_string();
        // `file_type` rather than `metadata`, so a symlink to a directory is
        // reported as the link it is instead of failing when it dangles.
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            name.push('/');
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

pub const LIST_DIR_DOC: &str = "(list-dir &optional DIRECTORY): Return the names of everything in \
         DIRECTORY -- the current directory if it is omitted -- as a sorted list of strings, with \
         a \"/\" after each subdirectory. `.' and `..' are not included, and a leading \"~\" is \
         expanded to the home directory.\n\n\
         Returns nil (logging a diagnostic) if the directory can't be read.\n\n\
         Example:\n\
         (list-dir \"/etc\") => (\"hosts\" \"ssh/\" ...)";

primitive!(list_dir, args, _env, ctx, {
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
    match directory_entries(&directory) {
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

pub const EXPAND_FILE_NAME_DOC: &str = "(expand-file-name PATH): Return PATH with a leading \"~\" \
         replaced by the home directory. Any other PATH comes back unchanged.\n\n\
         Example:\n\
         (expand-file-name \"~/notes.txt\") => \"/home/me/notes.txt\"";

primitive!(expand_file_name, args, _env, _ctx, {
    match args.first() {
        Some(ELispExp::String(path)) => Ok(ELispExp::string(expand_path(path))),
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
});

/// Split a part-typed path into the directory to list and the name to match
/// inside it.
///
/// `"/etc/ho"` lists `/etc` looking for `ho`; `"/etc/"` lists `/etc` looking
/// for everything; `"ho"` lists the current directory. The split is on the last
/// `/` rather than on `Path::parent`, because a path that ends in a separator
/// means "inside this directory" and `parent` would climb out of it.
pub(crate) fn split_for_completion(prefix: &str) -> (String, &str) {
    match prefix.rfind('/') {
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
