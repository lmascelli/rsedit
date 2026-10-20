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
use crate::editor::{COMMAND_ARG_COMPLETION_FUNCTION, COMMAND_COMPLETION_FUNCTION};
use crate::primitives::args;
use risp::call_callable;

/// Read a command name from an argument that may be a symbol or a string, so
/// `(commandp 'find-file)` and `(commandp "find-file")` both work -- M-x has a
/// string in hand, Lisp code has a symbol.
/// `(KIND PROMPT)` for one argument, as Lisp sees it -- already parsed, so the
/// prompting code dispatches on a symbol instead of re-splitting a string.
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

/// Answer what the editor can, then either prompt for the next argument or --
/// nothing left to ask -- run the command.
///
/// Both ends of the chain funnel through here: the first call from
/// `call-interactively`, and every later one from a confirmed prompt. One
/// place decides what happens next, so the two cannot drift.
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
            ELispExp::string(argument_title(spec, ctx)),
            ELispExp::primitive(command_arg_confirm, None),
            ELispExp::primitive(command_arg_complete, None),
            ELispExp::primitive(command_arg_cancel, None),
        ],
        env.clone(),
        ctx,
    )
}

/// The prompt shown while reading one of a command's arguments.
///
/// A bare `Find file:` says nothing about who is waiting on the answer, and
/// with more than one argument it says nothing about which one is being asked
/// for -- two prompts reading `Replace:` then `With:` look like two unrelated
/// questions. So the command's own name leads, and a multi-argument command
/// also shows its position:
///
/// ```text
///   find-file - Find file:
///   query-replace (2/2) - With:
/// ```
///
/// If nothing is pending -- `minibuffer-read` called straight from Lisp, as
/// `M-:` does -- the caller's prompt is used unchanged, since there is no
/// command to name.
fn argument_title<B: BufferTrait>(spec: &ArgSpec, ctx: &EditorState<B>) -> String {
    let prompt = spec.prompt();
    match ctx.pending_progress() {
        Some((name, at, total)) if total > 1 => format!("{name} ({at}/{total}) - {prompt}"),
        Some((name, _, _)) => format!("{name} - {prompt}"),
        None => prompt.to_string(),
    }
}

/// Convert the raw minibuffer input to the value the command should receive.
fn convert_arg<B: BufferTrait>(spec: &ArgSpec, input: &str) -> ELispExp<B> {
    match spec {
        ArgSpec::Number { .. } => ELispExp::number(input.trim().parse::<f64>().unwrap_or(0.0)),
        _ => ELispExp::string(input.to_string()),
    }
}

// One argument confirmed. Records it, then either prompts for the next or --
// this being the last -- applies the command.
//
// This is the link in the chain, and it is a primitive rather than a Lisp
// closure because collecting a command's arguments is the mechanism by which
// commands work at all: it has to be present whether or not any `.lisp` file
// loaded.
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

    ctx.accept_pending_arg(convert_arg(&spec, &input));
    advance_pending(env, ctx)
});

// The prompt was cancelled, so the command is abandoned. Nothing is applied:
// a half-collected argument list must never reach a command.
primitive!(command_arg_cancel, _args, _env, ctx, {
    ctx.take_pending_command();
    Ok(ELispExp::nil())
});

// Completion candidates for the argument currently being prompted for.
//
// The kind comes from the pending command rather than being captured when the
// prompt opened, which is what lets one primitive serve every argument without
// needing a closure to partially apply it.
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
            .filter(|name| name != "*Minibuffer*")
            .collect(),
        // Already whole paths, and already filtered against what was typed --
        // a file name is completed one path component at a time, so the
        // generic "starts with the prefix" filter below would be both wrong
        // and redundant. Returned directly for that reason.
        ArgSpec::File { .. } => {
            return Ok(ELispExp::proper_list(
                super::io::file_completions(&prefix)
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

pub const EXECUTE_EXTENDED_COMMAND_DOC: &str = "(execute-extended-command NAME): Run the command \
         NAME as M-x does, asking for its arguments, or say No such command when it is none.\n\n\
         Example:\n\
         (execute-extended-command \"save-buffer\")";

primitive!(execute_extended_command, args, env, ctx, {
    let name = match args.first() {
        Some(exp) => args::name(Some(exp))?,
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

/// Register this module's primitives: the command registry, and running a command by name.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // The registry itself. These are plain functions, not commands: they are
    // how Lisp inspects and extends the command set, not things a user runs
    // from M-x.
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
    // Which command's prompt is up, for anything that has to know -- a
    // preview, a mode line. See `primitives::commands`.
    into.function("pending-command", pending_command, PENDING_COMMAND_DOC);
}
