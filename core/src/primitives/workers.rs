//! Defining, stopping and listing background workers.
//!
//! The mechanism is in [`crate::worker`]; this is the door Lisp comes through.
use super::*;
use crate::modes::highlighter::TURN_INTERVAL;
use crate::task::WorkerMessage;
use crate::worker::{DEFAULT_WORKER_FUEL, LispWorker};
use std::time::Duration;

/// A name written as a symbol or a string, which are the same thing here.
///
/// `'indexer` reads better at a definition and `"indexer"` reads better when
/// the name was computed, and a mechanism has no business preferring one.
fn worker_name<B: BufferTrait>(value: Option<&ELispExp<B>>) -> Option<String> {
    match value {
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) => Some(name.to_string()),
        _ => None,
    }
}

pub const DEFINE_WORKER_DOC: &str = "(define-worker NAME FIBER &optional INTERVAL): Run FIBER in the \
         background, resuming it every INTERVAL milliseconds until it \
         finishes.\n\n\
         FIBER is a `fiber' -- see `yield'. Each turn resumes it once, so a worker is written as \
         a loop that says where it is willing to be put down:\n\n\
         \x20 (define-worker 'indexer\n\
         \x20   (fiber (while t\n\
         \x20            (if (index--stale-p) (index--one-chunk) nil)\n\
         \x20            (yield))))\n\n\
         INTERVAL defaults to the interval syntax colouring runs at, which is what a worker \
         wants unless it is watching something slower.\n\n\
         Defining a worker under a name that already has one *replaces* it: the old one retires \
         the next time it wakes. That is what makes re-evaluating a module safe -- without it, \
         loading a file twice would leave two copies of its worker running against each other.\n\n\
         A turn gets its own execution budget, `worker-fuel', which is separate from the one \
         commands run under: nothing the user does can cut a worker's turn short. A turn that \
         runs past its own budget is stopped and the worker retired, because a worker that never \
         reaches a `yield' holds the thread that syntax colouring and the prescanner also run on.\n\n\
         Returns NAME.";

primitive!(define_worker, args, env, ctx, {
    if !(2..=3).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let Some(name) = worker_name(args.first()) else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol or String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    // Refused here rather than discovered a turn later on another thread,
    // where the only way to report it is a line in the log.
    if args[1].fiber_is_done().is_none() {
        return Err(EvalError::WrongArgumentType {
            expected: "Fiber".into(),
            got: args[1].clone(),
        });
    }
    let interval = match args.get(2) {
        Some(ELispExp::Number(ms)) if ms.is_finite() && *ms >= 1.0 => {
            Duration::from_millis(*ms as u64)
        }
        _ => TURN_INTERVAL,
    };

    // Bumped before the job is posted, so the replacement is already the live
    // generation by the time the old one next wakes to check.
    let generation = ctx.runtime_mut(|runtime| runtime.next_worker_generation(&name));
    let worker = LispWorker::new(
        name.clone(),
        generation,
        args[1].clone(),
        env.clone(),
        worker_fuel(&env),
    );
    if !ctx.send_to_worker(WorkerMessage::Schedule {
        task: Box::new(worker),
        interval,
    }) {
        // The scheduler is gone, which happens only as the editor shuts down.
        // Undone rather than left claiming to be running.
        ctx.runtime_mut(|runtime| runtime.retire_worker(&name, generation));
        return Ok(ELispExp::nil());
    }
    Ok(ELispExp::symbol(name.as_str().into()))
});

pub const STOP_WORKER_DOC: &str = "(stop-worker NAME): Retire the background worker called NAME. \
         Returns t if there was one, nil otherwise.\n\n\
         The worker finds out the next time it wakes, which is at most one turn away. Nothing \
         waits for that: waiting would block the thread you are typing on until the worker \
         thread got round to it, which is the arrangement workers exist to avoid.";

primitive!(stop_worker, args, _env, ctx, {
    let Some(name) = worker_name(args.first()) else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol or String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    Ok(ELispExp::boolean(
        ctx.runtime_mut(|runtime| runtime.stop_worker(&name)),
    ))
});

pub const RUNNING_WORKERS_DOC: &str = "(running-workers): The names of the background workers currently running, as a list of \
         strings.";

primitive!(running_workers, _args, _env, ctx, {
    let mut names = ctx.runtime(|runtime| runtime.running_workers());
    // Sorted so that the answer is the same twice running: it comes out of a
    // hash map, and a list that reorders itself between calls is one nothing
    // can be tested against.
    names.sort();
    Ok(ELispExp::proper_list(
        names.into_iter().map(ELispExp::string).collect(),
    ))
});

/// The per-turn allowance as Lisp currently defines it.
///
/// Read when the worker is defined rather than every turn: a turn is the one
/// place this mechanism must not do anything it does not have to, and a
/// worker that wants a different allowance can be defined again.
fn worker_fuel<B: BufferTrait>(env: &std::sync::Arc<Env<EditorState<B>>>) -> u32 {
    match env.get_variable(WORKER_FUEL) {
        Some(ELispExp::Number(fuel)) if fuel.is_finite() && fuel >= 1.0 => fuel as u32,
        _ => DEFAULT_WORKER_FUEL,
    }
}

/// The name of the Lisp variable holding one worker turn's execution budget.
pub const WORKER_FUEL: &str = "worker-fuel";
