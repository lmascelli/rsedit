//! Starting, stopping and listing work in the background, and tuning the
//! scheduler that runs it.
//!
//! The job these start is in [`super::lisp_job`], and the scheduler is the
//! editor's WorkManager, in [`crate::work`]; this is the door Lisp comes
//! through.
use super::*;
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

pub const DEFINE_WORKER_DOC: &str = "(define-worker NAME FIBER &optional INTERVAL): Run FIBER in \
         the background, a step at a time, until it finishes.\n\n\
         FIBER is a `fiber' -- see `yield'. Each step resumes it once, so a worker is written as \
         a loop that says where it is willing to be put down, and what it yields says when it \
         wants its next step:\n\n\
         \x20 (define-worker 'symbol-index\n\
         \x20   (fiber (while t\n\
         \x20            (if (index-stale-p)\n\
         \x20                (progn (index-one-chunk) (yield 'continue))\n\
         \x20              (yield '(wait buffer-changed))))))\n\n\
         \x20 (yield)                    wait INTERVAL milliseconds, 40 if not given\n\
         \x20 (yield 'continue)          go on as soon as its turn comes round again\n\
         \x20 (yield '(sleep MS))        wait MS milliseconds\n\
         \x20 (yield '(wait TOPIC...))   wait for one of the TOPICs: see `emit-event'\n\
         \x20 (yield '(wait-at-most MS TOPIC...))\n\
         \x20                            for one of the TOPICs, or MS milliseconds at most\n\n\
         Any other value waits INTERVAL, as `(yield)' does, and a list that starts like one of \
         these and is not one stops the worker with a message saying so. The topics of a wait \
         are the worker's until a later wait names others, so `(wait)' keeps them. The editor \
         emits `buffer-changed' when the text of a buffer changes and `syntax-changed' when a \
         grammar or a syntax table does.\n\n\
         Defining a worker under a name that already has one *replaces* it: the old one stops \
         before its next step. That is what makes re-evaluating a module safe -- without it, \
         loading a file twice would leave two copies of its worker running against each other. \
         Names are shared with `background-call' and with the editor's own work, which \
         `work-list' shows.\n\n\
         A step gets its own execution budget, `worker-fuel', which is separate from the one \
         commands run under: nothing the user does can cut a step short. A step that runs past \
         its own budget is stopped, and the worker with it, because a worker that never \
         reaches a `yield' holds the thread syntax colouring runs on.\n\n\
         Returns NAME.";

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

pub const STOP_WORKER_DOC: &str = "(stop-worker NAME): Stop the work in the background called \
         NAME: a worker, a job started with `background-call', or any of the editor's own work \
         that `work-list' shows. Returns t if there was one, nil otherwise.\n\n\
         A job stops at the end of the step it is taking, or before its next one. Nothing waits \
         for that: waiting would block the thread you are typing on until the job got round to \
         it, which is the arrangement this whole mechanism exists to avoid.\n\n\
         A job stopped this way does not call its ON-DONE, and the callbacks it had queued are \
         dropped. It was not done -- it was stopped -- and whoever stopped it is on the thread \
         that would have been told.\n\n\
         The editor's own work stops just the same, and does not come back until the editor is \
         started again: stopping `highlighter' stops syntax colouring.";

primitive!(stop_worker, args, _env, ctx, {
    let name = required_name(args.first())?;
    // Silenced at once, in the registry, rather than when the job hears of
    // it: whatever it queued is dropped from this command on. A job that
    // finished before it could be stopped is still owed its ON-DONE.
    Ok(ELispExp::boolean(ctx.work().cancel_named(&name)))
});

pub const RUNNING_WORKERS_DOC: &str = "(running-workers): The names of the workers and \
         background jobs started from Lisp that are still running, as a sorted list of \
         strings.\n\n\
         The part of `work-list' that Lisp started; `work-list' shows the editor's own work as \
         well. One list for workers and jobs because there is one namespace: `define-worker' \
         and `background-call' name what they start in the same place, so that a name means \
         one thing and `stop-worker' stops whichever it is.";

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

/// The name of the Lisp variable holding one step's execution budget.
pub const WORKER_FUEL: &str = "worker-fuel";

pub const BACKGROUND_CALL_DOC: &str = "(background-call NAME JOB &optional ON-PROGRESS ON-DONE \
         INTERVAL): Do JOB in the background under the name NAME, and return NAME at once \
         without waiting for it.\n\n\
         This is how anything that may take longer than a frame is started. JOB is either a \
         function of no arguments -- called once, in the background -- or a `fiber', which is \
         resumed a step at a time and is the shape to use when the work can be done in \
         pieces:\n\n\
         \x20 (background-call 'manpage-index\n\
         \x20                  (fiber (let ((left manpage-path))\n\
         \x20                           (while left\n\
         \x20                             (manpage--index-one (car left))\n\
         \x20                             (setq left (cdr left))\n\
         \x20                             (yield 'continue))))\n\
         \x20                  'manpage--index-progress\n\
         \x20                  'manpage--index-done)\n\n\
         A `while' and not a `dolist': a `yield' is legal only where the evaluator can record \
         a way back to it, which is a body statement or a form of a `while'. A `dolist' body \
         is not one, and a yield there is refused -- so a fiber walks its own list. What a \
         fiber yields says when it wants its next step, as for `define-worker': the \
         `(yield 'continue)' above goes on as soon as its turn comes round, where `(yield)' \
         would wait INTERVAL first.\n\n\
         ON-PROGRESS is called with NAME after every step but the last, so each `yield' in a \
         fiber is a progress point and a plain function has none. ON-DONE is called once, with \
         NAME and t if the job reached its end or nil if it failed -- an error, or a step that \
         spent its allowance -- because a mode that is never told stays waiting for ever. It \
         is not called for a job stopped with `stop-worker' or replaced by another of the same \
         name: whoever did that knows already.\n\n\
         Both run on the thread that runs commands, *between* commands, and so may do anything \
         a command may do. The job itself may not: it runs while you are typing, and the editor \
         it can see is in whatever state the command in progress has left it in. So a job \
         computes and stores; a callback is where the mode is told, and the mode is what keeps \
         itself consistent with what it is told.\n\n\
         Neither callback is given what the job produced. They are given the name it ran under, \
         and look the rest up -- because between the step that produced something and the \
         callback that reports it the user may have killed the buffer, changed the variable, or \
         started the same job again.\n\n\
         Starting a job under a name that already has one *replaces* it: the old job stops \
         before its next step, and whatever callbacks it had already queued are dropped rather \
         than delivered against the new job's state. That is what makes re-evaluating a module, \
         or pressing the same key twice, safe. `stop-worker' stops a job and `running-workers' \
         lists one -- jobs and workers share a namespace, and a name cannot be both.\n\n\
         INTERVAL is how many milliseconds a plain `(yield)' waits, 40 if not given. It means \
         nothing for a job that is a plain function, which has one step.\n\n\
         Each step gets `worker-fuel' to spend, separately from the budget commands run under. \
         A step that runs past it is stopped and the job fails, because a step that never ends \
         holds the thread syntax colouring runs on.";

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

pub const EMIT_EVENT_DOC: &str = "(emit-event TOPIC): Say that something TOPIC stands for has \
         changed, waking every worker or background job waiting for it. TOPIC is a symbol or a \
         string. Returns nil.\n\n\
         A topic carries nothing: it says \"go and look\", and a job woken by it looks at \
         whatever it was waiting to see change. So a mode tells its worker there is work \
         without handing it any, and a burst of emissions is one wake-up:\n\n\
         \x20 (emit-event 'my-mode-refresh)\n\
         \x20 (define-worker 'my-mode-indexer\n\
         \x20   (fiber (while t\n\
         \x20            (my-mode-reindex)\n\
         \x20            (yield '(wait my-mode-refresh buffer-changed)))))\n\n\
         A waiting job takes its step `event-delay' milliseconds after the first emission it \
         has not seen: see `work-option'. The editor emits `buffer-changed' and \
         `syntax-changed' itself.";

primitive!(emit_event, args, _env, ctx, {
    exact_arity(args, 1)?;
    let topic = required_name(args.first())?;
    ctx.work().topic(&topic).emit();
    Ok(ELispExp::nil())
});

pub const WORK_OPTION_DOC: &str = "(work-option NAME): The value of NAME, an option of the \
         scheduler that runs the background work. Signals an error for a name that is not one; \
         `work-options' lists them with their values.\n\n\
         \x20 high-steps      colouring steps in a row before any other job gets one: 4\n\
         \x20 event-delay     milliseconds from an emission to the step it wakes: 20\n\
         \x20 long-step       a step longer than this many milliseconds is logged: 40\n\
         \x20 late-high       colouring kept waiting longer than this, in ms, is logged: 40\n\
         \x20 late-normal     any other job kept waiting longer than this is logged: 200\n\
         \x20 stuck-step      a thread slower to stop than this, in ms, is logged: 1000\n\
         \x20 busy-share      a second busier than this many percent is logged: 50\n\
         \x20 summary-every   milliseconds between two summaries of one report: 10000\n\
         \x20 watchdog-every  milliseconds between two looks of the watchdog: 250\n\n\
         `set-work-option' changes one.";

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

pub const SET_WORK_OPTION_DOC: &str = "(set-work-option NAME VALUE): Set NAME, an option of \
         the scheduler that runs the background work, to VALUE, a whole number. It counts from \
         the scheduler's next step. Returns VALUE.\n\n\
         Signals an error, and changes nothing, for a name that is not an option or a value \
         outside what the option accepts; the message gives the range. See `work-option' for \
         what each one does. It works from init.lisp as well:\n\n\
         \x20 (set-work-option 'high-steps 6)";

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

pub const WORK_OPTIONS_DOC: &str = "(work-options): Every option of the scheduler that runs \
         the background work, with its value, as a property list: (high-steps 4 event-delay 20 \
         ...). See `work-option' for what each one does.";

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

pub const WORK_LIST_DOC: &str = "(work-list): Everything running in the background -- the \
         editor's own work as well as Lisp's -- as a list of property lists, one each:\n\n\
         \x20 ((name \"highlighter\" kind job class high state waiting\n\
         \x20   steps 1204 total-ms 412 max-ms 38) ...)\n\n\
         A job has a `class' -- high for what the user is waiting to see, colouring and the \
         prescanner, normal for the rest -- and a `state': ready, running, sleeping or \
         waiting. `steps' is how many steps it has taken, `total-ms' how long they took \
         together and `max-ms' how long the longest did. The figures are brought up to date \
         whenever the scheduler runs out of work, and every tenth of a second while it is \
         busy.\n\n\
         A thread of its own has `kind thread', a `state' of running or stopping, and \
         `running-ms', how long it has been running.\n\n\
         `running-workers' is the part of this list that Lisp started, and `stop-worker' stops \
         anything in it by name.";

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
