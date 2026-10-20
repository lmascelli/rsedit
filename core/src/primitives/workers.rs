//! Starting, stopping and listing work in the background, and tuning the
//! scheduler that runs it.
//!
//! The job these start is in [`super::lisp_job`], and the scheduler is the
//! editor's WorkManager, in [`crate::work`]; this is the door Lisp comes
//! through.
use super::*;
use crate::editor::WORKER_FUEL;
use crate::modes::highlighter::TURN_INTERVAL;
use crate::primitives::lisp_job::{Body, DEFAULT_WORKER_FUEL, LispJob};
use crate::work::{Class, OptionError, Place};
use std::collections::HashSet;
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

/// The name in VALUE, or the error a primitive gives for one that is not.
fn required_name<B: BufferTrait>(
    value: Option<&ELispExp<B>>,
) -> Result<String, EvalError<EditorState<B>>> {
    worker_name(value).ok_or_else(|| EvalError::WrongArgumentType {
        expected: "Symbol or String".into(),
        got: value.cloned().unwrap_or_else(ELispExp::nil),
    })
}

/// What a plain `(yield)` waits: INTERVAL milliseconds when it is a number of
/// at least one, the default wait otherwise.
fn interval<B: BufferTrait>(value: Option<&ELispExp<B>>) -> Duration {
    match value {
        Some(ELispExp::Number(ms)) if ms.is_finite() && *ms >= 1.0 => {
            Duration::from_millis(*ms as u64)
        }
        _ => TURN_INTERVAL,
    }
}

/// Hand JOB to the editor's WorkManager under NAME, and note that Lisp
/// started it. NAME, or nil when the work has shut down.
fn start<B: BufferTrait>(
    ctx: &EditorState<B>,
    door: &str,
    name: &str,
    job: LispJob<B>,
) -> ELispExp<B> {
    let spec = job.spec();
    match ctx.work().submit(spec, Box::new(job)) {
        Some(id) => {
            ctx.runtime_mut(|runtime| runtime.adopt_lisp_job(name, id));
            ELispExp::symbol(name.into())
        }
        // Only as the editor shuts down.
        None => {
            ctx.log_diagnostic(&format!(
                "[ERROR] {door}: {name} was not started: the editor's work has shut down"
            ));
            ELispExp::nil()
        }
    }
}

pub const DEFINE_WORKER_DOC: &str = "(define-worker NAME FIBER &optional INTERVAL): Run FIBER, a \
         `fiber', in the background under NAME, a step at a time for as long as it goes on, and \
         return NAME. Each `yield' says when the next step comes: (yield) after INTERVAL \
         milliseconds, 40 unless given; (yield 'continue) at the next turn; (yield '(sleep MS)) \
         after MS; (yield '(wait TOPIC...)) when `emit-event' emits one of the TOPICs; (yield \
         '(wait-at-most MS TOPIC...)) at whichever comes first. A worker defined under a name in \
         use replaces what ran under it. An error, or a step that spends all its `worker-fuel', \
         stops the worker, and the log says why.\n\n\
         Example:\n\
         (define-worker 'my-ticker (fiber (while t (my-tick) (yield '(sleep 1000)))))";

primitive!(define_worker, args, env, ctx, {
    if !(2..=3).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let name = required_name(args.first())?;
    // Refused here rather than discovered a step later on another thread,
    // where the only way to report it is a line in the log.
    if args[1].fiber_is_done().is_none() {
        return Err(EvalError::WrongArgumentType {
            expected: "Fiber".into(),
            got: args[1].clone(),
        });
    }
    let job = LispJob::worker(
        name.clone(),
        args[1].clone(),
        interval(args.get(2)),
        env.clone(),
        worker_fuel(&env),
    );
    Ok(start(ctx, "define-worker", &name, job))
});

pub const STOP_WORKER_DOC: &str = "(stop-worker NAME): Stop the background work called NAME at \
         the end of the step it is taking, and return t; nil when nothing runs under NAME. NAME \
         is any name `work-list' shows: a worker, a job of `background-call', the editor's own \
         work, or a shell command by the name of its buffer. A job stopped does not call its \
         ON-DONE, and the editor's own work stays stopped until the editor starts again.\n\n\
         Example:\n\
         (stop-worker \"*Shell Output*\")";

primitive!(stop_worker, args, _env, ctx, {
    let name = required_name(args.first())?;
    // Silenced at once, in the registry, rather than when the job hears of
    // it: whatever it queued is dropped from this command on. A job that
    // finished before it could be stopped is still owed its ON-DONE.
    Ok(ELispExp::boolean(ctx.work().cancel_named(&name)))
});

pub const RUNNING_WORKERS_DOC: &str = "(running-workers): Return the names of the workers and the \
         jobs started from Lisp that are still running, as a sorted list of strings. `work-list' \
         shows the editor's own work as well.\n\n\
         Example:\n\
         (member \"my-index\" (running-workers))";

primitive!(running_workers, _args, _env, ctx, {
    let running: HashSet<_> = ctx.work().jobs().into_iter().map(|job| job.id).collect();
    let mut names = ctx.runtime(|runtime| runtime.lisp_jobs_among(&running));
    // Sorted so that the answer is the same twice running: it comes out of a
    // hash map, and a list that reorders itself between calls is one nothing
    // can be tested against.
    names.sort();
    Ok(ELispExp::proper_list(
        names.into_iter().map(ELispExp::string).collect(),
    ))
});

/// The per-step allowance as Lisp currently defines it.
///
/// Read when the job is started rather than at every step: a job that wants
/// a different allowance can be started again.
fn worker_fuel<B: BufferTrait>(env: &std::sync::Arc<Env<EditorState<B>>>) -> u32 {
    // At least one step, because a job with no fuel never reaches its own
    // first `yield` and so never finishes either.
    match env.number_at_least(WORKER_FUEL, 1.0) {
        Some(fuel) => fuel as u32,
        None => DEFAULT_WORKER_FUEL,
    }
}

pub const BACKGROUND_CALL_DOC: &str = "(background-call NAME JOB &optional ON-PROGRESS ON-DONE \
         INTERVAL): Run JOB in the background under NAME, and return NAME at once. JOB is a \
         function of no arguments, called once, or a `fiber', resumed a step at a time as \
         `define-worker' resumes one. ON-PROGRESS is called with NAME after each step but the \
         last, and ON-DONE with NAME and t when JOB reached its end, or nil when it failed; both \
         run between commands. A job started under a name in use replaces the one running, whose \
         callbacks are then not called. INTERVAL is how many milliseconds a plain (yield) waits: \
         40 unless given.\n\n\
         Example:\n\
         (defvar my-sum nil)\n\
         (defun my-done (name ok) (message \"%s finished: %s\" name ok))\n\
         (background-call 'my-sum (lambda () (setq my-sum (my-slow-sum))) nil 'my-done)";

primitive!(background_call, args, env, ctx, {
    if !(2..=5).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let name = required_name(args.first())?;
    // A fiber is the streaming shape and anything else callable is the
    // one-shot shape, so the two are told apart here rather than by asking
    // the caller to say which it meant.
    let body = if args[1].fiber_is_done().is_some() {
        Body::Fiber(args[1].clone())
    } else if is_callable(&args[1]) {
        Body::Once(args[1].clone())
    } else {
        // Refused here rather than discovered a step later on another
        // thread, where the only way to report it is a line in the log.
        return Err(EvalError::WrongArgumentType {
            expected: "a fiber, or a function to call".into(),
            got: args[1].clone(),
        });
    };
    let on_progress = optional_callback(args.get(2))?;
    let on_done = optional_callback(args.get(3))?;
    let job = LispJob::call(
        name.clone(),
        body,
        interval(args.get(4)),
        on_progress,
        on_done,
        env.clone(),
        worker_fuel(&env),
    );
    Ok(start(ctx, "background-call", &name, job))
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

pub const EMIT_EVENT_DOC: &str = "(emit-event TOPIC): Wake every worker and job waiting for \
         TOPIC, a symbol or a string, and return nil. A topic carries nothing: the job it wakes \
         looks for itself at what changed, and emissions that come together wake it once. The \
         editor emits buffer-changed and syntax-changed itself.\n\n\
         Example:\n\
         (emit-event 'my-index-stale)";

primitive!(emit_event, args, _env, ctx, {
    exact_arity(args, 1)?;
    let topic = required_name(args.first())?;
    ctx.work().topic(&topic).emit();
    Ok(ELispExp::nil())
});

pub const WORK_OPTION_DOC: &str = "(work-option NAME): Return the value of NAME, an option of the \
         scheduler that runs the background work, or signal an error when there is no option of \
         that name. `work-options' lists them with their values, and the `background' page says \
         what each does.\n\n\
         Example:\n\
         (work-option 'high-steps)";

primitive!(work_option, args, _env, ctx, {
    exact_arity(args, 1)?;
    let name = required_name(args.first())?;
    match ctx.work().options().get(&name) {
        Some(value) => Ok(ELispExp::number(value as f64)),
        None => Err(EvalError::RuntimeMessage(format!(
            "work-option: there is no option called {name}; (work-options) lists them"
        ))),
    }
});

pub const SET_WORK_OPTION_DOC: &str = "(set-work-option NAME VALUE): Set NAME, an option of the \
         scheduler that runs the background work, to VALUE, a whole number, and return VALUE; it \
         counts from the scheduler's next step. Signal an error, and change nothing, when there \
         is no such option or it does not take VALUE: the message says what it takes.\n\n\
         Example:\n\
         (set-work-option 'high-steps 6)";

primitive!(set_work_option, args, _env, ctx, {
    exact_arity(args, 2)?;
    let name = required_name(args.first())?;
    let value = match &args[1] {
        ELispExp::Number(n) if n.is_finite() && n.fract() == 0.0 && *n >= 0.0 => *n as u64,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "a whole number, from 0 up".into(),
                got: other.clone(),
            });
        }
    };
    match ctx.work().options().set(&name, value) {
        Ok(()) => Ok(args[1].clone()),
        Err(OptionError::Unknown(_)) => Err(EvalError::RuntimeMessage(format!(
            "set-work-option: there is no option called {name}; (work-options) lists them"
        ))),
        Err(OptionError::OutOfRange { name, min, max }) => Err(EvalError::RuntimeMessage(format!(
            "set-work-option: {name} takes a value from {min} to {max}"
        ))),
    }
});

pub const WORK_OPTIONS_DOC: &str = "(work-options): Return every option of the scheduler that \
         runs the background work, with its value, as a property list: (high-steps 4 event-delay \
         20 ...). `work-option' reads one, and `set-work-option' sets one.\n\n\
         Example:\n\
         (work-options)";

primitive!(work_options, args, _env, ctx, {
    exact_arity(args, 0)?;
    let options = ctx.work().options().all();
    Ok(ELispExp::proper_list(
        options
            .into_iter()
            .flat_map(|(name, value)| {
                [
                    ELispExp::symbol(name.into()),
                    ELispExp::number(value as f64),
                ]
            })
            .collect(),
    ))
});

pub const WORK_LIST_DOC: &str = "(work-list): Return everything in the background, the editor's \
         own work as well as Lisp's, as a list of property lists, one each. A job's gives its \
         name, kind job, class high or normal, state ready, running, sleeping or waiting, and \
         steps, total-ms and max-ms: how many steps it has taken, how long they took and how long \
         the longest did. A thread's, such as a shell command's, gives its name, kind thread, \
         state running or stopping, and running-ms.\n\n\
         Example:\n\
         (mapcar (lambda (work) (nth 1 work)) (work-list))";

primitive!(work_list, args, _env, ctx, {
    exact_arity(args, 0)?;
    let symbol = |name: &str| ELispExp::symbol(name.into());
    let millis = |span: Duration| ELispExp::number(span.as_millis() as f64);
    let jobs = ctx.work().jobs().into_iter().map(|job| {
        let class = match job.class {
            Class::High => "high",
            Class::Normal => "normal",
        };
        let state = match job.place {
            Place::Ready => "ready",
            Place::Running => "running",
            Place::Sleeping => "sleeping",
            Place::Waiting => "waiting",
        };
        ELispExp::proper_list(vec![
            symbol("name"),
            ELispExp::string(job.name),
            symbol("kind"),
            symbol("job"),
            symbol("class"),
            symbol(class),
            symbol("state"),
            symbol(state),
            symbol("steps"),
            ELispExp::number(job.steps as f64),
            symbol("total-ms"),
            millis(job.total),
            symbol("max-ms"),
            millis(job.longest),
        ])
    });
    let threads = ctx.work().threads().into_iter().map(|thread| {
        ELispExp::proper_list(vec![
            symbol("name"),
            ELispExp::string(thread.name),
            symbol("kind"),
            symbol("thread"),
            symbol("state"),
            symbol(if thread.stopping {
                "stopping"
            } else {
                "running"
            }),
            symbol("running-ms"),
            millis(thread.running_for),
        ])
    });
    Ok(ELispExp::proper_list(jobs.chain(threads).collect()))
});

/// Register this module's primitives: work in the background, and the
/// scheduler that runs it.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Plain functions rather than commands: a worker is defined by a module
    // at load time, not run from M-x.
    into.function("define-worker", define_worker, DEFINE_WORKER_DOC);
    into.function("stop-worker", stop_worker, STOP_WORKER_DOC);
    into.function("running-workers", running_workers, RUNNING_WORKERS_DOC);
    // Work that must not happen on the thread that draws. See `lisp_job`,
    // which states the rule the whole editor follows for it.
    into.function("background-call", background_call, BACKGROUND_CALL_DOC);
    into.function("emit-event", emit_event, EMIT_EVENT_DOC);
    into.function("work-option", work_option, WORK_OPTION_DOC);
    into.function("set-work-option", set_work_option, SET_WORK_OPTION_DOC);
    into.function("work-options", work_options, WORK_OPTIONS_DOC);
    into.function("work-list", work_list, WORK_LIST_DOC);
}
