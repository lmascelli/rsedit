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

pub const KMACRO_START_MACRO_DOC: &str = "(kmacro-start-macro): Begin recording the keys you \
         press, so they can be pressed again. Returns t, or nil if a recording is already \
         running.\n\n\
         Everything you do is recorded *and* done: this is not a rehearsal. Finish with \
         `kmacro-end-macro' and run what you recorded with `kmacro-call-macro'.\n\n\
         Keys are what is recorded, not commands, so a macro does whatever its keys mean when it \
         is replayed -- rebind one and the macro follows. It also means a command that reads a \
         character of its own, like `zap-to-char', records the character too.\n\n\
         The counter is set to zero. See `kmacro-insert-counter'.\n\n\
         Example:\n\
         (define-key nil \"C-x (\" 'kmacro-start-macro)";

primitive!(kmacro_start_macro, _args, _env, ctx, {
    let started = ctx.macros_mut(|macros| macros.start());
    if started {
        ctx.set_echo_message("Recording keyboard macro...");
    } else {
        ctx.set_echo_message("Already recording a keyboard macro");
    }
    Ok(ELispExp::boolean(started))
});

pub const KMACRO_END_MACRO_DOC: &str = "(kmacro-end-macro): Stop recording, and keep what was \
         recorded as the last macro. Returns the number of keys, or nil if nothing was being \
         recorded.\n\n\
         The keys that ended the recording are not part of it, so a macro never ends by stopping \
         a recording that is not running.\n\n\
         An empty recording is not kept: there would be nothing to replay, and `C-x e' doing \
         nothing with no way to tell why is worse than being told there is no macro.\n\n\
         Example:\n\
         (define-key nil \"C-x )\" 'kmacro-end-macro)";

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

pub const KMACRO_CANCEL_MACRO_DOC: &str = "(kmacro-cancel-macro): Stop recording and throw away \
         what was recorded. Returns t if a recording was running.\n\n\
         What was done while recording stays done -- recording changes nothing about what the \
         keys did. This throws away the *recording*, not the edits, which `undo' is for.";

primitive!(kmacro_cancel_macro, _args, _env, ctx, {
    let had = ctx.macros_mut(|macros| macros.cancel());
    if had {
        ctx.set_echo_message("Keyboard macro abandoned");
    }
    Ok(ELispExp::boolean(had))
});

pub const KMACRO_CALL_MACRO_DOC: &str = "(kmacro-call-macro &optional COUNT): Press the keys of \
         the last macro again, COUNT times (default once). Returns how many times it ran.\n\n\
         Each key goes through the same door a key from the keyboard does, so a macro does \
         exactly what pressing its keys does -- prompts open, prefix arguments count, a command \
         that reads a character reads the one that was recorded.\n\n\
         A repetition that fails stops everything, and says so. Every key after a failure would \
         be pressed in a state the recording never saw, which is how a macro does damage rather \
         than merely not working.\n\n\
         Each repetition is one undo step, so a macro run twenty times can be walked back one \
         repetition at a time.\n\n\
         Nothing can interrupt it: the keys are replayed inside one command, on the thread that \
         would deliver a `C-g'. A macro that calls itself is stopped by a limit on how deeply \
         replays may nest -- not by the execution budget, which a replay barely touches while \
         growing the call stack until the process would abort.\n\n\
         Example:\n\
         (define-key nil \"C-x e\" 'kmacro-call-macro)";

primitive!(kmacro_call_macro, args, env, ctx, {
    let Some(keys) = ctx.macros(|macros| macros.last()) else {
        ctx.set_echo_message("No keyboard macro has been recorded");
        return Ok(ELispExp::nil());
    };
    let ran = ctx.replay_keys(&keys, repetitions(args), &env);
    Ok(ELispExp::number(ran as f64))
});

pub const KMACRO_END_AND_CALL_MACRO_DOC: &str = "(kmacro-end-and-call-macro &optional COUNT): \
         Finish a recording if one is running, and then run the last macro COUNT times.\n\n\
         One key for both, so that `C-x e' means \"do that again\" whether or not you have \
         remembered to stop recording -- which is the common case, since the thing you want \
         immediately after recording a macro is to run it.";

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

pub const KBD_MACRO_KEYS_DOC: &str = "(kbd-macro-keys): The last macro, as a string of key names \
         -- `\"C-a C-k C-y\"' -- or nil if none has been recorded.\n\n\
         The same syntax `define-key' and `define-kbd-macro' take, which is the whole of saving a \
         macro: print this, put it in your configuration, and it is a macro again next time.\n\n\
         Example:\n\
         (kbd-macro-keys) => \"C-a C-k C-y\"";

primitive!(kbd_macro_keys, _args, _env, ctx, {
    Ok(match ctx.macros(|macros| macros.last()) {
        None => ELispExp::nil(),
        Some(keys) => ELispExp::string(describe_keys(&keys)),
    })
});

pub const DEFINE_KBD_MACRO_DOC: &str = "(define-kbd-macro NAME KEYS): Make KEYS -- a string of \
         key names, as `define-key' takes -- into a command called NAME. Returns NAME.\n\n\
         A real command: it appears in `M-x', `describe-function' explains it, `define-key' binds \
         it and `where-is' finds it. Nothing about it is special except what it does.\n\n\
         This is how a macro outlives the session it was recorded in -- `(define-kbd-macro \
         'tidy-line \"C-a C-k C-y\")' in your configuration is a macro you have on every start, \
         with no file format and nothing to load.\n\n\
         Also makes KEYS the last macro, so `kmacro-call-macro' runs it.\n\n\
         Example:\n\
         (define-kbd-macro 'tidy-line \"C-a C-k C-y\")\n\
         (define-key nil \"C-c t\" 'tidy-line)";

primitive!(define_kbd_macro, args, env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
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
        ELispExp::lambda(crate::lisp::Lambda {
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

pub const KMACRO_CALL_KEYS_DOC: &str = "(kmacro-call-keys KEYS &optional COUNT): Press KEYS -- a \
         string of key names -- COUNT times. Returns how many times it ran.\n\n\
         What a command made by `define-kbd-macro' is: the keys are in the definition, so the \
         command is readable rather than a handle to something stored elsewhere. Useful on its \
         own for a module that wants to press keys without recording them first.";

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

pub const KMACRO_RECORDING_P_DOC: &str = "(kmacro-recording-p): How many keys have been recorded \
         so far, or nil when nothing is being recorded.\n\n\
         For a mode line that says so. Recording changes nothing visible on its own -- the keys \
         do what they always did -- so without something showing it, the only way to tell is to \
         remember.";

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
         macro counter and advance it by one. Returns what was inserted.\n\n\
         What a macro is for beyond repetition: numbering a run of lines, filling a column, \
         generating a sequence. Record `kmacro-insert-counter' inside a macro and each repetition \
         puts in the next number.\n\n\
         WIDTH pads with leading zeros to that many digits -- `(kmacro-insert-counter 3)' gives \
         `007' -- which is what makes a numbered list sort the way it reads.\n\n\
         The counter is set to zero when a recording starts, and otherwise only by \
         `kmacro-set-counter'. It advances when it is *inserted*, not once per repetition, so a \
         macro that inserts it twice gets two different numbers.\n\n\
         Example:\n\
         (define-key nil \"C-x C-k C-i\" 'kmacro-insert-counter)";

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
        return Err(EvalError::RuntimeMessage(
            "This buffer is read-only".into(),
        ));
    }
    Ok(ELispExp::string(text))
});

pub const KMACRO_SET_COUNTER_DOC: &str = "(kmacro-set-counter VALUE): Set the macro counter to \
         VALUE. Returns it.\n\n\
         For starting a numbered run at something other than zero, and for starting it again \
         after a run that went wrong.";

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

pub const KMACRO_COUNTER_DOC: &str = "(kmacro-counter): The macro counter's current value.\n\n\
         Read without advancing it, which `kmacro-insert-counter' does. For a module that wants \
         to put the number somewhere other than the buffer.";

primitive!(kmacro_counter, _args, _env, ctx, {
    Ok(ELispExp::number(
        ctx.macros(|macros| macros.counter()) as f64,
    ))
});
