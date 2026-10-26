//! File names: a path made absolute and taken apart, the entries of a
//! directory, and the candidates file-name completion offers. Names are
//! shaped without asking the disk, which is read only to list a directory.
//!
//! Design: `doc/architettura-editor.typ, <nomi-file>`.
use super::*;

/// Returns PATH as an absolute, normalised path: `~` expanded, resolved
/// against the working directory if it is relative, and `.` and `..`
/// collapsed by name, without asking the filesystem.
pub(crate) fn expand_path(path: &str) -> String {
    expand_path_in(path, &working_directory())
}

/// Returns PATH made absolute against BASE rather than against the working
/// directory.
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

/// Returns the directory the editor is running in, or the root if even that
/// cannot be had.
///
/// A failure here means the working directory was deleted out from under the
/// process. The root is a poor answer but a usable one, and better than
/// refusing to expand anything for the rest of the session.
fn working_directory() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from(std::path::MAIN_SEPARATOR_STR))
}

/// Expands a leading `~`, leaving anything else alone.
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

/// Collapses `.` and `..` without touching the filesystem.
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

/// Returns PATH with a trailing separator, so that a name can be joined onto
/// it.
pub(crate) fn as_directory(path: &str) -> String {
    if path.ends_with(std::path::is_separator) {
        return path.to_string();
    }
    format!("{path}{}", std::path::MAIN_SEPARATOR)
}

/// Returns PATH without its trailing separator: the directory named as a
/// *file*, which is the form whose parent can be asked for.
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

/// Returns the directory part of PATH -- everything up to and including its
/// last separator -- or nothing when PATH names no directory at all.
///
/// Lexical, like Emacs's `file-name-directory`: `"/a/b"` gives `"/a/"` and
/// `"/a/b/"` gives itself, because the question is about the *name*, not about
/// what is on disk.
pub(crate) fn directory_part(path: &str) -> Option<String> {
    let cut = path.rfind(std::path::is_separator)?;
    Some(path[..=cut].to_string())
}

/// Returns the last component of PATH: everything after its last separator.
pub(crate) fn last_component(path: &str) -> String {
    match path.rfind(std::path::is_separator) {
        Some(cut) => path[cut + 1..].to_string(),
        None => path.to_string(),
    }
}

/// Returns everything in DIRECTORY, sorted, with the platform's separator
/// after each subdirectory.
///
/// The trailing separator is the whole reason this is useful for completion:
/// it is what tells the caller -- and the user reading a cycled candidate --
/// that pressing Tab again will descend rather than confirm, and it is what
/// makes a completed directory name a legal prefix for the next component.
///
/// `.` and `..` are not listed. They are always there, they are never what
/// somebody is looking for, and they would sit at the top of every listing.
pub(crate) fn directory_entries(directory: &str) -> Result<Vec<String>, std::io::Error> {
    directory_entries_ordered(directory, false)
}

/// Returns the same, with directories gathered before files when DIRS_FIRST.
///
/// Ordered here rather than by the caller because the answer is already in
/// hand: `file_type` has been asked for anyway, to decide the trailing
/// separator. A caller sorting afterwards would work out what each entry is
/// all over again, at a `stat` and an evaluation apiece from Lisp, which on a
/// large tree is more than `find-file-recursive` can afford.
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

/// Splits a part-typed path into the directory to list and the name to match
/// inside it.
///
/// `"/etc/ho"` lists `/etc` looking for `ho`; `"/etc/"` lists `/etc` looking
/// for everything; `"ho"` lists the current directory. The split is on the last
/// separator rather than on `Path::parent`, because a path that ends in a
/// separator means "inside this directory" and `parent` would climb out of it.
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

/// Returns the file names that could complete PREFIX, as whole paths ready to
/// be confirmed.
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

pub const LIST_DIR_DOC: &str = "(list-dir &optional DIRECTORY ORDER): Return the names in \
         DIRECTORY, the current directory by default, as a sorted list of strings with a / after \
         each subdirectory; . and .. are left out. ORDER is name, the default, for one \
         alphabetical run, or type for the subdirectories first. A leading ~ is expanded, and a \
         directory that cannot be read gives nil.\n\n\
         Example:\n\
         (list-dir \"/etc\" 'type)";

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

pub const MATCH_LIST_DOC: &str = "(match-list LIST PATTERN): Return the strings in LIST that \
         contain PATTERN, in their order; anything that is not a string is left out.\n\n\
         Example:\n\
         (match-list (list-dir \"/etc\") \".conf\")";

primitive!(match_list, args, _env, _ctx, {
    exact_arity(args, 2)?;
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

pub const EXPAND_FILE_NAME_DOC: &str = "(expand-file-name PATH &optional DIRECTORY): Return PATH \
         made absolute: a leading ~ becomes the home directory, a relative PATH is taken from \
         DIRECTORY or from the directory the editor was started in, and . and .. are resolved by \
         name, without looking at the disk.\n\n\
         Example:\n\
         (expand-file-name \"~/notes.txt\")    => \"/home/me/notes.txt\"\n\
         (expand-file-name \"..\" \"/a/b/c/\")   => \"/a/b\"";

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
         separator at its end, adding one only when it has none, ready for a name to be joined on \
         with `concat'.\n\n\
         Example:\n\
         (concat (file-name-as-directory \"/etc\") \"hosts\") => \"/etc/hosts\"";

primitive!(file_name_as_directory, args, _env, _ctx, {
    let path = path_string(args.first())?;
    Ok(ELispExp::string(as_directory(&path)))
});

pub const DIRECTORY_FILE_NAME_DOC: &str = "(directory-file-name PATH): Return PATH without the \
         separator at its end; a root comes back as it is.\n\n\
         Example:\n\
         (file-name-directory (directory-file-name \"/a/b/\")) => \"/a/\"";

primitive!(directory_file_name, args, _env, _ctx, {
    let path = path_string(args.first())?;
    Ok(ELispExp::string(without_trailing_separator(&path)))
});

pub const FILE_NAME_DIRECTORY_DOC: &str = "(file-name-directory PATH): Return the directory part \
         of PATH, up to and including its last separator, or nil when it has none. It reads the \
         name, not the disk.\n\n\
         Example:\n\
         (file-name-directory \"/a/b/c\") => \"/a/b/\"";

primitive!(file_name_directory, args, _env, _ctx, {
    let path = path_string(args.first())?;
    Ok(match directory_part(&path) {
        Some(directory) => ELispExp::string(directory),
        None => ELispExp::nil(),
    })
});

pub const FILE_NAME_NONDIRECTORY_DOC: &str = "(file-name-nondirectory PATH): Return the part of \
         PATH after its last separator, or all of PATH when it has none.\n\n\
         Example:\n\
         (file-name-nondirectory \"/a/b/notes.txt\") => \"notes.txt\"";

primitive!(file_name_nondirectory, args, _env, _ctx, {
    let path = path_string(args.first())?;
    Ok(ELispExp::string(last_component(&path)))
});

/// Reads a string argument, unexpanded: the name-shaping functions above work
/// on the name they are given and must not quietly make it absolute.
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

/// Registers the primitives that shape a file name and list a directory.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("list-dir", list_dir, LIST_DIR_DOC);
    into.function("match-list", match_list, MATCH_LIST_DOC);
    into.function("expand-file-name", expand_file_name, EXPAND_FILE_NAME_DOC);
    // In Rust, not in a module: what a separator and a root are is the
    // platform's to say.
    into.function(
        "file-name-as-directory",
        file_name_as_directory,
        FILE_NAME_AS_DIRECTORY_DOC,
    );
    into.function(
        "directory-file-name",
        directory_file_name,
        DIRECTORY_FILE_NAME_DOC,
    );
    into.function(
        "file-name-directory",
        file_name_directory,
        FILE_NAME_DIRECTORY_DOC,
    );
    into.function(
        "file-name-nondirectory",
        file_name_nondirectory,
        FILE_NAME_NONDIRECTORY_DOC,
    );
}
