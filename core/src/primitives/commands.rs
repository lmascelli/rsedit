//! The command registry from Lisp, and running a command: `register-command`,
//! what a command is, `call-interactively` and `M-x`. A command's arguments
//! are asked for by a chain of prompts, with the pending command keeping what
//! has been answered between one and the next.
//!
//! Design: `doc/architettura-editor.typ, <comandi>`.
use super::*;
use crate::commands::ArgSpec;
use crate::editor::{COMMAND_ARG_COMPLETION_FUNCTION, COMMAND_COMPLETION_FUNCTION};
use crate::primitives::args;
use risp::call_callable;

/// Returns SPEC as Lisp sees it, `(KIND PROMPT)`: parsed already, so that
/// the code that prompts goes by a symbol rather than splitting a string.
fn spec_to_lisp<B: BufferTrait>(spec: &ArgSpec) -> ELispExp<B> {
    ELispExp::proper_list(vec![
        ELispExp::symbol(spec.kind().into()),
        ELispExp::string(spec.prompt().to_string()),
    ])
}

pub const REGISTER_COMMAND_DOC: &str = "(register-command NAME SPECS): Make the function NAME a \
         command, which M-x reaches and a key collects the arguments of, and return t. SPECS has \
         one code for each argument -- \"sPROMPT\" a string, \"nPROMPT\" a number, \"bPROMPT\" a \
         buffer's name, \"fPROMPT\" a file's, \"p\" the prefix count, \"P\" the raw prefix, \"r\" \
         the region, which is two -- or is nil. Codes that do not fit the function's parameters \
         are refused.\n\n\
         Example:\n\
         (register-command 'save-buffer nil)";

primitive!(register_command, args, env, ctx, {
    exact_arity(args, 2)?;
    let name = args::name(args.first())?;

    let codes = args::list(&args[1]);

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

    // A Lisp function's arity is known, so specs that disagree with it are
    // refused now, rather than failing the first time the command runs. A
    // primitive's arity is not known, and is taken on trust.
    if let Some(ELispExp::Lambda(lambda)) = env.get_function(&name) {
        let required = lambda.params.len();
        let accepted = required + lambda.optionals.len();
        // Values, not specs: `r` alone supplies two of them.
        let supplied: usize = specs.iter().map(ArgSpec::arity).sum();
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

pub const COMMANDP_DOC: &str = "(commandp NAME): Return t when NAME, a symbol or a string, names \
         a command, and nil otherwise.\n\n\
         Example:\n\
         (commandp 'find-file)   ; t";

primitive!(commandp, args, _env, ctx, {
    exact_arity(args, 1)?;
    Ok(ELispExp::boolean(
        ctx.is_command(&args::name(args.first())?),
    ))
});

pub const COMMAND_ARGS_DOC: &str = "(command-args NAME): Return the arguments the editor collects \
         for the command NAME, as a list of (KIND PROMPT), KIND being one of string, number, \
         buffer, file, count, raw-count and region; nil for a command with none and for a name \
         that is no command.\n\n\
         Example:\n\
         (command-args 'find-file)   ; ((file \"Find file: \"))";

primitive!(command_args, args, _env, ctx, {
    exact_arity(args, 1)?;
    let specs = ctx
        .command_specs(&args::name(args.first())?)
        .unwrap_or_default();
    Ok(ELispExp::proper_list(
        specs.iter().map(spec_to_lisp).collect(),
    ))
});

pub const ALL_COMMANDS_DOC: &str = "(all-commands): Return the names of every command, sorted, as \
         strings: what M-x completes over.";

primitive!(all_commands, _args, _env, ctx, {
    Ok(ELispExp::proper_list(
        ctx.command_names()
            .into_iter()
            .map(ELispExp::string)
            .collect(),
    ))
});

pub const CALL_INTERACTIVELY_DOC: &str = "(call-interactively COMMAND): Run COMMAND as a key \
         press would: ask for the arguments its registration names, one prompt at a time, then \
         call it with them. A function that is no command is called with no arguments. A command \
         with arguments to ask for runs after this returns, when the last prompt is answered, and \
         one that takes the region is refused when there is none.\n\n\
         Example:\n\
         (call-interactively 'find-file)";

primitive!(call_interactively, args, env, ctx, {
    exact_arity(args, 1)?;
    let name = args::name(args.first())?;

    // What is on the stack while no prompt is open was left by a prompt
    // closed by neither confirm nor cancel. A new command would sit above it
    // and never read it; cleared, the stack does not grow.
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

    // Fixed here, at the start, and carried by the pending command from now
    // on -- see `Invocation`.
    let invocation = ctx.capture_invocation();
    if specs.contains(&ArgSpec::Region) && invocation.region.is_none() {
        // Refused before a single prompt opens. A command that asked two
        // questions and only then said "there is no region" would have wasted
        // the answers.
        return Err(EvalError::RuntimeMessage(
            "The mark is not set now, so there is no region".into(),
        ));
    }

    ctx.push_pending_command(name, specs, invocation);
    advance_pending(env, ctx)
});

/// Fills the arguments the editor answers itself, then prompts for the next
/// one or, with nothing left to ask, runs the command. The first call comes
/// from `call-interactively`, every later one from a confirmed prompt.
fn advance_pending<B: BufferTrait>(
    env: std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    if let Some(next) = ctx.fill_answerable_args() {
        return prompt_for(&next, env, ctx);
    }
    let Some((name, collected)) = ctx.take_pending_command() else {
        return Ok(ELispExp::nil());
    };
    let callable = env
        .get_function(&name)
        .ok_or_else(|| EvalError::UndefinedFunction(name.clone()))?;
    call_callable(&callable, &collected, env.clone(), ctx)
}

/// Opens a prompt for SPEC, with this module's primitives as its callbacks.
/// Through `minibuffer-read` by name, so that a prompt put in its place by
/// `*minibuffer-read-function*` asks for arguments too.
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
            ELispExp::string(argument_title(spec, ctx)),
            ELispExp::primitive(command_arg_confirm, None),
            ELispExp::primitive(command_arg_complete, None),
            ELispExp::primitive(command_arg_cancel, None),
        ],
        env.clone(),
        ctx,
    )
}

/// Returns the title of the prompt for SPEC: the pending command's name, its
/// place among the arguments when there is more than one, then SPEC's prompt.
///
/// ```text
///   find-file - Find file:
///   query-replace (2/2) - With:
/// ```
///
/// With no command pending, as when Lisp calls `minibuffer-read` itself, the
/// prompt alone.
fn argument_title<B: BufferTrait>(spec: &ArgSpec, ctx: &EditorState<B>) -> String {
    let prompt = spec.prompt();
    match ctx.pending_progress() {
        Some((name, at, total)) if total > 1 => format!("{name} ({at}/{total}) - {prompt}"),
        Some((name, _, _)) => format!("{name} - {prompt}"),
        None => prompt.to_string(),
    }
}

/// Returns INPUT as the value a command receives for SPEC, or `None` when it
/// is not one: anything but a number, nothing included, for a number.
fn convert_arg<B: BufferTrait>(spec: &ArgSpec, input: &str) -> Option<ELispExp<B>> {
    match spec {
        ArgSpec::Number { .. } => input
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(ELispExp::number),
        _ => Some(ELispExp::string(input.to_string())),
    }
}

// The confirm callback of an argument's prompt: records the argument, then
// prompts for the next or runs the command. A primitive, so that commands
// collect their arguments whatever `.lisp` file failed to load.
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

    // Asked again, as Emacs asks, rather than handed to the command as a
    // value nobody typed. The prompt has closed already, and what was typed
    // is in its history.
    let Some(value) = convert_arg(&spec, &input) else {
        ctx.set_echo_message("Please enter a number");
        return prompt_for(&spec, env, ctx);
    };
    ctx.accept_pending_arg(value);
    advance_pending(env, ctx)
});

// The cancel callback: the command is dropped, so that an argument list half
// collected never reaches it.
primitive!(command_arg_cancel, _args, _env, ctx, {
    ctx.take_pending_command();
    Ok(ELispExp::nil())
});

// The completion callback: the candidates for the argument being asked for.
// It reads the argument's kind from the pending command, so one primitive
// serves every argument.
primitive!(command_arg_complete, args, env, ctx, {
    let prefix = match args.first() {
        Some(ELispExp::String(s)) => s.to_string(),
        _ => String::new(),
    };
    let Some(spec) = ctx.pending_current_spec() else {
        return Ok(ELispExp::nil());
    };

    // A Lisp override, if one is bound, decides for every kind.
    if let Some(hook) = env.get_variable(COMMAND_ARG_COMPLETION_FUNCTION)
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
            .filter(|name| name != crate::feature::minibuffer::PROMPT_BUFFER)
            .collect(),
        // Whole paths, filtered already: a file name completes a component
        // at a time, which the filter below knows nothing of.
        ArgSpec::File { .. } => {
            return Ok(ELispExp::proper_list(
                crate::paths::file_completions(&prefix, &ctx.default_directory())
                    .into_iter()
                    .map(ELispExp::string)
                    .collect(),
            ));
        }
        // Free text: nothing to complete over.
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

pub const EXECUTE_EXTENDED_COMMAND_DOC: &str = "(execute-extended-command NAME &optional \
         PREFIX): Run the command NAME as M-x does, asking for its arguments, or say No such \
         command when it is none. PREFIX, when given, is the prefix argument it runs with, as \
         \"P\" gives one: nil, (4) for C-u, a number, or - for C-u -.\n\n\
         Example:\n\
         (execute-extended-command \"save-buffer\")";

primitive!(execute_extended_command, args, env, ctx, {
    let name = match args.first() {
        Some(exp) if args.len() <= 2 => args::name(Some(exp))?,
        _ => {
            return Err(EvalError::WrongNumberOfArguments {
                expected: if args.is_empty() { 1 } else { 2 },
                got: args.len(),
            });
        }
    };
    let prefix = match args.get(1) {
        Some(raw) => Some(crate::managers::prefix_from_raw(raw).ok_or_else(|| {
            EvalError::RuntimeMessage(format!("{raw:?} is not a prefix argument"))
        })?),
        None => None,
    };
    if !ctx.is_command(&name) {
        ctx.set_echo_message(&format!("No such command: {name}"));
        return Ok(ELispExp::nil());
    }
    if let Some(prefix) = prefix {
        ctx.set_prefix_argument(prefix);
    }
    call_interactively(&[ELispExp::string(name)], env, ctx)
});

pub const COMMAND_COMPLETIONS_DOC: &str = "(command-completions PREFIX): Return the names of the \
         commands that begin with PREFIX: what M-x completes with, unless \
         `*command-completion-function*' names another function.\n\n\
         Example:\n\
         (command-completions \"save-\")";

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

pub const COMMAND_EXECUTE_PROMPT_DOC: &str = "(command-execute-prompt): Ask for the name of a \
         command, completing it, and run it. M-x.";

primitive!(command_execute_prompt, _args, env, ctx, {
    let reader = env
        .get_function("minibuffer-read")
        .ok_or_else(|| EvalError::UndefinedFunction("minibuffer-read".into()))?;
    let completer = match env.get_variable(COMMAND_COMPLETION_FUNCTION) {
        Some(hook) if hook.is_truthy() => hook,
        _ => ELispExp::primitive(command_completions, None),
    };
    // The prefix argument typed before M-x is the command's, as in Emacs. It
    // goes when M-x's own key has run, before the name is read, so the
    // callback carries it: (lambda (name) (execute-extended-command name 'P)).
    let run = match ctx.prefix_argument() {
        None => ELispExp::primitive(execute_extended_command, None),
        Some(prefix) => {
            let name = || ELispExp::symbol("name".into());
            let quoted = ELispExp::form(vec![
                ELispExp::symbol("quote".into()),
                crate::managers::raw_prefix(Some(prefix)),
            ]);
            let lambda = ELispExp::form(vec![
                ELispExp::symbol("lambda".into()),
                ELispExp::form(vec![name()]),
                ELispExp::form(vec![
                    ELispExp::symbol("execute-extended-command".into()),
                    name(),
                    quoted,
                ]),
            ]);
            risp::eval(&lambda, env.clone(), ctx)?
        }
    };
    call_callable(
        &reader,
        &[
            ELispExp::string("M-x".into()),
            run,
            completer,
            ELispExp::nil(),
        ],
        env.clone(),
        ctx,
    )
});

pub const PENDING_COMMAND_DOC: &str = "(pending-command): Return the name of the command whose \
         arguments a prompt is asking for, or nil when none is: how a preview knows which \
         command's prompt is open.\n\n\
         Example:\n\
         (equal (pending-command) \"string-rectangle\")";

primitive!(pending_command, _args, _env, ctx, {
    Ok(match ctx.pending_progress() {
        Some((name, _, _)) => ELispExp::string(name),
        None => ELispExp::nil(),
    })
});

/// Registers the primitives of the command registry and of running a
/// command.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // The registry, which Lisp reads and extends: functions, not commands a
    // user would run from M-x.
    into.function("register-command", register_command, REGISTER_COMMAND_DOC);
    into.function("commandp", commandp, COMMANDP_DOC);
    into.function("command-args", command_args, COMMAND_ARGS_DOC);
    into.function("all-commands", all_commands, ALL_COMMANDS_DOC);
    into.function(
        "execute-extended-command",
        execute_extended_command,
        EXECUTE_EXTENDED_COMMAND_DOC,
    );
    into.function(
        "command-completions",
        command_completions,
        COMMAND_COMPLETIONS_DOC,
    );
    into.command(
        "command-execute-prompt",
        command_execute_prompt,
        &[],
        COMMAND_EXECUTE_PROMPT_DOC,
    );
    into.function("all-buffer-names", all_buffer_names, ALL_BUFFER_NAMES_DOC);
    into.function(
        "call-interactively",
        call_interactively,
        CALL_INTERACTIVELY_DOC,
    );
    // Which command's prompt is up, for what has to know: a preview, a mode
    // line.
    into.function("pending-command", pending_command, PENDING_COMMAND_DOC);
}
