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
