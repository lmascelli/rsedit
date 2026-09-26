//! Odds and ends belonging to the session rather than to any document:
//! whether the editor is running, what the worker is doing, and the small
//! pieces of state that reset between commands.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Quit the editor
    pub(crate) fn quit(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Start, or continue, an incremental search.
    pub(crate) fn begin_isearch(&self, session: Isearch) {
        self.runtime_mut(|runtime| runtime.begin_isearch(session));
    }

    /// Take the running search *out* of the editor, leaving none behind.
    ///
    /// Out rather than borrowed, because acting on a session means taking
    /// buffer locks and moving point. Handing a `&mut` to a closure would mean
    /// holding this lock across all of that, putting an ordering between it and
    /// the buffers that nothing else in the editor respects. Taking it out owes
    /// no ordering to anything -- the caller puts it back with
    /// [`Self::begin_isearch`] when the search continues, and simply drops it
    /// when it does not.
    pub(crate) fn take_isearch(&self) -> Option<Isearch> {
        self.runtime_mut(|runtime| runtime.take_isearch())
    }

    /// Whether an incremental search is running. Only for reporting -- anything
    /// that acts on the session takes it.
    pub(crate) fn isearch_active(&self) -> bool {
        self.runtime(|runtime| runtime.isearch_active())
    }

    /// Send a task to the worker thread, and say whether it was accepted.
    ///
    /// The mailbox used to be a public field, so anything could post work to
    /// the background scheduler. It is a method now for the same reason the
    /// rest of the state is: there is exactly one queue, and the count of what
    /// is in flight has to be kept in step with it -- see `begin_shell_command`.
    pub(crate) fn send_to_worker(&self, message: WorkerMessage<B>) -> bool {
        self.worker_mailbox.send(message).is_ok()
    }

    /// Note that a shell command has started.
    pub(crate) fn begin_shell_command(&self) {
        self.shell_commands.fetch_add(1, Ordering::Relaxed);
    }

    /// Note that one has finished. Called from the worker thread, after the
    /// last of its output is in the buffer.
    pub(crate) fn finish_shell_command(&self) {
        // Saturating rather than wrapping: a stray extra call would otherwise
        // take the count to `usize::MAX` and leave the renderer spinning for
        // the rest of the session.
        let _ = self
            .shell_commands
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_sub(1))
            });
    }

    /// How many shell commands are still running.
    pub(crate) fn shell_commands_running(&self) -> usize {
        self.shell_commands.load(Ordering::Relaxed)
    }

    pub(crate) fn goal_column(&self) -> Option<usize> {
        self.runtime(|runtime| runtime.goal_column())
    }

    pub(crate) fn set_goal_column(&self, col: Option<usize>) {
        self.runtime_mut(|runtime| runtime.set_goal_column(col));
    }

    /// Open a fresh Lisp execution budget for one top-level command -- a
    /// keystroke, a hook run, a config file being loaded.
    ///
    /// What counts as "one command" is editor *policy*, which is why this lives
    /// here rather than in `FuelMeter`: only the editor knows a keystroke is one
    /// unit of work. Nesting is safe -- the meter tracks depth and only the
    /// outermost scope refills -- so a command that re-enters the evaluator, via
    /// the Lisp-callable `eval-file` primitive for instance, keeps spending the
    /// budget it already has instead of quietly being handed a new one.
    pub(crate) fn begin_command(&self) -> FuelScope {
        // The meter is cloned out and the lock given straight back: a metered
        // scope lasts a whole command, and holding this lock for that long
        // would be holding it across the interpreter.
        self.runtime(|runtime| runtime.fuel()).begin()
    }

    /// The execution meter behind [`Self::begin_command`].
    ///
    /// Exposed for `lisp::measure`, which needs the meter to hold a scope of
    /// its own for the duration of a measurement.
    pub(crate) fn fuel_meter(&self) -> Arc<FuelMeter> {
        self.runtime(|runtime| runtime.fuel())
    }

    /// Set how much fuel a fresh command receives, and top the current thread's
    /// remaining fuel up to it. Exposed so the `set-command-fuel` primitive --
    /// and tests that want a deliberately tiny budget -- can reach it.
    pub(crate) fn set_fuel_budget(&self, budget: u32) {
        self.runtime(|runtime| runtime.fuel()).set_budget(budget);
    }

    /// Return the call stack captured at the point of the most recent
    /// uncaught error, innermost (deepest) call first -- or an empty list
    /// if nothing has errored since the last `clear_backtrace`. See
    /// `LispContext::push_call_frame` for the capture protocol and its
    /// tail-call caveat.
    pub fn backtrace(&self) -> Vec<String> {
        self.runtime(|runtime| runtime.backtrace())
    }

    /// Discard the captured backtrace, so the next error starts from a
    /// clean stack instead of stacking on top of a stale one. Callers that
    /// catch and report an error (a key handler, `eval_file`, ...) should
    /// call this once they're done reading `backtrace()`.
    pub fn clear_backtrace(&self) {
        self.runtime_mut(|runtime| runtime.clear_backtrace());
    }
}
