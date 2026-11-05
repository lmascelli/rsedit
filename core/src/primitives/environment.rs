//! What the editor knows of where it runs: the environment variables, the
//! separator between the paths in one, and the directory its own data is
//! installed in.
//!
//! Design: `doc/architettura-editor.typ, <ambiente>`.
use super::*;

pub const GETENV_DOC: &str = "(getenv NAME): Return the value of the environment variable NAME, \
         as `setenv' left it, or nil when it is not set or is not valid Unicode.\n\n\
         Example:\n\
         (getenv \"HOME\")";

primitive!(getenv, args, _env, ctx, {
    let Some(ELispExp::String(name)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    Ok(match ctx.getenv(name) {
        Some(value) => ELispExp::string(value),
        None => ELispExp::nil(),
    })
});

pub const SETENV_DOC: &str = "(setenv NAME &optional VALUE): Set the environment variable NAME to \
         VALUE, or remove it when VALUE is nil or missing, and return VALUE. The change reaches \
         `getenv' and the programs the editor starts from then on, not the shell that started \
         the editor, and not the editor's own process.\n\n\
         Example:\n\
         (setenv \"PAGER\" \"cat\")\n\
         (setenv \"PAGER\")   ; remove it";

primitive!(setenv, args, _env, ctx, {
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
    // An empty or otherwise impossible name would make every program started
    // afterwards fail to start, and a typo in a configuration file should be
    // said where it is.
    if name.is_empty() || name.contains('=') || name.contains('\0') {
        return Err(EvalError::RuntimeMessage(format!(
            "{name:?} is not a usable environment variable name"
        )));
    }
    // Into the editor's table, not the process's environment: see
    // `EditorState::environment`. No `unsafe`, then: nothing another thread
    // reads changes under it.
    match args.get(1) {
        None => {
            ctx.setenv(name, None);
            Ok(ELispExp::nil())
        }
        Some(exp) if exp.is_nil() => {
            ctx.setenv(name, None);
            Ok(ELispExp::nil())
        }
        Some(ELispExp::String(value)) => {
            if value.contains('\0') {
                return Err(EvalError::RuntimeMessage(
                    "an environment variable's value cannot contain a null byte".into(),
                ));
            }
            ctx.setenv(name, Some(value.to_string()));
            Ok(ELispExp::string(value.to_string()))
        }
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.clone(),
        }),
    }
});

pub const PATH_SEPARATOR_DOC: &str = "(path-separator): Return what separates the paths in a \
         variable such as PATH, as a string: \":\" on Unix, \";\" on Windows.\n\n\
         Example:\n\
         (split-string (getenv \"PATH\") (path-separator))";

primitive!(path_separator, _args, _env, _ctx, {
    Ok(ELispExp::string(
        if cfg!(windows) { ";" } else { ":" }.to_string(),
    ))
});

pub const DATA_DIRECTORY_DOC: &str = "(data-directory &optional KIND): Return the directory the \
         editor's own data is installed in, or nil when it cannot be found. With KIND, a string \
         such as \"man\" or \"lisp\", return that subdirectory of it.\n\n\
         Example:\n\
         (data-directory \"man\")";

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
    let Some(base) = crate::paths::installed_data_dir() else {
        ctx.log_diagnostic("Cannot find the rsedit executable, so nor its data directory");
        return Ok(ELispExp::nil());
    };
    let path = match kind {
        Some(kind) => base.join(kind),
        None => base,
    };
    Ok(ELispExp::string(path.display().to_string()))
});

/// Registers the primitives that read and set the environment.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("data-directory", data_directory, DATA_DIRECTORY_DOC);
    into.function("getenv", getenv, GETENV_DOC);
    into.function("setenv", setenv, SETENV_DOC);
    into.function("path-separator", path_separator, PATH_SEPARATOR_DOC);
}
