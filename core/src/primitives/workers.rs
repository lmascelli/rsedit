//! Defining, stopping and listing background workers.
//!
//! The mechanism is in [`crate::background::worker`]; this is the door Lisp comes through.
use super::*;
use crate::background::WorkerMessage;
use crate::background::worker::{BackgroundJob, DEFAULT_WORKER_FUEL, JobBody, LispWorker};
use crate::modes::highlighter::TURN_INTERVAL;
use std::time::Duration;

/// A name written as a symbol or a string, which are the same thing here.
///
/// `'indexer` reads better at a definition and `"indexer"` reads better when
/// the name was computed, and a mechanism has no business preferring one.
fn worker_name<B: BufferTrait>(value: Option<&ELispExp<B>>) -> Option<String> {
    crate::primitives::args::optional_name(value, "Symbol or String naming a worker")
        .ok()
        .flatten()
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

pub const STOP_WORKER_DOC: &str = "(stop-worker NAME): Retire the background worker or job \
         called NAME. Returns t if there was one, nil otherwise.\n\n\
         It finds out the next time it wakes, which is at most one turn away. Nothing waits for \
         that: waiting would block the thread you are typing on until the worker thread got \
         round to it, which is the arrangement this whole mechanism exists to avoid.\n\n\
         A job stopped this way does not call its ON-DONE. It was not done -- it was stopped -- \
         and whoever stopped it is on the thread that would have been told.";

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

pub const RUNNING_WORKERS_DOC: &str = "(running-workers): The names of the background workers \
         and jobs currently running, as a list of strings.\n\n\
         One list because there is one namespace: `define-worker' and `background-call' name \
         what they start in the same place, so that a name means one thing and `stop-worker' \
         stops whichever it is.";

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
    // At least one step, because a worker with no fuel never reaches its own
    // first `yield` and so never finishes either -- it would be resumed
    // forever having done nothing.
    match env.number_at_least(WORKER_FUEL, 1.0) {
        Some(fuel) => fuel as u32,
        None => DEFAULT_WORKER_FUEL,
    }
}

/// The name of the Lisp variable holding one worker turn's execution budget.
pub const WORKER_FUEL: &str = "worker-fuel";

pub const BACKGROUND_CALL_DOC: &str = "(background-call NAME JOB &optional ON-PROGRESS ON-DONE \
         INTERVAL): Do JOB in the background under the name NAME, and return NAME at once \
         without waiting for it.\n\n\
         This is how anything that may take longer than a frame is started. JOB is either a \
         function of no arguments -- run once, on the background thread -- or a `fiber', which \
         is resumed a turn at a time and is the shape to use when the work can be done in \
         pieces:\n\n\
         \x20 (background-call 'manpage-index\n\
         \x20                  (fiber (let ((left manpage-path))\n\
         \x20                           (while left\n\
         \x20                             (manpage--index-one (car left))\n\
         \x20                             (setq left (cdr left))\n\
         \x20                             (yield))))\n\
         \x20                  'manpage--index-progress\n\
         \x20                  'manpage--index-done)\n\n\
         A `while' and not a `dolist': a `yield' is legal only where the evaluator can record \
         a way back to it, which is a body statement or a form of a `while'. A `dolist' body \
         is not one, and a yield there is refused -- so a fiber walks its own list.\n\n\
         ON-PROGRESS is called with NAME after every turn but the last, so each `(yield)' in a \
         fiber is a progress point and a plain function has none. ON-DONE is called once, with \
         NAME and t if the job reached its end or nil if it was stopped by an error or by \
         spending its allowance -- and it is called either way, because a mode that is never \
         told stays waiting for ever.\n\n\
         Both run on the thread that runs commands, *between* commands, and so may do anything \
         a command may do. The job itself may not: it runs while you are typing, and the editor \
         it can see is in whatever state the command in progress has left it in. So a job \
         computes and stores; a callback is where the mode is told, and the mode is what keeps \
         itself consistent with what it is told.\n\n\
         Neither callback is given what the job produced. They are given the name it ran under, \
         and look the rest up -- because between the turn that produced something and the \
         callback that reports it the user may have killed the buffer, changed the variable, or \
         started the same job again.\n\n\
         Starting a job under a name that already has one *replaces* it: the old job retires the \
         next time it wakes, and whatever callbacks it had already queued are dropped rather \
         than delivered against the new job's state. That is what makes re-evaluating a module, \
         or pressing the same key twice, safe. `stop-worker' stops a job and `running-workers' \
         lists one -- jobs and workers share a namespace, and a name cannot be both.\n\n\
         INTERVAL is how many milliseconds apart a fiber's turns are, defaulting to the interval \
         syntax colouring runs at. It means nothing for a job that is a plain function, which \
         has one turn.\n\n\
         Each turn gets `worker-fuel' to spend, separately from the budget commands run under. A \
         turn that runs past it is stopped and the job retired, because a turn that never ends \
         holds the thread syntax colouring and the prescanner also run on.";

primitive!(background_call, args, env, ctx, {
    if !(2..=5).contains(&args.len()) {
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
    // A fiber is the streaming shape and anything else callable is the
    // one-shot shape, so the two are told apart here rather than by asking
    // the caller to say which it meant.
    let body = if args[1].fiber_is_done().is_some() {
        JobBody::Steps(args[1].clone())
    } else if is_callable(&args[1]) {
        JobBody::Once(args[1].clone())
    } else {
        // Refused here rather than discovered a turn later on another thread,
        // where the only way to report it is a line in the log.
        return Err(EvalError::WrongArgumentType {
            expected: "a fiber, or a function to call".into(),
            got: args[1].clone(),
        });
    };
    let on_progress = optional_callback(args.get(2))?;
    let on_done = optional_callback(args.get(3))?;
    let interval = match args.get(4) {
        Some(ELispExp::Number(ms)) if ms.is_finite() && *ms >= 1.0 => {
            Duration::from_millis(*ms as u64)
        }
        _ => TURN_INTERVAL,
    };

    // Bumped before the job is posted, so the replacement is already the live
    // generation by the time the old one next wakes to check.
    let generation = ctx.runtime_mut(|runtime| runtime.next_worker_generation(&name));
    let job = BackgroundJob::new(
        name.clone(),
        generation,
        body,
        on_progress,
        on_done,
        env.clone(),
        worker_fuel(&env),
    );
    // Counted before the task is sent, not inside it: the worker may not
    // reach it for a moment, and a frame drawn in that moment would decide
    // nothing was running and go back to sleep until a key was pressed.
    ctx.begin_background_work();
    if !ctx.send_to_worker(WorkerMessage::Schedule {
        task: Box::new(job),
        interval,
    }) {
        // The scheduler is gone, which happens only as the editor shuts down.
        // Undone rather than left claiming to be running -- and no ON-DONE,
        // because nothing is going to run to be done.
        ctx.finish_background_work();
        ctx.runtime_mut(|runtime| runtime.retire_worker(&name, generation));
        ctx.log_diagnostic("[ERROR] background-call: the background worker has gone");
        return Ok(ELispExp::nil());
    }
    Ok(ELispExp::symbol(name.as_str().into()))
});

/// Whether VALUE is something [`call_callable`](risp::call_callable)
/// could call.
///
/// A symbol is accepted without asking whether it names a function yet: a
/// module that starts a job at the bottom of the file and defines its
/// callback above it is fine, and one that names a function it never defines
/// finds out in the log. What is refused is a value that could never be a
/// call however the environment changes -- a number, a string, nil.
fn is_callable<B: BufferTrait>(value: &ELispExp<B>) -> bool {
    matches!(
        value,
        ELispExp::Symbol(_) | ELispExp::Lambda(_) | ELispExp::Primitive { .. }
    ) && value.is_truthy()
}

/// A callback argument: nothing, or something callable.
fn optional_callback<B: BufferTrait>(
    value: Option<&ELispExp<B>>,
) -> Result<Option<ELispExp<B>>, EvalError<EditorState<B>>> {
    match value {
        None => Ok(None),
        Some(value) if value.is_nil() => Ok(None),
        Some(value) if is_callable(value) => Ok(Some(value.clone())),
        Some(value) => Err(EvalError::WrongArgumentType {
            expected: "a function to call, or nil".into(),
            got: value.clone(),
        }),
    }
}

/// Register this module's primitives: background workers and named background jobs.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Background workers. Plain functions rather than commands: a worker is
    // defined by a module at load time, not run from M-x.
    into.function("define-worker", define_worker, DEFINE_WORKER_DOC);
    into.function("stop-worker", stop_worker, STOP_WORKER_DOC);
    into.function("running-workers", running_workers, RUNNING_WORKERS_DOC);
    // Work that must not happen on the thread that draws. See `worker`, which
    // states the rule the whole editor follows for it.
    into.function("background-call", background_call, BACKGROUND_CALL_DOC);
}
