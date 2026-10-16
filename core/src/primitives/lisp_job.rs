//! The job a Lisp program starts: a fiber resumed a step at a time, or a
//! function called once, on the WorkManager.
//!
//! `define-worker` and `background-call`, in [`super::workers`], are the two
//! doors; this is what both of them start. One kind of job, because the two
//! differ only in what they say back: a worker says nothing, a background call
//! has callbacks.
//!
//! # A step, and what a yield asks for
//!
//! A step resumes the fiber once, so a job is written as a loop that says,
//! with `(yield)`, where it is willing to be put down. What it yields says
//! when it wants the next step:
//!
//! ```lisp
//! (yield)                                     ; the default wait: INTERVAL, 40 ms if not given
//! (yield 'continue)                           ; as soon as its turn comes again
//! (yield '(sleep 250))                        ; in 250 ms
//! (yield '(wait buffer-changed))              ; at the next emission of a topic named
//! (yield '(wait-at-most 500 buffer-changed))  ; at the next emission, or in 500 ms
//! (yield 42)                                  ; anything else: the default wait
//! ```
//!
//! The topics of a wait become the job's own and stay so until a later wait
//! names others: `(wait)` and `(wait-at-most 500)` keep them. The last line
//! is for the jobs written before the table, which yield their progress as a
//! value: they go on as they did. A list that starts like one of the shapes
//! and is not one -- `(sleep "soon")` -- is a mistake rather than a value,
//! and fails the job with a message saying what the shape is, instead of
//! quietly waking it every 40 ms.
//!
//! # The rule, for everything a Lisp program runs in the background
//!
//! 1. **Anything that may take longer than a frame is a background job**, and
//!    a job is started by one call that names what to do and what to do when
//!    it is done. Not a function that starts something and leaves the caller
//!    to find out.
//!
//! 2. **A job never draws.** It changes state; the renderer notices -- the
//!    end of every step tells it to look.
//!
//! 3. **A job never assumes the editor stood still.** The buffer it was
//!    started for may be gone, the variable may have been set again, the
//!    version it read may be two edits old. So it re-checks before it stores,
//!    and what it cannot re-check it does not keep.
//!
//! 4. **Progress is a signal, not a redraw.** A job says "there is more now";
//!    what more means is the mode's business.
//!
//! 5. **The mode keeps itself consistent.** Which is only a fair thing to ask
//!    if the signal arrives somewhere a mode can respond safely -- so the
//!    callbacks run on the thread that runs commands, between commands. See
//!    [`crate::editor`]'s owed callbacks for why that is not a detail.
//!
//! 6. **Errors are logged, never raised.** The command that started the job
//!    returned long ago; there is nobody to raise to. A job that failed still
//!    says that it is done, because a mode that is never told stays waiting
//!    for ever.
//!
//! 7. **A job is superseded, not duplicated.** Jobs are named, and starting
//!    one under a name that has one stops the old -- which is what makes
//!    re-evaluating a module, or pressing the same key twice, safe. The old
//!    one's queued callbacks are dropped, and it does not call its ON-DONE:
//!    whoever replaced it knows already.
//!
//! # The allowance, and what it does not do
//!
//! Each step is given its own fuel, `worker-fuel`, set on the thread that
//! takes the step. The budget is thread-local, so a command running on the
//! command thread can never cut a step short, and a step can never spend a
//! command's budget.
//!
//! What the allowance is for is the other direction. A `(while t)` with no
//! `(yield)` in it would hold the executor for as long as the process lives,
//! and stop colouring with it. So a step that runs past its allowance ends,
//! the job leaves as failed, and the log says why -- a visible failure of one
//! job rather than an invisible failure of everything. Work that cannot be
//! bounded that way does not come here at all: it gets a thread of its own.
use crate::{
    BufferTrait, ELispExp, EditorState,
    editor::WorkerTurn,
    work::{Exit, Job, JobError, Spec, Step, StepContext, WorkId},
};
use risp::{Env, EvalError, FuelMeter, call_callable, eval, set_remaining};
use std::sync::Arc;
use std::time::Duration;

/// How much fuel one step gets when nothing sets `worker-fuel`.
///
/// Sized against the default wait rather than against what a job might want:
/// at roughly 100ns per step of fuel in release, this is about twenty
/// milliseconds, or half a turn. A job that needs more than that in one go is
/// one that should be yielding more often, and the point of the number is to
/// make that obvious rather than to accommodate it.
pub const DEFAULT_WORKER_FUEL: u32 = 200_000;

/// What a job does: something that finishes in one step, or something that
/// says where it can be put down.
pub(crate) enum Body<B: BufferTrait> {
    /// A function of no arguments, called once.
    Once(ELispExp<B>),
    /// A fiber, resumed one step at a time until it is done. Each `(yield)`
    /// is a progress point -- which is the whole of what a job has to do to
    /// report progress, since there is nothing else a yield could mean.
    Fiber(ELispExp<B>),
}

/// A job started from Lisp. See the module documentation.
pub(crate) struct LispJob<B: BufferTrait> {
    name: String,
    body: Body<B>,
    /// What a plain `(yield)` waits.
    pause: Duration,
    /// Called with the name after every step but the last.
    on_progress: Option<ELispExp<B>>,
    /// Called with the name and whether the job reached its end.
    on_done: Option<ELispExp<B>>,
    /// The environment the body runs in.
    ///
    /// Carried rather than looked up because there is nowhere to look it up
    /// from: a step is handed the editor's state and no environment, and the
    /// two are deliberately separate -- the editor does not own the
    /// interpreter. `Env` crosses threads already; `(spawn ...)` has been
    /// sending one to a new thread since before this existed.
    env: Arc<Env<EditorState<B>>>,
    fuel: u32,
    /// Its number, learnt at its first step: what its callbacks are owed
    /// under. `None` for a job that never took one, which owes nothing.
    id: Option<WorkId>,
}

/// What a yielded value asks for. See the module documentation.
#[derive(Debug, PartialEq)]
enum Ask {
    Pause,
    Continue,
    Sleep(Duration),
    Wait(Vec<String>),
    WaitAtMost(Duration, Vec<String>),
}

impl<B: BufferTrait> LispJob<B> {
    /// What `define-worker` starts: a fiber, and nobody to tell.
    pub(crate) fn worker(
        name: String,
        fiber: ELispExp<B>,
        pause: Duration,
        env: Arc<Env<EditorState<B>>>,
        fuel: u32,
    ) -> Self {
        Self::call(name, Body::Fiber(fiber), pause, None, None, env, fuel)
    }

    /// What `background-call` starts.
    pub(crate) fn call(
        name: String,
        body: Body<B>,
        pause: Duration,
        on_progress: Option<ELispExp<B>>,
        on_done: Option<ELispExp<B>>,
        env: Arc<Env<EditorState<B>>>,
        fuel: u32,
    ) -> Self {
        Self {
            name,
            body,
            pause,
            on_progress,
            on_done,
            env,
            fuel,
            id: None,
        }
    }

    /// How it is submitted: normal class, started at once, measured in fuel.
    ///
    /// Normal because a Lisp job is never what the user is waiting to see:
    /// colouring and the prescanner go first, and a Lisp job gets a step
    /// after every few of theirs. No topics: a job has the ones its waits
    /// name.
    pub(crate) fn spec(&self) -> Spec {
        Spec::new(&self.name).unit("fuel")
    }

    /// Resume the body once. The value it stopped with, and whether it has
    /// finished.
    fn resume(
        &self,
        state: &EditorState<B>,
    ) -> (Result<ELispExp<B>, EvalError<EditorState<B>>>, bool) {
        match &self.body {
            // One step is the whole of it, either way.
            Body::Once(function) => (call_callable(function, &[], self.env.clone(), state), true),
            Body::Fiber(fiber) => {
                let call = ELispExp::form(vec![ELispExp::symbol("resume".into()), fiber.clone()]);
                let result = eval(&call, self.env.clone(), state);
                // `None` would mean the job was given something that is not a
                // fiber, which both doors refuse -- so it cannot happen, and
                // if it somehow did, stopping is the safe answer.
                (result, fiber.fiber_is_done().unwrap_or(true))
            }
        }
    }
}

impl<B: BufferTrait> Job<EditorState<B>> for LispJob<B> {
    fn step(&mut self, cx: &mut StepContext<'_, EditorState<B>>) -> Result<Step, JobError> {
        let id = *self.id.get_or_insert(cx.id);
        let state = cx.state;

        // The allowance is set, not raised: the thread carries whatever the
        // last step, or its own work, left. And put back afterwards -- by a
        // guard, so a step that panics puts it back too -- so that a step
        // taken by hand, on a test's thread, leaves that thread's fuel as it
        // found it.
        let restore = RestoreFuel(fuel_left());
        set_remaining(self.fuel);
        let (result, finished) = {
            let _turn = WorkerTurn::begin();
            self.resume(state)
        };
        let left = fuel_left();
        drop(restore);
        // Fuel is what a Lisp job's work is measured in: steps of the
        // evaluator, the same on every machine.
        cx.worked(u64::from(self.fuel.saturating_sub(left)), "");

        let value = match result {
            Ok(_) if finished => return Ok(Step::Done),
            Ok(value) => value,
            Err(EvalError::OutOfFuel) => {
                return Err(JobError(format!(
                    "a step ran past its allowance of {} fuel and was stopped. A job has to \
                     reach a (yield) -- or return -- within one step; raise `worker-fuel' if \
                     the work genuinely needs more.",
                    self.fuel
                )));
            }
            // A fiber has already ended itself: an error inside one does,
            // because the position it stopped at is gone.
            Err(error) => return Err(JobError(format!("{error:?}"))),
        };
        let step = match ask(&value).map_err(JobError)? {
            Ask::Pause => Step::Sleep(self.pause),
            Ask::Continue => Step::Continue,
            Ask::Sleep(span) => Step::Sleep(span),
            Ask::Wait(topics) => {
                if !topics.is_empty() {
                    cx.subscribe(topics);
                }
                Step::Wait
            }
            Ask::WaitAtMost(span, topics) => {
                if !topics.is_empty() {
                    cx.subscribe(topics);
                }
                Step::WaitAtMost(span)
            }
        };
        // More to do. The yield it just reached is the progress point.
        if let Some(on_progress) = &self.on_progress {
            state.owe_callback(
                &self.name,
                id,
                on_progress.clone(),
                vec![ELispExp::string(self.name.clone())],
            );
        }
        Ok(step)
    }

    fn exit(&mut self, state: &EditorState<B>, why: Exit) {
        let Some(id) = self.id else {
            // It never took a step, so it never queued anything, and it can
            // only be leaving because somebody stopped it.
            return;
        };
        match why {
            // Told either way, because a mode that is never told stays
            // waiting for ever: t when it reached its end, nil when it failed.
            Exit::Finished | Exit::Failed | Exit::Panicked => {
                if let Some(on_done) = &self.on_done {
                    state.owe_callback(
                        &self.name,
                        id,
                        on_done.clone(),
                        vec![
                            ELispExp::string(self.name.clone()),
                            ELispExp::boolean(why == Exit::Finished),
                        ],
                    );
                }
            }
            // Not told: whoever stopped it, or replaced it, or closed the
            // editor, knows already. Whatever it had queued is dropped too,
            // since the registry no longer counts it the latest under its
            // name.
            Exit::Cancelled | Exit::Shutdown => {}
        }
    }
}

/// Puts the calling thread's fuel back to what it holds when it is dropped.
struct RestoreFuel(u32);

impl Drop for RestoreFuel {
    fn drop(&mut self) {
        set_remaining(self.0);
    }
}

/// How much fuel the calling thread has left.
///
/// Read by spending it, which is the only way risp's API allows: a meter's
/// `consume` takes an amount when there is that much left and nothing
/// otherwise, so taking each power of two that still fits, largest first,
/// adds up to exactly what was there. The caller puts it back.
pub(crate) fn fuel_left() -> u32 {
    let meter = FuelMeter::new(0);
    (0..u32::BITS)
        .rev()
        .map(|bit| 1u32 << bit)
        .filter(|&amount| meter.consume(amount).is_ok())
        .sum()
}

/// What VALUE, yielded by a fiber, asks for. An error for a list that starts
/// like a shape of the table and is not one.
fn ask<B: BufferTrait>(value: &ELispExp<B>) -> Result<Ask, String> {
    let ELispExp::Cons(_) = value else {
        return Ok(match value {
            ELispExp::Symbol(name) if name.as_str() == "continue" => Ask::Continue,
            _ => Ask::Pause,
        });
    };
    let (items, tail) = value.split_list();
    let shape = match items.first() {
        Some(ELispExp::Symbol(head)) => head.as_str(),
        _ => return Ok(Ask::Pause),
    };
    let expected = match shape {
        "sleep" => "(sleep MS)",
        "wait" => "(wait TOPIC...)",
        "wait-at-most" => "(wait-at-most MS TOPIC...)",
        _ => return Ok(Ask::Pause),
    };
    let wrong = || {
        format!(
            "yielded {value:?}, which is not {expected}: MS is a number of milliseconds from 0 \
             up, and each TOPIC a symbol or a string"
        )
    };
    if !tail.is_nil() {
        return Err(wrong());
    }
    let rest = &items[1..];
    let topics = |names: &[ELispExp<B>]| -> Option<Vec<String>> {
        names
            .iter()
            .map(|name| match name {
                ELispExp::Symbol(name) | ELispExp::String(name) if !name.is_empty() => {
                    Some(name.to_string())
                }
                _ => None,
            })
            .collect()
    };
    match (shape, rest) {
        ("sleep", [ms]) => millis(ms).map(Ask::Sleep).ok_or_else(wrong),
        ("wait", names) => topics(names).map(Ask::Wait).ok_or_else(wrong),
        ("wait-at-most", [ms, names @ ..]) => match (millis(ms), topics(names)) {
            (Some(span), Some(names)) => Ok(Ask::WaitAtMost(span, names)),
            _ => Err(wrong()),
        },
        _ => Err(wrong()),
    }
}

/// MS, a number of milliseconds from 0 up, as a duration. `None` for anything
/// else. A span too long to hold is held as the longest there is, which the
/// scheduler then caps at a year.
fn millis<B: BufferTrait>(ms: &ELispExp<B>) -> Option<Duration> {
    match ms {
        ELispExp::Number(ms) if ms.is_finite() && *ms >= 0.0 => {
            Some(Duration::try_from_secs_f64(ms / 1000.0).unwrap_or(Duration::MAX))
        }
        _ => None,
    }
}
