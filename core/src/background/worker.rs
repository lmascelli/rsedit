//! Background workers written in Lisp.
//!
//! # What a worker is
//!
//! A fiber that the editor resumes on a timer, on the thread the syntax
//! highlighter and the prescanner already share. It is written as an ordinary
//! loop that says `(yield)` where it is willing to be put down:
//!
//! ```lisp
//! (define-worker 'indexer
//!   (fiber (while t
//!            (if (index--stale-p)
//!                (index--one-chunk)
//!              (yield)))))
//! ```
//!
//! and the editor does the rest. Nothing here knows what a worker is *for*,
//! which is the point: the same mechanism carries an index, a language server
//! client, or anything else that has to make progress while the user types.
//!
//! # Why a shared thread rather than one each
//!
//! Because a worker's usual state is waiting, and a thread that is waiting is
//! a thread doing nothing. The obvious counter-argument -- that a slow worker
//! would stall the others -- is answered by the turn, not by a thread: a
//! worker that does not come back promptly is a bug in that worker, and the
//! allowance below is what stops that bug from being everyone's problem.
//!
//! # The allowance, and what it does not do
//!
//! Each turn is given its own fuel, set on the worker thread and nowhere else.
//! The editor's own budget is thread-local, so a command running on the main
//! thread can never cut a worker's turn short, and a worker can never spend a
//! command's budget: that is the guarantee a worker is written against.
//!
//! What the allowance is for is the other direction. A `(while t)` with no
//! `(yield)` in it would hold this thread for as long as the process lives,
//! and take colouring, prescanning, file watching and every other worker down
//! with it. So a turn that runs past its own allowance ends, is reported
//! once, and the worker is retired -- a visible failure of one worker rather
//! than an invisible failure of everything.
//!
//! Work that *cannot* be bounded that way does not come here at all: it gets
//! a thread of its own. See [`crate::background`], which states the rule.
//!
//! # The rule, for everything that runs in the background
//!
//! Four things had grown up here separately -- a worker woken on a timer, a
//! task posted once, a search that wrote into a buffer from the worker
//! thread, a shell command appending its output -- and each had invented its
//! own answer to the same three questions. These are the answers, and
//! [`BackgroundJob`] is where the ones that can be mechanised live.
//!
//! 1. **Anything that may take longer than a frame is a background job**, and
//!    a job is started by one call that names what to do and what to do when
//!    it is done. Not a function that starts something and leaves the caller
//!    to find out.
//!
//! 2. **A job never draws.** It changes state; the renderer notices. There is
//!    no other order that works, because the thread that draws is not the
//!    thread the job is on.
//!
//! 3. **A job never assumes the editor stood still.** The buffer it was
//!    started for may be gone, the variable may have been set again, the
//!    version it read may be two edits old. So it re-checks before it stores,
//!    and what it cannot re-check it does not keep.
//!
//! 4. **Progress is a signal, not a redraw.** A job says "there is more now";
//!    what more means is the mode's business, and how it reaches the screen is
//!    the renderer's.
//!
//! 5. **The mode keeps itself consistent.** Which is only a fair thing to ask
//!    if the signal arrives somewhere a mode can respond safely -- so the
//!    callbacks run on the thread that runs commands, between commands. See
//!    [`BackgroundJob`] for why that is not a detail.
//!
//! 6. **Errors are logged, never raised.** The command that started the job
//!    returned long ago; there is nobody to raise to. A job that died still
//!    reports that it is done, because a mode that is never told stays
//!    waiting for ever.
//!
//! 7. **A job is superseded, not duplicated.** Jobs are named, and starting
//!    one under a name that has one retires the old -- which is what makes
//!    re-evaluating a module, or pressing the same key twice, safe.
//!
//! What is *not* covered here is anything that has to run on a schedule for
//! the life of the session: that is a worker, and it is the other half of
//! this file. The two share a namespace and a retirement mechanism on
//! purpose, so that `stop-worker` and `running-workers` mean one thing.
use crate::{
    BufferTrait, ELispExp, EditorState,
    background::ScheduledTask,
};
use risp::{Env, EvalError, LispContext, call_callable, eval, set_remaining};
use std::cell::Cell;
use std::sync::Arc;

/// How much fuel one worker turn gets when nothing sets `worker-fuel`.
///
/// Sized against the turn interval rather than against what a worker might
/// want: at roughly 100ns per step in release, this is about twenty
/// milliseconds, or half a turn. A worker that needs more than that in one go
/// is one that should be yielding more often, and the point of the number is
/// to make that obvious rather than to accommodate it.
pub const DEFAULT_WORKER_FUEL: u32 = 200_000;

thread_local! {
    /// Whether this thread is part-way through a worker's turn.
    ///
    /// # What it is for
    ///
    /// Some things an editor can do only make sense on the thread the user is
    /// typing on. Opening a prompt is the clearest: a minibuffer put up by a
    /// worker is a window nobody is typing into, over a command nobody
    /// started, and the keystroke that answers it goes to whatever the user
    /// was actually doing.
    ///
    /// A worker *can* call `minibuffer-read` -- nothing in Rust stops a Lisp
    /// program from naming a function -- so the refusal has to be somewhere,
    /// and it has to be somewhere that knows which thread it is on. This is
    /// that.
    static IN_WORKER: Cell<bool> = const { Cell::new(false) };
}

/// Whether the caller is running inside a worker's turn.
///
/// For the handful of primitives that are meaningless off the thread the user
/// is typing on. A primitive that merely *reads* editor state has no business
/// asking: it is as correct here as anywhere, and asking would make workers a
/// second class of Lisp rather than the same Lisp somewhere else.
pub fn in_worker() -> bool {
    IN_WORKER.get()
}

/// Sets [`in_worker`] for as long as it is alive.
///
/// RAII rather than a pair of calls so that a turn which ends by propagating
/// an error cannot leave the flag set -- which would make every later
/// primitive on this thread believe it was inside a worker.
///
/// # Who holds one
///
/// [`crate::background::BackgroundScheduler`], around every task it drives and
/// every thread it starts; and, inside that, a worker or a background job
/// around its own turn.
///
/// # Why it nests
///
/// It is held in two places on purpose, and they overlap. The scheduler holds
/// one around everything it drives, which is what makes the flag a fact about
/// the *thread* -- covering the file watcher and the auto-saver, which run
/// Lisp and never set it, and any task type added later, which cannot forget
/// something it does not do. A worker and a background job hold one of their
/// own as well, so that a turn driven by hand -- which is how every test in
/// the tree drives one, to avoid waiting on a timer -- is the same turn the
/// scheduler would have driven.
///
/// So this restores what it found rather than clearing: an inner turn ending
/// inside an outer one must not announce that the thread has become the
/// command thread.
pub(crate) struct WorkerTurn(bool);

impl WorkerTurn {
    pub(crate) fn begin() -> Self {
        WorkerTurn(IN_WORKER.replace(true))
    }
}

impl Drop for WorkerTurn {
    fn drop(&mut self) {
        IN_WORKER.set(self.0);
    }
}

/// One named Lisp worker, as the scheduler holds it.
/// A job the editor runs in the background and is told about when it ends.
///
/// # The shape, and why it is one call
///
/// Everything an editor does in the background has the same three parts: work
/// that must not happen on the thread that draws, something to do with what it
/// produced, and a way of not running two of it at once. Before this they were
/// three separate arrangements -- a task that changed a variable and left
/// whoever cared to notice, a worker that had to invent its own way of saying
/// it was done, a search that drew into a buffer from the worker thread. So
/// the parts are named once, here:
///
/// ```lisp
/// (background-call 'manpage-index
///                  (fiber (let ((left manpage-path))
///                           (while left
///                             (manpage--index-one (car left))
///                             (setq left (cdr left))
///                             (yield))))
///                  'manpage--index-progress
///                  'manpage--index-done)
/// ```
///
/// A `while` rather than a `dolist`: a `(yield)` is legal only where the
/// evaluator can record a way back to it, and a `dolist` body is not such a
/// place.
///
/// JOB runs on the worker thread. ON-PROGRESS and ON-DONE run on the thread
/// that runs commands, queued and drained between them.
///
/// # Why the callbacks are not run where the job is
///
/// Because the alternative is a rule nobody can keep. A job runs while a
/// command is running: while `self-insert-command` is half-way through
/// inserting, while a mode's hook is walking a list it is also editing. A
/// callback run on the worker thread is therefore code that may observe the
/// editor in any state at all, including states that exist only inside one
/// command -- and no amount of care in the callback can fix that, because it
/// is not the callback's fault.
///
/// Queued to the command thread, a callback runs *between* commands: the same
/// moment a keystroke would be handled, with the same locks free and the same
/// invariants holding. That is what makes "the mode keeps itself consistent"
/// a rule a mode can actually follow -- it is being asked to be consistent at
/// a point where consistency is defined.
///
/// The cost is that a callback is not immediate. It is at most one pass of
/// the loop away, which is the same delay a keystroke has.
///
/// # What the job may not assume
///
/// That the editor stood still. Between the turn that produced a result and
/// the callback that reports it, the user may have killed the buffer, changed
/// the variable, or started the same job again. So a job stores only what it
/// can re-check, and a callback is handed the name it was started under and
/// looks the rest up.
pub struct BackgroundJob<B: BufferTrait> {
    name: String,
    /// Which run of `name` this is. A newer run retires this one, and also
    /// drops whatever callbacks it had already queued -- they would otherwise
    /// tell a mode that the job it is now waiting for had finished.
    generation: u64,
    body: JobBody<B>,
    /// Called with the name after every turn but the last. Nil for a job with
    /// nothing to report until it is done, which is every job whose body is a
    /// plain function: there is nowhere in one for progress to be.
    on_progress: Option<ELispExp<B>>,
    /// Called with the name and whether the job reached its end. Nil when
    /// nobody is waiting.
    on_done: Option<ELispExp<B>>,
    env: Arc<Env<EditorState<B>>>,
    fuel: u32,
}

/// What a job is made of: something that finishes in one turn, or something
/// that says where it can be put down.
pub enum JobBody<B: BufferTrait> {
    /// A function of no arguments, called once.
    Once(ELispExp<B>),
    /// A fiber, resumed one turn at a time until it is done. Each `(yield)`
    /// is a progress point -- which is the whole of what a job has to do to
    /// report progress, since there is nothing else a yield could mean.
    Steps(ELispExp<B>),
}

/// Whether a job ran to its end.
enum Outcome {
    /// Its body returned, or its fiber finished.
    Finished,
    /// It raised, or spent its allowance. What it had done up to then stands;
    /// what it had not is not coming.
    Stopped,
}

impl<B: BufferTrait> BackgroundJob<B> {
    pub fn new(
        name: String,
        generation: u64,
        body: JobBody<B>,
        on_progress: Option<ELispExp<B>>,
        on_done: Option<ELispExp<B>>,
        env: Arc<Env<EditorState<B>>>,
        fuel: u32,
    ) -> Self {
        Self {
            name,
            generation,
            body,
            on_progress,
            on_done,
            env,
            fuel,
        }
    }

    /// Run one turn of the body. `None` while there is more to do.
    fn turn(&mut self, state: &EditorState<B>) -> Option<Outcome> {
        // The same allowance a worker turn gets, and for the same reason: a
        // runaway loop on this thread would spin for ever, where one on the
        // command thread is stopped by the budget the command runs under.
        set_remaining(self.fuel);
        // Held here as well as by the scheduler, so that a turn driven by
        // hand is the same turn the scheduler would have driven. Nesting is
        // what [`WorkerTurn`] restores rather than clears.
        let _turn = WorkerTurn::begin();

        let (result, finished) = match &self.body {
            JobBody::Once(function) => (
                call_callable(function, &[], self.env.clone(), state),
                // One turn is the whole of it either way.
                true,
            ),
            JobBody::Steps(fiber) => {
                let call = ELispExp::form(vec![ELispExp::symbol("resume".into()), fiber.clone()]);
                let result = eval(&call, self.env.clone(), state);
                // `None` would mean the job was given something that is not a
                // fiber, which `background-call` refuses -- so it cannot
                // happen, and if it somehow did, stopping is the safe answer.
                let done = fiber.fiber_is_done().unwrap_or(true);
                (result, done)
            }
        };

        match result {
            Ok(_) if finished => Some(Outcome::Finished),
            Ok(_) => None,
            Err(EvalError::OutOfFuel) => {
                state.log_diagnostic(&format!(
                    "[ERROR] background job {}: a turn ran past its allowance of {} and was \
                     stopped. A job has to reach a (yield) -- or return -- within one turn; \
                     raise `worker-fuel' if the work genuinely needs more.",
                    self.name, self.fuel
                ));
                Some(Outcome::Stopped)
            }
            Err(error) => {
                // Reported rather than raised: there is nobody to raise it to.
                // The command that asked for this returned long ago. A fiber
                // has already retired itself -- an error inside one ends it,
                // because the position it stopped at is gone.
                state.log_diagnostic(&format!("[ERROR] background job {}: {error:?}", self.name));
                Some(Outcome::Stopped)
            }
        }
    }
}

impl<B: BufferTrait> ScheduledTask<B> for BackgroundJob<B> {
    fn execute(&mut self, state: &EditorState<B>) -> bool {
        // Asked before the turn rather than after, so a job that has been
        // replaced does not do one more chunk of work on the strength of
        // having already been woken. Nothing is queued on this path: a
        // superseded job owes its caller nothing, because the caller is now
        // waiting on the job that replaced it.
        if !state.runtime(|runtime| runtime.worker_is_current(&self.name, self.generation)) {
            // The count is dropped here rather than left to the job that
            // superseded this one, which has a count of its own.
            state.finish_background_work();
            return false;
        }

        let Some(outcome) = self.turn(state) else {
            // More to do. The yield it just reached is the progress point.
            if let Some(on_progress) = &self.on_progress {
                state.owe_callback(
                    &self.name,
                    self.generation,
                    on_progress.clone(),
                    vec![ELispExp::string(self.name.clone())],
                );
            }
            return true;
        };

        // Queued *before* the retirement and before the count is dropped.
        // Either of those first would leave a window in which the editor said
        // nothing was running while a callback was still owed, and a renderer
        // that looked in that window would go back to sleep holding it.
        if let Some(on_done) = &self.on_done {
            state.owe_callback(
                &self.name,
                self.generation,
                on_done.clone(),
                vec![
                    ELispExp::string(self.name.clone()),
                    ELispExp::boolean(matches!(outcome, Outcome::Finished)),
                ],
            );
        }
        state.runtime_mut(|runtime| runtime.retire_worker(&self.name, self.generation));
        state.finish_background_work();
        false
    }
}

pub struct LispWorker<B: BufferTrait> {
    name: String,
    /// Which run of `name` this is. Compared against the editor's answer every
    /// turn, so that redefining or stopping a worker retires this one.
    generation: u64,
    /// The fiber, held as the `LispExp` so that `resume` is reached the same
    /// way Lisp reaches it -- there is one implementation of what resuming
    /// means, and it is not duplicated here.
    fiber: ELispExp<B>,
    /// The environment the `resume` call is made in.
    ///
    /// Carried rather than looked up because there is nowhere to look it up
    /// from: the scheduler holds an `EditorState` and no environment, and the
    /// two are deliberately separate -- the editor does not own the
    /// interpreter. `Env` crosses threads already; `(spawn ...)` has been
    /// sending one to a new thread since before this existed.
    env: Arc<Env<EditorState<B>>>,
    fuel: u32,
}

impl<B: BufferTrait> LispWorker<B> {
    pub fn new(
        name: String,
        generation: u64,
        fiber: ELispExp<B>,
        env: Arc<Env<EditorState<B>>>,
        fuel: u32,
    ) -> Self {
        Self {
            name,
            generation,
            fiber,
            env,
            fuel,
        }
    }
}

impl<B: BufferTrait> ScheduledTask<B> for LispWorker<B> {
    fn execute(&mut self, state: &EditorState<B>) -> bool {
        // Asked before the turn rather than after, so a worker that has been
        // replaced does not get one more turn on the strength of having
        // already been woken.
        if !state.runtime(|runtime| runtime.worker_is_current(&self.name, self.generation)) {
            return false;
        }

        // The allowance is set, not raised: this thread carries whatever the
        // last turn left, and the first turn of all carries the compile-time
        // default, which is fifty times too much.
        set_remaining(self.fuel);
        let _turn = WorkerTurn::begin();

        let call = ELispExp::form(vec![ELispExp::symbol("resume".into()), self.fiber.clone()]);
        match eval(&call, self.env.clone(), state) {
            Ok(_) => {}
            Err(EvalError::OutOfFuel) => {
                state.log_diagnostic(&format!(
                    "[ERROR] worker {}: a turn ran past its allowance of {} and was stopped. \
                     A worker has to reach a (yield) within one turn; raise `worker-fuel' if \
                     the work genuinely needs more.",
                    self.name, self.fuel
                ));
                state.runtime_mut(|runtime| runtime.retire_worker(&self.name, self.generation));
                return false;
            }
            Err(error) => {
                // The fiber has already retired itself -- an error inside one
                // ends it, because the position it stopped at is gone. So
                // this reports and stops rather than trying again with a
                // fiber that would answer nil forever.
                state.log_diagnostic(&format!("[ERROR] worker {}: {error:?}", self.name));
                state.runtime_mut(|runtime| runtime.retire_worker(&self.name, self.generation));
                return false;
            }
        }

        // `None` would mean the worker was given something that is not a
        // fiber, which `define-worker` refuses -- so it cannot happen, and if
        // it somehow did, stopping is the safe answer.
        if self.fiber.fiber_is_done().unwrap_or(true) {
            state.runtime_mut(|runtime| runtime.retire_worker(&self.name, self.generation));
            return false;
        }
        true
    }
}
