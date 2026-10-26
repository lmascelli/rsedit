//! What the editor reports: the log, the call stack of the last failure,
//! and the echo area.
//!
//! Design: `doc/architettura-editor.typ, <valutazione>`.
use super::*;

pub const LOG_DOC: &str = "(log MESSAGE): Add the string MESSAGE to the log, without showing \
         it. It reaches the log file, when there is one, within a moment.\n\n\
         Example:\n\
         (log \"my-mode: index rebuilt\")";

primitive!(log, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        if let ELispExp::String(message) = &args[0] {
            ctx.log_diagnostic(&message);
            Ok(ELispExp::nil())
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: args.first().cloned().unwrap_or_else(ELispExp::nil),
            })
        }
    }
});

pub const ALL_LOGS_DOC: &str = "(all-logs): Return the log, oldest line first, as a list of \
         strings: what `log' and `message' added and what the editor reported itself. The \
         oldest lines go once there are many.\n\n\
         Example:\n\
         (length (all-logs))";

primitive!(all_logs, _args, _env, ctx, {
    Ok(ELispExp::proper_list(
        ctx.get_logs().into_iter().map(ELispExp::string).collect(),
    ))
});

pub const BACKTRACE_DOC: &str = "(backtrace): Return the call stack of the most recent failure, \
         innermost call first, as a list of function names; nil once that failure has been \
         reported. A call made last in its function is not in the list: by the time it ran, the \
         function that made it had returned.\n\n\
         Example:\n\
         (backtrace) => (\"eval-string\" \"eval-expression-confirm\" \"funcall\" \"minibuffer-confirm\")";

primitive!(backtrace, _args, _env, ctx, {
    // `backtrace` is itself a call, so by the time this runs, the generic
    // per-call instrumentation has already pushed a frame for it -- drop
    // that one frame so the result reflects the stack as it stood before
    // this call, not this call's own (otherwise self-referential) frame.
    let mut frames = ctx.backtrace();
    if !frames.is_empty() {
        frames.remove(0);
    }
    Ok(ELispExp::proper_list(
        frames.into_iter().map(ELispExp::string).collect(),
    ))
});

pub const SET_ECHO_MESSAGE_DOC: &str = "(set-echo-message MESSAGE): Show MESSAGE in the editor's \
         echo area (the status line at the bottom of the frame). Prefer \
         `message', which also keeps MESSAGE in the log, unless you \
         specifically don't want that.\n\n\
         MESSAGE stays on screen for `echo-message-timeout' seconds and is \
         then no longer drawn; set that variable to nil to leave messages up \
         until something replaces them. Each new message starts its own \
         timeout.\n\n\
         Example:\n\
         (set-echo-message \"Saved\")\n\
         (setq echo-message-timeout nil) ; messages no longer time out";

primitive!(set_echo_message, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else if let ELispExp::String(message) = &args[0] {
        ctx.set_echo_message(message);
        Ok(ELispExp::nil())
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        })
    }
});

/// Registers the primitives of what the editor reports.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("log", log, LOG_DOC);
    into.function("all-logs", all_logs, ALL_LOGS_DOC);
    into.function("backtrace", backtrace, BACKTRACE_DOC);
    into.function("set-echo-message", set_echo_message, SET_ECHO_MESSAGE_DOC);
}
