//! Modes from Lisp: making a major mode, the hooks it runs, and the file
//! names that choose it; making a minor mode, turning it on and off in a
//! buffer, and the major modes that turn it on.
//!
//! Design: `doc/architettura-editor.typ, <modi>`, and
//! `doc/architettura-editor.typ, <modi-minori>` for the minor modes.
use super::*;
use crate::primitives::args;
use crate::primitives::grammar::compile;

/// Returns the hook and the function `add-hook` and `remove-hook` take, as
/// the hook's name and the function to call, or an error naming the wrong one.
fn hook_and_function<B: BufferTrait>(
    args: &[ELispExp<B>],
) -> Result<(String, ELispExp<B>), EvalError<EditorState<B>>> {
    let ELispExp::String(hook) = &args[1] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String naming a hook, such as \"post-command-hook\"".into(),
            got: args[1].clone(),
        });
    };
    let ELispExp::Symbol(function) = &args[2] else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol naming a function".into(),
            got: args[2].clone(),
        });
    };
    Ok((hook.to_string(), ELispExp::symbol(function.to_string())))
}

pub const MAKE_MODE_DOC: &str = "(make-mode NAME): Make a major mode called NAME, a symbol, with \
         no keys, hooks, colouring or syntax of its own, and return t. A mode that exists already \
         keeps its keys, hooks and completion, whoever gave them, and starts its colouring and \
         syntax table again: its file can be loaded again, after a change, without each rule \
         coming twice.\n\n\
         Example:\n\
         (make-mode 'conf-mode)";

primitive!(make_mode, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        if let Some(ELispExp::Symbol(mode_name)) = args.get(0) {
            ctx.make_mode(mode_name);
            Ok(ELispExp::symbol("t".into()))
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: args.first().cloned().unwrap_or_else(ELispExp::nil),
            })
        }
    }
});

pub const ADD_HOOK_DOC: &str = "(add-hook MODE HOOK FUNCTION): Call FUNCTION, the name of a \
         function of no arguments, at HOOK -- a string, such as \"find-file-hook\" -- in the \
         buffers of major mode MODE, or in every buffer when MODE is nil. Return t, or nil when \
         there is no such mode. A function already there is not added again.\n\n\
         Example:\n\
         (add-hook 'rust-mode \"find-file-hook\" 'my-rust-setup)";

primitive!(add_hook, args, _env, ctx, {
    exact_arity(args, 3)?;
    // nil for the mode means every mode, the way it does in `define-key'.
    let mode = args::optional_name(args.first(), "Symbol naming a mode, or nil")?;
    let (hook, function) = hook_and_function(args)?;
    let Some(mode) = mode else {
        ctx.add_global_hook(&hook, function);
        return Ok(ELispExp::t());
    };
    // Reported outside the closure: the diagnostic writes the echo area,
    // which is a compartment of its own.
    let added = ctx.modes_mut(|modes| {
        modes.edit(&mode, |major| {
            let hooks = major.hooks.entry(hook).or_default();
            if !hooks.contains(&function) {
                hooks.push(function);
            }
        })
    });
    if added {
        Ok(ELispExp::t())
    } else {
        ctx.log_diagnostic(&format!("Mode {mode} does not exist"));
        Ok(ELispExp::nil())
    }
});

pub const REMOVE_HOOK_DOC: &str = "(remove-hook MODE HOOK FUNCTION): Take FUNCTION off HOOK in \
         major mode MODE, or off the hook for every mode when MODE is nil: the arguments \
         `add-hook' took. Return t, or nil when there is no such mode.";

primitive!(remove_hook, args, _env, ctx, {
    exact_arity(args, 3)?;
    let mode = args::optional_name(args.first(), "Symbol naming a mode, or nil")?;
    let (hook, function) = hook_and_function(args)?;
    Ok(ELispExp::boolean(ctx.modes_mut(|modes| {
        modes.remove_hook(mode.as_deref(), &hook, &function)
    })))
});

pub const ADD_AUTO_MODE_DOC: &str = "(add-auto-mode PATTERN MODE): Open in major mode MODE a file \
         whose whole path the regular expression PATTERN matches. Of the patterns that match a \
         file, the one added first is used. Return t, or nil when PATTERN is wrong, which the log \
         says.\n\n\
         Example:\n\
         (add-auto-mode \"\\\\.ino$\" 'cpp-mode)";

primitive!(add_auto_mode, args, _env, ctx, {
    exact_arity(args, 2)?;
    let ELispExp::String(pattern) = &args[0] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[0].clone(),
        });
    };
    let mode = match &args[1] {
        ELispExp::Symbol(name) | ELispExp::String(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol or String naming a mode".into(),
                got: other.clone(),
            });
        }
    };
    let Some(pattern) = compile(ctx, pattern) else {
        return Ok(ELispExp::nil());
    };
    ctx.add_auto_mode(pattern, &mode);
    Ok(ELispExp::t())
});

pub const MAKE_MINOR_MODE_DOC: &str = "(make-minor-mode NAME &optional LIGHTER): Make a minor \
         mode called NAME, a symbol, which the status line shows as LIGHTER, a string, while it \
         is on, nothing when LIGHTER is nil, and return t. Its keys are given with `define-key' \
         and its hooks with `add-hook', as a major mode's; a mode that exists already keeps \
         them. `define-minor-mode' makes one with its command.\n\n\
         Example:\n\
         (make-minor-mode 'my-mode \"My\")";

primitive!(make_minor_mode, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let name = args::name(args.first())?;
    let lighter = match args.get(1) {
        None => String::new(),
        Some(value) if value.is_nil() => String::new(),
        Some(ELispExp::String(text)) => text.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String, or nil".into(),
                got: other.clone(),
            });
        }
    };
    ctx.make_minor_mode(&name, &lighter);
    Ok(ELispExp::t())
});

/// Returns the minor mode the first argument names and the buffer the second
/// names, the current one when it is nil or left out; refused when the first
/// is no minor mode.
fn minor_and_buffer<B: BufferTrait>(
    args: &[ELispExp<B>],
    ctx: &EditorState<B>,
) -> Result<(String, String), EvalError<EditorState<B>>> {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let minor = args::name(args.first())?;
    if !ctx.modes(|modes| modes.is_minor(&minor)) {
        return Err(EvalError::RuntimeMessage(format!(
            "No such minor mode: {minor}"
        )));
    }
    let buffer = match args.get(1) {
        Some(ELispExp::String(name)) if !name.is_empty() => name.to_string(),
        _ => ctx.get_current_buffer_name(),
    };
    Ok((minor, buffer))
}

pub const MINOR_MODE_ON_DOC: &str = "(minor-mode-on MODE &optional BUFFER): Turn the minor mode \
         MODE on in BUFFER, the current one by default, and run its \"minor-mode-hook\" there. \
         Return t, or nil when it was on already.";

primitive!(minor_mode_on, args, env, ctx, {
    let (minor, buffer) = minor_and_buffer(args, ctx)?;
    Ok(ELispExp::boolean(ctx.minor_mode_on(&buffer, &minor, &env)))
});

pub const MINOR_MODE_OFF_DOC: &str = "(minor-mode-off MODE &optional BUFFER): Turn the minor \
         mode MODE off in BUFFER, the current one by default, and run its \"minor-mode-hook\" \
         there. Return t, or nil when it was not on.";

primitive!(minor_mode_off, args, env, ctx, {
    let (minor, buffer) = minor_and_buffer(args, ctx)?;
    Ok(ELispExp::boolean(ctx.minor_mode_off(&buffer, &minor, &env)))
});

pub const MINOR_MODE_P_DOC: &str = "(minor-mode-p MODE &optional BUFFER): Return t when the minor \
         mode MODE is on in BUFFER, the current one by default.\n\n\
         Example:\n\
         (minor-mode-p 'auto-fill-mode)";

primitive!(minor_mode_p, args, _env, ctx, {
    let (minor, buffer) = minor_and_buffer(args, ctx)?;
    Ok(ELispExp::boolean(
        ctx.minor_modes_of(&buffer).contains(&minor),
    ))
});

pub const MINOR_MODES_DOC: &str = "(minor-modes &optional BUFFER): Return the minor modes on in \
         BUFFER, the current one by default, as a list of symbols in the order they were turned \
         on.";

primitive!(minor_modes, args, _env, ctx, {
    let buffer = match args.first() {
        Some(ELispExp::String(name)) if !name.is_empty() => name.to_string(),
        _ => ctx.get_current_buffer_name(),
    };
    Ok(ELispExp::proper_list(
        ctx.minor_modes_of(&buffer)
            .into_iter()
            .map(ELispExp::symbol)
            .collect(),
    ))
});

pub const MINOR_MODE_ENABLE_DOC: &str = "(minor-mode-enable MODE &optional MAJOR): Turn the minor \
         mode MODE on in every buffer that gets the major mode MAJOR from now on, or every major \
         mode when MAJOR is nil, after its \"major-mode-hook\". Buffers already in it are left \
         as they are. Return t.\n\n\
         Example:\n\
         (minor-mode-enable 'auto-fill-mode 'fundamental-mode)";

primitive!(minor_mode_enable, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let minor = args::name(args.first())?;
    if !ctx.modes(|modes| modes.is_minor(&minor)) {
        return Err(EvalError::RuntimeMessage(format!(
            "No such minor mode: {minor}"
        )));
    }
    let major = args::optional_name(args.get(1), "Symbol naming a major mode, or nil")?;
    ctx.modes_mut(|modes| modes.add_minor_rule(major.as_deref(), &minor));
    Ok(ELispExp::t())
});

/// Registers the primitives of major modes and their hooks.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("make-mode", make_mode, MAKE_MODE_DOC);
    into.function("add-hook", add_hook, ADD_HOOK_DOC);
    into.function("remove-hook", remove_hook, REMOVE_HOOK_DOC);
    into.function("add-auto-mode", add_auto_mode, ADD_AUTO_MODE_DOC);
    into.function("make-minor-mode", make_minor_mode, MAKE_MINOR_MODE_DOC);
    into.function("minor-mode-on", minor_mode_on, MINOR_MODE_ON_DOC);
    into.function("minor-mode-off", minor_mode_off, MINOR_MODE_OFF_DOC);
    into.function("minor-mode-p", minor_mode_p, MINOR_MODE_P_DOC);
    into.function("minor-modes", minor_modes, MINOR_MODES_DOC);
    into.function(
        "minor-mode-enable",
        minor_mode_enable,
        MINOR_MODE_ENABLE_DOC,
    );
}
