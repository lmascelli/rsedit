//! Major modes from Lisp: making one, the hooks it runs, and the file names
//! that choose it.
//!
//! Design: `doc/architettura-editor.typ, <modi>`.
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
         is made again, empty.\n\n\
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
            // TODO(SIN-4): a mode that exists already is replaced by an empty
            // one, without the keys and hooks other modules gave it.
            // Through `set_mode` rather than straight into the registry: a mode
            // made again has a new, empty grammar, and whatever was coloured
            // under the old one has to hear about it.
            ctx.set_mode(mode_name, MajorMode::new(mode_name));
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

/// Registers the primitives of major modes and their hooks.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("make-mode", make_mode, MAKE_MODE_DOC);
    into.function("add-hook", add_hook, ADD_HOOK_DOC);
    into.function("remove-hook", remove_hook, REMOVE_HOOK_DOC);
    into.function("add-auto-mode", add_auto_mode, ADD_AUTO_MODE_DOC);
}
