//! Asking the disk and changing it: whether a path exists, deleting and
//! renaming, walking a tree, reading a file whole. A failure is logged and
//! answered with nil, and nothing here asks the user anything.
//!
//! Design: `doc/architettura-editor.typ, <disco>`.
use super::*;
use crate::paths::expand_path_from_start;

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
         LIMIT PRUNE SUFFIXES): Return (TRUNCATED PATHS): the files under DIRECTORY, the current \
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

    let root = expand_path_from_start(&directory);
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
         file at PATH as a string, without making a buffer for it, or nil when it cannot be read \
         or is not UTF-8. A leading ~ is expanded.\n\n\
         Example:\n\
         (read-file-to-string \"~/.gitignore\")";

primitive!(read_file_to_string, args, _env, ctx, {
    let path = path_arg(ctx, args.first())?;
    match std::fs::read_to_string(&path) {
        Ok(contents) => Ok(ELispExp::string(contents)),
        Err(why) => {
            ctx.log_diagnostic(&format!("Cannot read {path}: {why}"));
            Ok(ELispExp::nil())
        }
    }
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
    into.function(
        "directory-files-recursive",
        directory_files_recursive,
        DIRECTORY_FILES_RECURSIVE_DOC,
    );
    into.function(
        "read-file-to-string",
        read_file_to_string,
        READ_FILE_TO_STRING_DOC,
    );
}
