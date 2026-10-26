//! What the editor knows of where it runs: the environment variables, the
//! separator between the paths in one, and the directory its own data is
//! installed in.
//!
//! Design: `doc/architettura-editor.typ, <ambiente>`.
use super::*;

pub const GETENV_DOC: &str = "(getenv NAME): Return the value of the environment variable NAME, \
         or nil when it is not set or is not valid Unicode.\n\n\
         Example:\n\
         (getenv \"HOME\")";

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
         VALUE, or remove it when VALUE is nil or missing, and return VALUE. The change reaches \
         the programs the editor starts from then on, not the shell that started the editor.\n\n\
         Example:\n\
         (setenv \"PAGER\" \"cat\")\n\
         (setenv \"PAGER\")   ; remove it";

// TODO(BKG-3): code that reads the environment without going through `std`
// is not kept out while it changes.
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
            // SAFETY: as for `set_var` below.
            unsafe { std::env::remove_var(name.as_str()) };
            Ok(ELispExp::nil())
        }
        Some(exp) if exp.is_nil() => {
            // SAFETY: as for `set_var` below.
            unsafe { std::env::remove_var(name.as_str()) };
            Ok(ELispExp::nil())
        }
        Some(ELispExp::String(value)) => {
            if value.contains('\0') {
                return Err(EvalError::RuntimeMessage(
                    "an environment variable's value cannot contain a null byte".into(),
                ));
            }
            // SAFETY: other threads read the environment while this runs --
            // a Lisp job on the executor calling `getenv` or starting a
            // process, the auto-saver expanding a `~`, `stop shell` starting
            // `/bin/sh` -- and all of them go through `std::env` and
            // `std::process`, which take the lock `set_var` takes: none of
            // them sees a change half made. The lock does not cover code that
            // reads the environment by itself, through the C library, which
            // is why `set_var` is unsafe: the TUI runs none on another thread,
            // and the graphical front-end's system libraries may.
            unsafe { std::env::set_var(name.as_str(), value.as_str()) };
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

/// Registers the primitives that read and set the environment.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("data-directory", data_directory, DATA_DIRECTORY_DOC);
    into.function("getenv", getenv, GETENV_DOC);
    into.function("setenv", setenv, SETENV_DOC);
    into.function("path-separator", path_separator, PATH_SEPARATOR_DOC);
}
