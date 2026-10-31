//! File names from Lisp: `list-dir`, `expand-file-name` and the functions
//! that take a name apart, over the helpers in [`crate::paths`]; and the
//! working directory, `working-directory`, `cd` and `pwd`, over
//! [`crate::editor::EditorState::working_directory`].
//!
//! Design: `doc/architettura-editor.typ, <nomi-file>`.
use super::*;
use crate::paths::{
    as_directory, directory_entries_ordered, directory_part, expand_path_in, last_component,
    without_trailing_separator,
};

pub const LIST_DIR_DOC: &str = "(list-dir &optional DIRECTORY ORDER): Return the names in \
         DIRECTORY, the working directory by default, as a sorted list of strings with a / after \
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
    match directory_entries_ordered(&ctx.expand_path(&directory), dirs_first) {
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
         DIRECTORY or from the working directory, and . and .. are resolved by \
         name, without looking at the disk.\n\n\
         Example:\n\
         (expand-file-name \"~/notes.txt\")    => \"/home/me/notes.txt\"\n\
         (expand-file-name \"..\" \"/a/b/c/\")   => \"/a/b\"";

primitive!(expand_file_name, args, _env, ctx, {
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
        None => ctx.expand_path(&path),
        Some(exp) if exp.is_nil() => ctx.expand_path(&path),
        Some(ELispExp::String(base)) => {
            // The base is expanded too, so that a relative or `~`-prefixed
            // DIRECTORY does not quietly produce a relative answer -- the
            // whole point of this function is that what comes out is absolute.
            expand_path_in(&path, std::path::Path::new(&ctx.expand_path(base)))
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
pub const WORKING_DIRECTORY_DOC: &str = "(working-directory): Return the directory a relative \
         file name is read in, from the current buffer: the editor's working directory, which \
         `cd' changes, unless the buffer has a directory of its own.\n\n\
         Example:\n\
         (expand-file-name \"notes.txt\" (working-directory))";

primitive!(working_directory, args, _env, ctx, {
    if !args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 0,
            got: args.len(),
        });
    }
    Ok(ELispExp::string(ctx.default_directory()))
});

pub const CD_DOC: &str = "(cd DIRECTORY): Make DIRECTORY, read in the working directory, the \
         editor's working directory, and return it; nil, and a message, when it is not a \
         directory. Relative file names are read in it from then on. A command already running \
         stays where it started.\n\n\
         Example:\n\
         (cd \"~/src/rsedit\")";

primitive!(cd, args, _env, ctx, {
    let Some(ELispExp::String(path)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    match ctx.set_working_directory(path) {
        Ok(directory) => {
            ctx.set_echo_message(&format!("Directory {directory}"));
            Ok(ELispExp::string(directory))
        }
        Err(why) => {
            ctx.set_echo_message(&why);
            Ok(ELispExp::nil())
        }
    }
});

pub const PWD_DOC: &str = "(pwd): Say in the echo area which directory relative file names are \
         read in, the value of `working-directory', and return it.";

primitive!(pwd, _args, _env, ctx, {
    let directory = ctx.default_directory();
    ctx.set_echo_message(&format!("Directory {directory}"));
    Ok(ELispExp::string(directory))
});

pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function(
        "working-directory",
        working_directory,
        WORKING_DIRECTORY_DOC,
    );
    into.command("cd", cd, &["fChange directory: "], CD_DOC);
    into.command("pwd", pwd, &[], PWD_DOC);
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
