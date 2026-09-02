// ========================================================================== //
//                 +------------------------------------------+
//                 |  Context that can embed the interpreter  |
//                 +------------------------------------------+
// ========================================================================== //

use super::EvalError;

pub trait LispContext: Clone + PartialEq + std::fmt::Debug + Send + Sync + 'static {
    /// Consumes a given amount of execution ticks.
    /// Returns `Err(EvalError::OutOfFuel)` if the host-defined budget is exhausted.
    fn consume_fuel(&self, amount: u32) -> Result<(), EvalError<Self>> {
        Ok(())
    }

    /// Allows the VM to bubble up non-fatal diagnostic logs, trace statements,
    /// or debugging notices to the host without knowing how the host presents them.
    fn log_diagnostic(&self, msg: &str) {}

    /// Called by the evaluator when it begins evaluating on a **newly created
    /// thread** -- currently only the `(spawn ...)` special form.
    ///
    /// A host that meters execution needs this because metering state is
    /// naturally *per-thread*: a budget belongs to one line of execution, and
    /// nothing about a parent thread's remaining allowance is meaningful to a
    /// child that runs concurrently with it. Since the evaluator is the only
    /// thing that knows a thread was just created, only the evaluator can tell
    /// the host to arm it.
    ///
    /// Unlike the scope-based entry points a host drives itself, this takes no
    /// guard and needs no matching "end" call: a fresh thread has no enclosing
    /// scope to nest inside or unwind back to, and its state dies with it.
    ///
    /// Default: a no-op, so hosts that do not meter pay nothing.
    fn begin_thread_evaluation(&self) {}

    /// Called by the evaluator right before running the body of a function
    /// call -- a named function, a primitive, or an inline lambda
    /// application -- with a short description of the frame (typically the
    /// function's name, or "<lambda>" for an anonymous one). A host that
    /// wants a call stack for backtraces overrides this (and
    /// `pop_call_frame`); the default is a no-op, so hosts that don't care
    /// pay nothing.
    ///
    /// The intended protocol: pop the frame when the call *succeeds*, but
    /// leave it in place when it fails, so that by the time an error has
    /// finished propagating out to whoever is watching for it, the frames
    /// still standing are exactly the chain of calls that were active at
    /// the moment things went wrong -- a backtrace frozen at throw time,
    /// not inspected after the stack has already unwound. `eval` follows
    /// this protocol at every call site; a host only needs to store and
    /// clear the frames.
    ///
    /// One caveat worth knowing: this reflects genuine call nesting, not
    /// full Lisp call semantics. A call in tail position is evaluated by
    /// the trampoline in `eval` *after* its caller's frame has already
    /// been popped -- that's the whole point of tail-call elimination, the
    /// caller's frame is gone, not still waiting -- so a chain of tail
    /// calls won't show up as a chain of frames, only the innermost one
    /// still "on the stack" will. Every non-tail call (argument
    /// evaluation, all but the last form in a body, and any nested
    /// `eval`/`funcall` performed by a primitive) is captured correctly.
    fn push_call_frame(&self, _frame: &str) {}
    /// Pop the most recently pushed frame, undoing one `push_call_frame`.
    fn pop_call_frame(&self) {}

    /// How many frames are currently pushed. Combined with
    /// `truncate_call_frames`, this lets a primitive that *catches* an
    /// error -- swallowing it into a returned value instead of letting it
    /// propagate, e.g. `eval-string-safe` -- restore the frame stack to
    /// how it was before its own (now-discarded) nested evaluation. That
    /// nested evaluation may have pushed frames that never got to pop
    /// (their call failed, per the `push_call_frame` protocol); without
    /// this they'd leak into whatever *actually* uncaught error is
    /// reported next, showing an unrelated, already-handled failure in a
    /// fresh backtrace.
    fn call_frame_depth(&self) -> usize {
        0
    }
    /// Pop frames until exactly `depth` remain (a no-op if already at or
    /// below `depth`).
    fn truncate_call_frames(&self, _depth: usize) {}
}
