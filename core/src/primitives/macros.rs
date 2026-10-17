//! Recording keys and pressing them again.
//!
//! The state is in [`crate::managers::Macros`] and the replay in
//! [`crate::editor`]; this is the door Lisp comes through.
use super::*;
use crate::input::describe_keys;

/// How many times a call should repeat when nothing says.
const ONCE: usize = 1;

/// The repeat count a prefix argument asks for.
fn repetitions<B: BufferTrait>(args: &[ELispExp<B>]) -> usize {
    match args.first() {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 1.0 => *n as usize,
        _ => ONCE,
    }
}

pub const KMACRO_START_MACRO_DOC: &str = "(kmacro-start-macro): Start recording the keys pressed, \
         which still do what they do, and set the macro counter to 0. Return t, or nil when a \
         recording is running already.";

primitive!(kmacro_start_macro, _args, _env, ctx, {
    let started = ctx.macros_mut(|macros| macros.start());
    if started {
        ctx.set_echo_message("Recording keyboard macro...");
    } else {
        ctx.set_echo_message("Already recording a keyboard macro");
    }
    Ok(ELispExp::boolean(started))
});

pub const KMACRO_END_MACRO_DOC: &str = "(kmacro-end-macro): Stop recording, and keep the keys as \
         the last macro. Return how many there were, or nil when nothing was recorded: an empty \
         recording keeps the macro there was.";

primitive!(kmacro_end_macro, _args, _env, ctx, {
    match ctx.macros_mut(|macros| macros.finish()) {
        None => {
            ctx.set_echo_message("No keyboard macro is being recorded");
            Ok(ELispExp::nil())
        }
        Some(keys) => {
            ctx.set_echo_message(&format!("Recorded {} keys", keys.len()));
            Ok(ELispExp::number(keys.len() as f64))
        }
    }
});

pub const KMACRO_CANCEL_MACRO_DOC: &str = "(kmacro-cancel-macro): Stop recording and throw the \
         keys away; what they did stays done. Return t if a recording was running.";

primitive!(kmacro_cancel_macro, _args, _env, ctx, {
    let had = ctx.macros_mut(|macros| macros.cancel());
    if had {
        ctx.set_echo_message("Keyboard macro abandoned");
    }
    Ok(ELispExp::boolean(had))
});

pub const KMACRO_CALL_MACRO_DOC: &str = "(kmacro-call-macro &optional COUNT): Press the keys of \
         the last macro again, COUNT times, once by default. Return how many times it ran to the \
         end: a command in it that fails stops it there.";

primitive!(kmacro_call_macro, args, env, ctx, {
    let Some(keys) = ctx.macros(|macros| macros.last()) else {
        ctx.set_echo_message("No keyboard macro has been recorded");
        return Ok(ELispExp::nil());
    };
    let ran = ctx.replay_keys(&keys, repetitions(args), &env);
    Ok(ELispExp::number(ran as f64))
});

pub const KMACRO_END_AND_CALL_MACRO_DOC: &str = "(kmacro-end-and-call-macro &optional COUNT): \
         Stop recording, if a recording is running, and press the keys of the last macro COUNT \
         times, once by default.";

primitive!(kmacro_end_and_call_macro, args, env, ctx, {
    if ctx.macros(|macros| macros.is_recording()) {
        ctx.macros_mut(|macros| macros.finish());
    }
    let Some(keys) = ctx.macros(|macros| macros.last()) else {
        ctx.set_echo_message("No keyboard macro has been recorded");
        return Ok(ELispExp::nil());
    };
    let ran = ctx.replay_keys(&keys, repetitions(args), &env);
    Ok(ELispExp::number(ran as f64))
});

pub const KBD_MACRO_KEYS_DOC: &str = "(kbd-macro-keys): Return the last macro as a string of key \
         names, the way `define-key' and `define-kbd-macro' take them, or nil when none has been \
         recorded.\n\n\
         Example:\n\
         (kbd-macro-keys) => \"C-a C-k C-y\"";

primitive!(kbd_macro_keys, _args, _env, ctx, {
    Ok(match ctx.macros(|macros| macros.last()) {
        None => ELispExp::nil(),
        Some(keys) => ELispExp::string(describe_keys(&keys)),
    })
});

pub const DEFINE_KBD_MACRO_DOC: &str = "(define-kbd-macro NAME KEYS): Make a command named NAME \
         that presses KEYS, a string of key names as `define-key' takes, and make KEYS the last \
         macro. Return NAME.\n\n\
         Example:\n\
         (define-kbd-macro 'tidy-line \"C-a C-k C-y\")\n\
         (define-key nil \"C-c m\" 'tidy-line)";

primitive!(define_kbd_macro, args, env, ctx, {
    exact_arity(args, 2)?;
    let name = match &args[0] {
        ELispExp::Symbol(name) | ELispExp::String(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol naming the command".into(),
                got: other.clone(),
            });
        }
    };
    let ELispExp::String(spelling) = &args[1] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String of key names".into(),
            got: args[1].clone(),
        });
    };
    // Refused here rather than discovered when the command is run, where the
    // only thing to say is that a macro nobody can see is empty.
    let Some(keys) = parse_key_sequence(spelling) else {
        return Err(EvalError::RuntimeMessage(format!(
            "{spelling:?} is not a sequence of key names"
        )));
    };
    ctx.macros_mut(|macros| macros.set_last(keys.clone()));
    // A command whose body replays the keys. Built as Lisp rather than as a
    // Rust closure because a command is a Lisp function here, and one that was
    // not would be invisible to `describe-function` and to everything else
    // that reads a definition.
    let body = ELispExp::form(vec![
        ELispExp::symbol("kmacro-call-keys".into()),
        ELispExp::string(spelling.to_string()),
    ]);
    env.set_function(
        name.clone(),
        ELispExp::lambda(risp::Lambda {
            params: Vec::new(),
            optionals: Vec::new(),
            rest: None,
            body: vec![body],
            env: env.clone(),
            // The keys, said in the definition: `describe-function` on a macro
            // should show what it presses, which is the only thing there is to
            // know about it.
            doc: Some(std::sync::Arc::new(format!(
                "Keyboard macro: press {spelling}.\n\nMade with `define-kbd-macro'."
            ))),
        }),
    );
    ctx.register_command(&name, Vec::new());
    Ok(ELispExp::symbol(name.as_str().into()))
});

pub const KMACRO_CALL_KEYS_DOC: &str = "(kmacro-call-keys KEYS &optional COUNT): Press KEYS, a \
         string of key names, COUNT times, once by default, and return how many times they ran to \
         the end. A command made by `define-kbd-macro' calls this.\n\n\
         Example:\n\
         (kmacro-call-keys \"C-e C-n\" 3)";

primitive!(kmacro_call_keys, args, env, ctx, {
    let Some(ELispExp::String(spelling)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String of key names".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let Some(keys) = parse_key_sequence(spelling) else {
        return Err(EvalError::RuntimeMessage(format!(
            "{spelling:?} is not a sequence of key names"
        )));
    };
    let ran = ctx.replay_keys(&keys, repetitions(&args[1..]), &env);
    Ok(ELispExp::number(ran as f64))
});

pub const KMACRO_RECORDING_P_DOC: &str = "(kmacro-recording-p): Return how many keys have been \
         recorded so far, or nil when nothing is being recorded.";

primitive!(kmacro_recording_p, _args, _env, ctx, {
    Ok(ctx.macros(|macros| {
        if macros.is_recording() {
            ELispExp::number(macros.recorded() as f64)
        } else {
            ELispExp::nil()
        }
    }))
});

// ---------------------------------------------------------------------------
// The counter
// ---------------------------------------------------------------------------

pub const KMACRO_INSERT_COUNTER_DOC: &str = "(kmacro-insert-counter &optional WIDTH): Insert the \
         macro counter at point, padded with zeros to WIDTH digits, and add one to it. Return the \
         text inserted.\n\n\
         Example:\n\
         (kmacro-insert-counter 3)   ; 007";

primitive!(kmacro_insert_counter, args, _env, ctx, {
    let width = match args.first() {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => *n as usize,
        _ => 0,
    };
    let value = ctx.macros_mut(|macros| macros.take_counter(1));
    let text = if width == 0 {
        value.to_string()
    } else if value < 0 {
        format!("-{:0>width$}", value.unsigned_abs(), width = width)
    } else {
        format!("{value:0>width$}")
    };
    let inserted = ctx.with_current_buffer_mut(|buf| {
        let at = buf.text.cursor_pos_1d();
        crate::primitives::edits::insert_text(buf, at, &text)
    });
    if !inserted {
        return Err(EvalError::RuntimeMessage("This buffer is read-only".into()));
    }
    Ok(ELispExp::string(text))
});

pub const KMACRO_SET_COUNTER_DOC: &str = "(kmacro-set-counter VALUE): Set the macro counter to \
         VALUE, and return it.";

primitive!(kmacro_set_counter, args, _env, ctx, {
    let value = match args.first() {
        Some(ELispExp::Number(n)) if n.is_finite() => *n as i64,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    ctx.macros_mut(|macros| macros.set_counter(value));
    Ok(ELispExp::number(value as f64))
});

pub const KMACRO_COUNTER_DOC: &str = "(kmacro-counter): Return the macro counter, without adding \
         one to it.";

primitive!(kmacro_counter, _args, _env, ctx, {
    Ok(ELispExp::number(
        ctx.macros(|macros| macros.counter()) as f64
    ))
});

/// Register this module's primitives: recording and replaying keystrokes.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Recording keys and pressing them again. See `primitives::macros`, and
    // `managers::macros` for why a macro is keys rather than commands.
    into.command(
        "kmacro-start-macro",
        kmacro_start_macro,
        &[],
        KMACRO_START_MACRO_DOC,
    );
    into.command(
        "kmacro-end-macro",
        kmacro_end_macro,
        &[],
        KMACRO_END_MACRO_DOC,
    );
    into.command(
        "kmacro-cancel-macro",
        kmacro_cancel_macro,
        &[],
        KMACRO_CANCEL_MACRO_DOC,
    );
    into.command(
        "kmacro-call-macro",
        kmacro_call_macro,
        &["p"],
        KMACRO_CALL_MACRO_DOC,
    );
    into.command(
        "kmacro-end-and-call-macro",
        kmacro_end_and_call_macro,
        &["p"],
        KMACRO_END_AND_CALL_MACRO_DOC,
    );
    into.command(
        "kmacro-insert-counter",
        kmacro_insert_counter,
        &[],
        KMACRO_INSERT_COUNTER_DOC,
    );
    into.command(
        "kmacro-set-counter",
        kmacro_set_counter,
        &["nSet macro counter to: "],
        KMACRO_SET_COUNTER_DOC,
    );
    into.function("kmacro-counter", kmacro_counter, KMACRO_COUNTER_DOC);
    into.function(
        "kmacro-recording-p",
        kmacro_recording_p,
        KMACRO_RECORDING_P_DOC,
    );
    into.function("kbd-macro-keys", kbd_macro_keys, KBD_MACRO_KEYS_DOC);
    into.function("define-kbd-macro", define_kbd_macro, DEFINE_KBD_MACRO_DOC);
    into.function("kmacro-call-keys", kmacro_call_keys, KMACRO_CALL_KEYS_DOC);
}
