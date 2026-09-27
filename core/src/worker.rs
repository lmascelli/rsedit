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
//! and take colouring, prescanning and shell output down with it. So a turn
//! that runs past its own allowance ends, is reported once, and the worker is
//! retired -- a visible failure of one worker rather than an invisible failure
//! of everything.
use crate::{
    BufferTrait, ELispExp, EditorState,
    lisp::{Env, EvalError, LispContext, eval, set_remaining},
    task::ScheduledTask,
};
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
struct WorkerTurn;

impl WorkerTurn {
    fn begin() -> Self {
        IN_WORKER.set(true);
        WorkerTurn
    }
}

impl Drop for WorkerTurn {
    fn drop(&mut self) {
        IN_WORKER.set(false);
    }
}

/// One named Lisp worker, as the scheduler holds it.
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
