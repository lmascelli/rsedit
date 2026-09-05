//! Primitives for the command registry: registering commands, asking what a
//! command is, and invoking one interactively.
//!
//! The split of responsibility here is deliberate. The **registry** is Rust --
//! it is editor state, looked up on every key press, and parsing specs once at
//! registration turns a bad spec into a boot-time error. The **prompting** is
//! Lisp: `minibuffer-read` fires its callback on a later keystroke, so
//! collecting N arguments is a continuation chain, and a chain is a closure in
//! Lisp versus a resumable state machine in Rust.
use super::*;
use crate::commands::ArgSpec;
use crate::lisp::call_callable;

/// A Lisp function of (KIND PREFIX) returning completion candidates, consulted
/// before the built-in ones. Unset by default; bind it to replace how any
/// argument completes, the same way `*minibuffer-read-function*` replaces the
/// prompt itself.
const COMPLETION_HOOK: &str = "*command-arg-completion-function*";

/// Read a command name from an argument that may be a symbol or a string, so
/// `(commandp 'find-file)` and `(commandp "find-file")` both work -- M-x has a
/// string in hand, Lisp code has a symbol.
fn command_name<B: BufferTrait>(exp: &ELispExp<B>) -> Result<String, EvalError<EditorState<B>>> {
    match exp {
        ELispExp::Symbol(name) | ELispExp::String(name) => Ok(name.to_string()),
        other => Err(EvalError::WrongArgumentType {
            expected: "Symbol or String".into(),
            got: other.clone(),
        }),
    }
}

/// `(KIND PROMPT)` for one argument, as Lisp sees it -- already parsed, so the
/// prompting code dispatches on a symbol instead of re-splitting a string.
fn spec_to_lisp<B: BufferTrait>(spec: &ArgSpec) -> ELispExp<B> {
    ELispExp::proper_list(vec![
        ELispExp::symbol(spec.kind().into()),
        ELispExp::string(spec.prompt().to_string()),
    ])
}

pub const REGISTER_COMMAND_DOC: &str = "(register-command NAME SPECS): Register NAME (a symbol or \
         string) as a command the user can run with M-x or bind to a key. SPECS is a list of \
         Emacs-style argument codes, one per argument the editor should collect: \"sPROMPT\" \
         reads a string, \"nPROMPT\" a number, \"bPROMPT\" a buffer name with completion, and \
         \"fPROMPT\" a file name with completion. A command taking no arguments registers with \
         nil.\n\n\
         The function itself is an ordinary function, defined however you like -- registering \
         only records that it may be invoked by name. Re-registering a name replaces its specs.\n\n\
         Example:\n\
         (register-command 'find-file '(\"fFind file: \"))\n\
         (register-command 'save-buffer nil)";

primitive!(register_command, args, env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let name = command_name(&args[0])?;

    // Accept the spec list whether it arrived as data or as syntax.
    //
    // `'("sName: ")` written in source is converted to a cons chain by the
    // reader, but the same list produced by a macro's backquote is still a
    // `Form`, and no list primitive traverses one. Rather than make callers
    // care which they have, this reads both -- it is asking for "a list of
    // strings", and both representations are that.
    let codes: Vec<ELispExp<B>> = match &args[1] {
        ELispExp::Form(items) => items.to_vec(),
        other if other.is_nil() => Vec::new(),
        other => other.iter().collect(),
    };

    let mut specs = Vec::with_capacity(codes.len());
    for code in &codes {
        let ELispExp::String(code) = code else {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: code.clone(),
            });
        };
        specs.push(
            ArgSpec::parse(code)
                .map_err(|why| EvalError::RuntimeMessage(format!("{name}: {why}")))?,
        );
    }

    // When the command is a Lisp function we know its arity, so a spec list
    // that disagrees with it is caught here -- at definition time -- instead of
    // becoming a `WrongNumberOfArguments` the first time somebody runs the
    // command. Primitives have no introspectable arity, so they are taken on
    // trust.
    if let Some(ELispExp::Lambda(lambda)) = env.get_function(&name) {
        let required = lambda.params.len();
        let accepted = required + lambda.optionals.len();
        let supplied = specs.len();
        if lambda.rest.is_none() && (supplied < required || supplied > accepted) {
            return Err(EvalError::RuntimeMessage(format!(
                "{name}: {supplied} argument spec(s) registered but the function takes {}",
                if required == accepted {
                    format!("{required}")
                } else {
                    format!("{required} to {accepted}")
                }
            )));
        }
    }

    ctx.register_command(&name, specs);
    Ok(ELispExp::t())
});

pub const COMMANDP_DOC: &str = "(commandp NAME): Return t if NAME (a symbol or string) names a \
         command -- something the user can run with M-x -- and nil otherwise.\n\n\
         Example:\n\
         (commandp 'find-file) => t\n\
         (commandp 'car)       => nil";

primitive!(commandp, args, _env, ctx, {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(ELispExp::boolean(ctx.is_command(&command_name(&args[0])?)))
});

pub const COMMAND_ARGS_DOC: &str = "(command-args NAME): Return the arguments the editor collects \
         for command NAME, as a list of (KIND PROMPT) pairs where KIND is one of the symbols \
         string, number, buffer or file. Returns nil both for a command taking no arguments and \
         for a name that is not a command -- use `commandp' to tell those apart.\n\n\
         Example:\n\
         (command-args 'find-file) => ((file \"Find file: \"))";

primitive!(command_args, args, _env, ctx, {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let specs = ctx
        .command_specs(&command_name(&args[0])?)
        .unwrap_or_default();
    Ok(ELispExp::proper_list(
        specs.iter().map(spec_to_lisp).collect(),
    ))
});

pub const ALL_COMMANDS_DOC: &str = "(all-commands): Return the names of every registered command, \
         as a sorted list of strings. This is what M-x completes over.\n\n\
         Example:\n\
         (all-commands) => (\"find-file\" \"next-line\" ...)";

primitive!(all_commands, _args, _env, ctx, {
    Ok(ELispExp::proper_list(
        ctx.command_names()
            .into_iter()
            .map(ELispExp::string)
            .collect(),
    ))
});

pub const CALL_INTERACTIVELY_DOC: &str = "(call-interactively COMMAND): Run COMMAND (a symbol or \
         string) the way a key press or M-x would: collect the arguments named by its \
         registration, then call it with them.\n\n\
         Every key bound to a bare command invocation goes through here, which makes this the \
         single place to observe or advise command execution.\n\n\
         A command taking no arguments -- or a function that was never registered -- is simply \
         called with none, so binding an ordinary Lisp function to a key keeps working. \
         Otherwise the arguments are collected by `read-command-args', which prompts for them \
         one at a time; because the minibuffer delivers its input on a later keystroke, COMMAND \
         runs after this call has already returned.\n\n\
         Example:\n\
         (call-interactively 'find-file)   ; prompts, then opens the file";

primitive!(call_interactively, args, env, ctx, {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let name = command_name(&args[0])?;

    // Anything left on the stack while no prompt is open belongs to a
    // minibuffer that was closed by neither confirm nor cancel.
    //
    // This is hygiene, not a correctness fix: because these are stacked, a new
    // command sits above any orphan and pops itself off, so an orphan cannot
    // steal input -- it just never goes away. Clearing here keeps repeated
    // abandonment from growing the stack without bound.
    if !ctx.minibuffer_is_open() {
        ctx.clear_pending_commands();
    }

    // Looked up and released before anything is evaluated: what runs below can
    // call `register-command`, and holding the registry lock across that would
    // deadlock on a non-reentrant `RwLock`.
    let specs = ctx.command_specs(&name).unwrap_or_default();

    if specs.is_empty() {
        let callable = env
            .get_function(&name)
            .ok_or_else(|| EvalError::UndefinedFunction(name.clone()))?;
        return call_callable(&callable, &[], env.clone(), ctx);
    }

    ctx.push_pending_command(name, specs.clone());
    prompt_for(&specs[0], env, ctx)
});

/// Open a minibuffer prompt for SPEC, with this module's own primitives as the
/// confirm and cancel callbacks.
///
/// Going through `minibuffer-read` by name rather than calling the built-in
/// prompt directly is deliberate: that indirection is what
/// `*minibuffer-read-function*` exists for, and a replacement prompt should
/// serve command arguments too.
fn prompt_for<B: BufferTrait>(
    spec: &ArgSpec,
    env: std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let reader = env
        .get_function("minibuffer-read")
        .ok_or_else(|| EvalError::UndefinedFunction("minibuffer-read".into()))?;
    call_callable(
        &reader,
        &[
            ELispExp::string(spec.prompt().to_string()),
            ELispExp::primitive(command_arg_confirm, None),
            ELispExp::primitive(command_arg_complete, None),
            ELispExp::primitive(command_arg_cancel, None),
        ],
        env.clone(),
        ctx,
    )
}

/// Convert the raw minibuffer input to the value the command should receive.
fn convert_arg<B: BufferTrait>(spec: &ArgSpec, input: &str) -> ELispExp<B> {
    match spec {
        ArgSpec::Number { .. } => ELispExp::number(input.trim().parse::<f64>().unwrap_or(0.0)),
        _ => ELispExp::string(input.to_string()),
    }
}

/// One argument confirmed. Records it, then either prompts for the next or --
/// this being the last -- applies the command.
///
/// This is the link in the chain, and it is a primitive rather than a Lisp
/// closure because collecting a command's arguments is the mechanism by which
/// commands work at all: it has to be present whether or not any `.lisp` file
/// loaded.
primitive!(command_arg_confirm, args, env, ctx, {
    let input = match args.first() {
        Some(ELispExp::String(s)) => s.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
        None => String::new(),
    };

    let Some(spec) = ctx.pending_current_spec() else {
        // No command is waiting -- the prompt outlived whatever started it.
        return Ok(ELispExp::nil());
    };

    if let Some(next) = ctx.accept_pending_arg(convert_arg(&spec, &input)) {
        return prompt_for(&next, env, ctx);
    }

    let Some((name, collected)) = ctx.take_pending_command() else {
        return Ok(ELispExp::nil());
    };
    let callable = env
        .get_function(&name)
        .ok_or_else(|| EvalError::UndefinedFunction(name.clone()))?;
    call_callable(&callable, &collected, env.clone(), ctx)
});

/// The prompt was cancelled, so the command is abandoned. Nothing is applied:
/// a half-collected argument list must never reach a command.
primitive!(command_arg_cancel, _args, _env, ctx, {
    ctx.take_pending_command();
    Ok(ELispExp::nil())
});

/// Completion candidates for the argument currently being prompted for.
///
/// The kind comes from the pending command rather than being captured when the
/// prompt opened, which is what lets one primitive serve every argument without
/// needing a closure to partially apply it.
primitive!(command_arg_complete, args, env, ctx, {
    let prefix = match args.first() {
        Some(ELispExp::String(s)) => s.to_string(),
        _ => String::new(),
    };
    let Some(spec) = ctx.pending_current_spec() else {
        return Ok(ELispExp::nil());
    };

    // A Lisp override, if one is bound, decides for every kind.
    if let Some(hook) = env.get_variable(COMPLETION_HOOK)
        && hook.is_truthy()
    {
        return call_callable(
            &hook,
            &[
                ELispExp::symbol(spec.kind().into()),
                ELispExp::string(prefix),
            ],
            env.clone(),
            ctx,
        );
    }

    let candidates = match spec {
        // The prompt's own buffer is live while the prompt is open, but it is
        // never a sensible answer to "which buffer?", so it is not offered.
        ArgSpec::Buffer { .. } => ctx
            .buffer_names()
            .into_iter()
            .filter(|name| name != "*Minibuffer*")
            .collect(),
        // Free text, and file completion is not implemented yet.
        _ => Vec::new(),
    };
    Ok(ELispExp::proper_list(
        candidates
            .into_iter()
            .filter(|name| name.starts_with(&prefix))
            .map(ELispExp::string)
            .collect(),
    ))
});

pub const ALL_BUFFER_NAMES_DOC: &str = "(all-buffer-names): Return the names of every live buffer, \
         as a sorted list of strings. This is what a buffer-name argument completes over.\n\n\
         Example:\n\
         (all-buffer-names) => (\"*Messages*\" \"*scratch*\")";

primitive!(all_buffer_names, _args, _env, ctx, {
    Ok(ELispExp::proper_list(
        ctx.buffer_names()
            .into_iter()
            .map(ELispExp::string)
            .collect(),
    ))
});

pub const EXECUTE_EXTENDED_COMMAND_DOC: &str = "(execute-extended-command NAME): Run the command \
         named NAME, or report that there is no such command. This is what M-x calls once the \
         user has typed a name and pressed Return.\n\n\
         Example:\n\
         (execute-extended-command \"save-buffer\")";

primitive!(execute_extended_command, args, env, ctx, {
    let name = match args.first() {
        Some(exp) => command_name(exp)?,
        None => {
            return Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: 0,
            });
        }
    };
    if !ctx.is_command(&name) {
        ctx.set_echo_message(&format!("No such command: {name}"));
        return Ok(ELispExp::nil());
    }
    call_interactively(&[ELispExp::string(name)], env, ctx)
});

pub const COMMAND_COMPLETIONS_DOC: &str = "(command-completions PREFIX): Return the command names \
         starting with PREFIX. This is what M-x completes over; rebind \
         *command-completion-function* to complete differently.\n\n\
         Example:\n\
         (command-completions \"save-\") => (\"save-buffer\")";

primitive!(command_completions, args, _env, ctx, {
    let prefix = match args.first() {
        Some(ELispExp::String(s)) => s.to_string(),
        _ => String::new(),
    };
    Ok(ELispExp::proper_list(
        ctx.command_names()
            .into_iter()
            .filter(|name| name.starts_with(&prefix))
            .map(ELispExp::string)
            .collect(),
    ))
});

/// A Lisp function of one argument returning M-x completion candidates.
/// Unset by default; bind it for fuzzy matching or any other policy.
const MX_COMPLETION_HOOK: &str = "*command-completion-function*";

pub const COMMAND_EXECUTE_PROMPT_DOC: &str = "(command-execute-prompt): Prompt for a command name \
         with completion, then run it. This is M-x.\n\n\
         Bound to M-x by default. The completion candidates come from \
         `command-completions' unless *command-completion-function* is bound to something else.";

primitive!(command_execute_prompt, _args, env, ctx, {
    let reader = env
        .get_function("minibuffer-read")
        .ok_or_else(|| EvalError::UndefinedFunction("minibuffer-read".into()))?;
    let completer = match env.get_variable(MX_COMPLETION_HOOK) {
        Some(hook) if hook.is_truthy() => hook,
        _ => ELispExp::primitive(command_completions, None),
    };
    call_callable(
        &reader,
        &[
            ELispExp::string("M-x".into()),
            ELispExp::primitive(execute_extended_command, None),
            completer,
            ELispExp::nil(),
        ],
        env.clone(),
        ctx,
    )
});
