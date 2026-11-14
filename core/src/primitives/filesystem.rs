//! Asking the disk and changing it: whether a path exists, deleting and
//! renaming, walking a tree, keeping an index of one, looking upwards for a
//! name, reading a file whole. A failure is logged and answered with nil, and nothing here asks
//! the user anything.
//!
//! Design: `doc/architettura-editor.typ, <disco>`.
use super::*;
use crate::editor::file_index::{self, Progress, Rules};
use crate::editor::locate::{self, Located, Question};

/// Collects every file under ROOT, depth-first, as paths relative to it, and
/// answers whether the walk stopped early.
///
/// PRUNE is a set of directory *names* -- not paths -- that the walk never
/// descends into. A file whose relative path ends in one of SUFFIXES is left
/// out, and not counted. LIMIT is how many files the walk collects before it
/// gives up.
///
/// KEEP_GOING is asked before each directory is read, since reading one is
/// what can be slow: when it says no, the walk ends there as if the limit had
/// stopped it. What a search on a thread of its own passes, so that being
/// asked to stop is heard during the walk too.
pub(crate) fn walk_files(
    root: &std::path::Path,
    prune: &std::collections::HashSet<String>,
    suffixes: &[String],
    limit: usize,
    keep_going: &dyn Fn() -> bool,
) -> (Vec<String>, bool) {
    let mut found = Vec::new();
    // Explicit stack rather than recursion: a deep tree -- or a symlink loop
    // that `read_dir` happens to follow -- would otherwise overflow, and an
    // editor should not be able to be crashed by a directory.
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        if !keep_going() {
            return (found, true);
        }
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
                // Dropped here rather than by the caller: filtering in Lisp
                // costs an evaluation per file per suffix, and a 5,000-file
                // tree with forty `*.ext` patterns in its .gitignore would
                // spend a whole command's fuel on it and return nothing.
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

pub const FILE_EXISTS_P_DOC: &str = "(file-exists-p PATH): Return t when something is at PATH, a \
         broken symbolic link included, nil otherwise. A leading ~ is expanded.";

primitive!(file_exists_p, args, _env, ctx, {
    let path = path_arg(ctx, args.first())?;
    // `symlink_metadata` rather than `exists`, which follows the link and so
    // calls a dangling symlink absent -- when in fact it is there, and is
    // exactly the kind of thing a file manager is asked to delete.
    Ok(bool_exp(
        std::fs::symlink_metadata(&path).is_ok() || std::path::Path::new(&path).exists(),
    ))
});

pub const FILE_DIRECTORY_P_DOC: &str = "(file-directory-p PATH): Return t when PATH is a \
         directory, or a symbolic link to one, nil otherwise. A leading ~ is expanded.";

primitive!(file_directory_p, args, _env, ctx, {
    let path = path_arg(ctx, args.first())?;
    Ok(bool_exp(std::path::Path::new(&path).is_dir()))
});

pub const DIRECTORY_ENTRY_COUNT_DOC: &str = "(directory-entry-count PATH): Return how many \
         entries the directory PATH holds, not counting . and .. nor what is in its \
         subdirectories, or nil when it is not a readable directory.\n\n\
         Example:\n\
         (directory-entry-count \"/etc\")";

primitive!(directory_entry_count, args, _env, ctx, {
    let path = path_arg(ctx, args.first())?;
    match std::fs::read_dir(&path) {
        Ok(entries) => Ok(ELispExp::number(entries.count() as f64)),
        Err(why) => {
            ctx.log_diagnostic(&format!("Cannot count {path}: {why}"));
            Ok(ELispExp::nil())
        }
    }
});

pub const DELETE_FILE_DOC: &str = "(delete-file PATH &optional RECURSIVE): Delete the file, \
         symbolic link or empty directory at PATH, and return t, or nil when it could not be \
         deleted. A directory with something in it is deleted, with everything under it, only \
         when RECURSIVE is non-nil. A symbolic link is removed, never what it points to. A \
         leading ~ is expanded.\n\n\
         Example:\n\
         (delete-file \"~/build\" t)";

primitive!(delete_file, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let path = path_arg(ctx, args.first())?;
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

pub const RENAME_FILE_DOC: &str = "(rename-file FROM TO): Rename FROM to TO, which may be in \
         another directory of the same filesystem, and return t, or nil when it failed. An \
         existing TO is not replaced: the rename is refused. A leading ~ is expanded in both.\n\n\
         Example:\n\
         (rename-file \"~/notes.txt\" \"~/notes.md\")";

primitive!(rename_file, args, _env, ctx, {
    exact_arity(args, 2)?;
    let from = path_arg(ctx, args.first())?;
    let to = path_arg(ctx, args.get(1))?;
    let from_path = std::path::Path::new(&from);
    let to_path = std::path::Path::new(&to);
    // Same file, so there is nothing to refuse and nothing to do. Checked
    // before the existence test below, which would otherwise reject
    // `(rename-file "a" "a")` as clobbering a file that is the one being
    // renamed.
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

pub const COPY_FILE_DOC: &str = "(copy-file FROM TO &optional RECURSIVE): Copy the file FROM \
         to TO, and return t, or nil when it failed. A directory is copied, with everything \
         under it, only when RECURSIVE is non-nil; a symbolic link inside one is copied as a \
         link where the platform has them, never followed. An existing TO is not replaced: the \
         copy is refused. A leading ~ is expanded in both.\n\n\
         Example:\n\
         (copy-file \"~/notes.txt\" \"~/notes.bak\")";

primitive!(copy_file, args, _env, ctx, {
    if args.len() < 2 || args.len() > 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let from = path_arg(ctx, args.first())?;
    let to = path_arg(ctx, args.get(1))?;
    let recursive = args.get(2).is_some_and(|flag| flag.is_truthy());
    let from_path = std::path::Path::new(&from);
    let to_path = std::path::Path::new(&to);
    // Refused before anything is written, as a rename is: a copy that
    // replaced what was there could not be taken back.
    if std::fs::symlink_metadata(to_path).is_ok() {
        ctx.log_diagnostic(&format!("Cannot copy {from} to {to}: it already exists"));
        return Ok(ELispExp::nil());
    }
    let is_dir = std::fs::symlink_metadata(from_path)
        .map(|meta| meta.is_dir())
        .unwrap_or(false);
    if is_dir && !recursive {
        ctx.log_diagnostic(&format!(
            "Cannot copy {from}: it is a directory, and the copy was not recursive"
        ));
        return Ok(ELispExp::nil());
    }
    // A directory into itself would never end.
    if is_dir
        && let (Ok(source), Some(parent)) = (
            std::fs::canonicalize(from_path),
            to_path.parent().and_then(|p| std::fs::canonicalize(p).ok()),
        )
        && parent.starts_with(&source)
    {
        ctx.log_diagnostic(&format!("Cannot copy {from} into itself"));
        return Ok(ELispExp::nil());
    }
    match copy_tree(from_path, to_path) {
        Ok(()) => Ok(ELispExp::t()),
        Err(why) => {
            ctx.log_diagnostic(&format!("Cannot copy {from} to {to}: {why}"));
            Ok(ELispExp::nil())
        }
    }
});

/// Copies FROM to TO: a file's contents, a directory with everything under
/// it, a symbolic link as a link where the platform has them.
fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        // The link, not what it points to: following one inside a tree could
        // copy the disk, or loop.
        #[cfg(unix)]
        {
            let target = std::fs::read_link(from)?;
            return std::os::unix::fs::symlink(target, to);
        }
        #[cfg(not(unix))]
        {
            std::fs::copy(from, to)?;
            return Ok(());
        }
    }
    if meta.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Reads a path argument: a string, expanded, so that every one of these
/// takes `~`.
fn path_arg<B: BufferTrait>(
    ctx: &EditorState<B>,
    arg: Option<&ELispExp<B>>,
) -> Result<String, EvalError<EditorState<B>>> {
    match arg {
        Some(ELispExp::String(path)) => Ok(ctx.expand_path(path)),
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

/// Returns `t` or `nil`, since Lisp has no booleans of its own.
fn bool_exp<B: BufferTrait>(yes: bool) -> ELispExp<B> {
    if yes { ELispExp::t() } else { ELispExp::nil() }
}

pub const DIRECTORY_FILES_RECURSIVE_DOC: &str = "(directory-files-recursive &optional DIRECTORY \
         LIMIT PRUNE SUFFIXES): Return (TRUNCATED PATHS): the files under DIRECTORY, the working \
         directory by default, as a sorted list of paths relative to it, and whether the walk \
         stopped at LIMIT files, 10000 by default. The directory names in PRUNE are not entered \
         at any depth, and files ending in one of SUFFIXES are left out and not counted. \
         Directories that cannot be read are skipped, and symbolic links are listed, not \
         followed.\n\n\
         Example:\n\
         (directory-files-recursive \".\" 5000 '(\".git\" \"target\") '(\".o\"))";

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

    let root = ctx.expand_path(&directory);
    let root = std::path::Path::new(&root);
    if !root.is_dir() {
        ctx.log_diagnostic(&format!("Cannot walk {directory}: not a directory"));
        return Ok(ELispExp::nil());
    }
    let (paths, truncated) = walk_files(root, &prune, &suffixes, limit, &|| true);
    // Charged for what it found, not for the one call it was: fuel is there
    // to bound time, and a walk the meter does not see makes a loop of walks
    // that the guard never stops. The rule is written at `expect_list`, in
    // `risp::base`.
    ctx.consume_fuel(u32::try_from(paths.len()).unwrap_or(u32::MAX))?;
    Ok(ELispExp::proper_list(vec![
        bool_exp(truncated),
        ELispExp::proper_list(paths.into_iter().map(ELispExp::string).collect()),
    ]))
});

pub const READ_FILE_TO_STRING_DOC: &str = "(read-file-to-string PATH): Return the contents of the \
         file at PATH as a string, without making a buffer for it, or nil when it cannot be read. \
         A file that is not UTF-8 is read as windows-1252. A leading ~ is expanded.\n\n\
         Example:\n\
         (read-file-to-string \"~/.gitignore\")";

primitive!(read_file_to_string, args, _env, ctx, {
    let path = path_arg(ctx, args.first())?;
    match crate::buffer::coding::read_text(&path) {
        Ok((contents, _)) => Ok(ELispExp::string(contents)),
        Err(why) => {
            ctx.log_diagnostic(&format!("Cannot read {path}: {why}"));
            Ok(ELispExp::nil())
        }
    }
});

pub const LOCATE_DOMINATING_FILE_DOC: &str = "(locate-dominating-file DIRECTORY NAMES &optional \
         WHICH): Return the nearest directory, from DIRECTORY upwards and DIRECTORY included, \
         that holds one of NAMES, or nil when none does up to the root. NAMES is a file name or \
         a list of them, and DIRECTORY the working directory when nil. WHICH 'topmost asks for \
         the highest such directory instead. The answer ends in a separator and is remembered \
         until `locate-dominating-file-forget'. A disk that does not answer within \
         `locate-dominating-file-timeout' seconds gives 'timeout.\n\n\
         Example:\n\
         (locate-dominating-file \"~/src/app/lib\" '(\".git\" \"Makefile\"))";

primitive!(locate_dominating_file, args, env, ctx, {
    if args.is_empty() || args.len() > 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let start = match args.first() {
        Some(exp) if exp.is_nil() => ctx.expand_path("."),
        other => path_arg(ctx, other)?,
    };
    let mut names = Vec::new();
    match args.get(1) {
        Some(name @ ELispExp::String(_)) => names.push(marker_name(name)?),
        Some(list) if list.is_nil() || matches!(list, ELispExp::Cons(..)) => {
            for name in list.iter() {
                names.push(marker_name(&name)?);
            }
        }
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "a String or a list of them".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    }
    let topmost = match args.get(2) {
        None => false,
        Some(exp) if exp.is_nil() => false,
        Some(ELispExp::Symbol(which)) if &**which == "nearest" => false,
        Some(ELispExp::Symbol(which)) if &**which == "topmost" => true,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "nearest or topmost".into(),
                got: other.clone(),
            });
        }
    };
    if names.is_empty() {
        return Ok(ELispExp::nil());
    }
    let question = Question {
        start,
        names,
        topmost,
    };
    // `symlink_metadata`, so that a link that points nowhere counts as there.
    let holds = |path: &std::path::Path| std::fs::symlink_metadata(path).is_ok();
    Ok(match ctx.locate(question, locate::timeout(&env), holds) {
        Located::Known(Some(directory)) => ELispExp::string(directory),
        Located::Known(None) => ELispExp::nil(),
        Located::Unanswered => ELispExp::symbol("timeout".into()),
    })
});

/// Reads one of the names `locate-dominating-file' looks for: a string that
/// is not empty and not absolute, since it is joined to each directory.
fn marker_name<B: BufferTrait>(name: &ELispExp<B>) -> Result<String, EvalError<EditorState<B>>> {
    match name {
        ELispExp::String(name) if !name.is_empty() && !std::path::Path::new(&**name).has_root() => {
            Ok(name.to_string())
        }
        other => Err(EvalError::WrongArgumentType {
            expected: "a relative file name".into(),
            got: other.clone(),
        }),
    }
}

pub const LOCATE_DOMINATING_FILE_FORGET_DOC: &str = "(locate-dominating-file-forget &optional \
         DIRECTORY): Forget what `locate-dominating-file' remembers for the searches that \
         started in DIRECTORY or under it, or for all of them when DIRECTORY is nil, and \
         return how many answers were forgotten. To call after creating or deleting one of the \
         names searched for.\n\n\
         Example:\n\
         (locate-dominating-file-forget \"~/src/app\")";

primitive!(locate_dominating_file_forget, args, _env, ctx, {
    if args.len() > 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let directory = match args.first() {
        None => None,
        Some(exp) if exp.is_nil() => None,
        some => Some(path_arg(ctx, some)?),
    };
    let forgotten = ctx.forget_located(directory.as_deref());
    Ok(ELispExp::number(forgotten as f64))
});

pub const FILE_INDEX_DOC: &str = "(file-index DIRECTORY &optional EXCLUDE IGNORE-FILES LIMIT): \
         Return (STATE PATHS), the files found so far under DIRECTORY, the working directory \
         when nil, as paths relative to it, without waiting for the disk. The first call starts \
         a walk in the background: STATE is walking until it ends, then complete, or truncated \
         at LIMIT files, 100000 by default. EXCLUDE is a list of lines in the syntax of a \
         .gitignore; IGNORE-FILES names the files whose lines count in their directory and \
         below. The index is kept, and walked again after `file-index-max-age' seconds.\n\n\
         Example:\n\
         (file-index \"~/src\" '(\".git/\" \"*.o\") '(\".gitignore\"))";

primitive!(file_index, args, env, ctx, {
    if args.is_empty() || args.len() > 4 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let root = match args.first() {
        Some(exp) if exp.is_nil() => ctx.expand_path("."),
        other => path_arg(ctx, other)?,
    };
    let limit = match args.get(3) {
        None => file_index::DEFAULT_LIMIT,
        Some(exp) if exp.is_nil() => file_index::DEFAULT_LIMIT,
        Some(ELispExp::Number(n)) if *n >= 0.0 => *n as usize,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "a non-negative Number".into(),
                got: other.clone(),
            });
        }
    };
    let rules = Rules {
        exclude: strings(args.get(1))?,
        ignore_files: strings(args.get(2))?,
        limit,
    };
    let indexed = ctx.file_index(&root, &rules, file_index::max_age(&env));
    // Charged for what it hands over, as `directory-files-recursive' is.
    ctx.consume_fuel(u32::try_from(indexed.paths.len()).unwrap_or(u32::MAX))?;
    let state = match indexed.progress {
        Progress::Walking => "walking",
        Progress::Complete => "complete",
        Progress::Truncated => "truncated",
    };
    Ok(ELispExp::proper_list(vec![
        ELispExp::symbol(state.into()),
        ELispExp::proper_list(
            indexed
                .paths
                .iter()
                .map(|path| ELispExp::string(path.to_string()))
                .collect(),
        ),
    ]))
});

/// Reads a list of strings, nil or absent being the empty one.
fn strings<B: BufferTrait>(
    list: Option<&ELispExp<B>>,
) -> Result<Vec<String>, EvalError<EditorState<B>>> {
    let mut out = Vec::new();
    if let Some(list) = list
        && !list.is_nil()
    {
        if !matches!(list, ELispExp::Cons(..)) {
            return Err(EvalError::WrongArgumentType {
                expected: "a list of Strings".into(),
                got: list.clone(),
            });
        }
        for item in list.iter() {
            match item {
                ELispExp::String(s) => out.push(s.to_string()),
                other => {
                    return Err(EvalError::WrongArgumentType {
                        expected: "String".into(),
                        got: other,
                    });
                }
            }
        }
    }
    Ok(out)
}

pub const FILE_INDEX_FORGET_DOC: &str = "(file-index-forget &optional DIRECTORY): Drop the \
         indexes `file-index' keeps for DIRECTORY and the directories under it, or all of them \
         when DIRECTORY is nil, stopping their walks, and return how many were dropped. The \
         next `file-index' walks again from the start.\n\n\
         Example:\n\
         (file-index-forget \"~/src\")";

primitive!(file_index_forget, args, _env, ctx, {
    if args.len() > 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let directory = match args.first() {
        None => None,
        Some(exp) if exp.is_nil() => None,
        some => Some(path_arg(ctx, some)?),
    };
    Ok(ELispExp::number(
        ctx.forget_file_index(directory.as_deref()) as f64,
    ))
});

/// Registers the primitives that ask the disk and change it.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Functions, not commands: a person deletes files from a listing, which
    // shows what is there and asks about it.
    into.function("file-exists-p", file_exists_p, FILE_EXISTS_P_DOC);
    into.function("file-directory-p", file_directory_p, FILE_DIRECTORY_P_DOC);
    into.function(
        "directory-entry-count",
        directory_entry_count,
        DIRECTORY_ENTRY_COUNT_DOC,
    );
    into.function("delete-file", delete_file, DELETE_FILE_DOC);
    into.function("rename-file", rename_file, RENAME_FILE_DOC);
    into.function("copy-file", copy_file, COPY_FILE_DOC);
    into.function(
        "directory-files-recursive",
        directory_files_recursive,
        DIRECTORY_FILES_RECURSIVE_DOC,
    );
    into.function("file-index", file_index, FILE_INDEX_DOC);
    into.function(
        "file-index-forget",
        file_index_forget,
        FILE_INDEX_FORGET_DOC,
    );
    into.function(
        "locate-dominating-file",
        locate_dominating_file,
        LOCATE_DOMINATING_FILE_DOC,
    );
    into.function(
        "locate-dominating-file-forget",
        locate_dominating_file_forget,
        LOCATE_DOMINATING_FILE_FORGET_DOC,
    );
    into.function(
        "read-file-to-string",
        read_file_to_string,
        READ_FILE_TO_STRING_DOC,
    );
}
