//! The runtime's half of the facade, and the session's own small state:
//! whether the editor is running, how many shell commands are, and which
//! results `next-error` walks.
//!
//! Design: `doc/architettura-editor.typ, <coppie>`.

use super::*;

/// Takes one off COUNTER, and never goes below zero: a stray extra call leaves
/// the count at zero, rather than at `usize::MAX` with the renderer waking for
/// the rest of the session.
///
/// `fetch_update`, because `try_update`, its newer name, is not stable yet
/// (rust#135894).
fn finish_one(counter: &AtomicUsize) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        Some(n.saturating_sub(1))
    });
}

impl<B: BufferTrait> EditorState<B> {
    // ------------------------------------------------------------------
    // Running
    // ------------------------------------------------------------------

    /// Asks the editor to stop: [`Self::is_running`] answers false from now
    /// on, and the host's loop ends.
    pub(crate) fn quit(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Returns whether the editor is running: false once `quit` has been asked
    /// for.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    // ------------------------------------------------------------------
    // The search and the replace in progress
    // ------------------------------------------------------------------

    /// Starts an incremental search with SESSION, or goes on with it.
    pub(crate) fn begin_isearch(&self, session: Isearch) {
        self.runtime_mut(|runtime| runtime.begin_isearch(session));
    }

    /// Takes the incremental search in progress out of the editor, leaving
    /// none.
    ///
    /// Taken rather than lent: acting on a session takes a buffer's lock and
    /// moves point, and lending it would hold the runtime's lock across that.
    /// The caller gives it back with [`Self::begin_isearch`] when the search
    /// goes on.
    pub(crate) fn take_isearch(&self) -> Option<Isearch> {
        self.runtime_mut(|runtime| runtime.take_isearch())
    }

    /// Returns whether an incremental search is running. For reporting: what
    /// acts on the session takes it.
    pub(crate) fn isearch_active(&self) -> bool {
        self.runtime(|runtime| runtime.isearch_active())
    }

    /// Returns whether a replace is running.
    pub(crate) fn replace_active(&self) -> bool {
        self.runtime(|runtime| runtime.replace().is_some())
    }

    /// Returns how many matches the replace in progress has replaced.
    pub(crate) fn replace_count(&self) -> usize {
        self.runtime(|runtime| runtime.replace().map_or(0, |session| session.replaced))
    }

    // ------------------------------------------------------------------
    // Shell commands, and the results being walked
    // ------------------------------------------------------------------

    /// Counts a shell command that has started.
    pub(crate) fn begin_shell_command(&self) {
        self.shell_commands.fetch_add(1, Ordering::Relaxed);
    }

    /// Counts a shell command that has finished: from the command's own
    /// thread, once the last of its output is in its buffer.
    pub(crate) fn finish_shell_command(&self) {
        finish_one(&self.shell_commands);
    }

    /// Returns how many shell commands are running.
    pub(crate) fn shell_commands_running(&self) -> usize {
        self.shell_commands.load(Ordering::Relaxed)
    }

    /// Returns the buffer whose results `next-error` walks, if any.
    pub(crate) fn current_results(&self) -> Option<String> {
        self.current_results
            .read()
            .expect("Failed to acquire read lock on current_results")
            .clone()
    }

    /// Makes NAME the listing `M-g n` walks. Whatever made a listing sets it,
    /// so that the list just asked for is the one walked.
    pub(crate) fn set_current_results(&self, name: Option<String>) {
        *self
            .current_results
            .write()
            .expect("Failed to acquire write lock on current_results") = name;
    }

    // ------------------------------------------------------------------
    // A command's own state
    // ------------------------------------------------------------------

    /// Returns the column vertical movement is aiming for, if any.
    pub(crate) fn goal_column(&self) -> Option<usize> {
        self.runtime(|runtime| runtime.goal_column())
    }

    /// Sets the column vertical movement aims for, or forgets it.
    pub(crate) fn set_goal_column(&self, col: Option<usize>) {
        self.runtime_mut(|runtime| runtime.set_goal_column(col));
    }

    /// Opens a fresh execution budget for one entry into the interpreter -- a
    /// command, a hook, a file being loaded -- for as long as the scope it
    /// returns lives.
    ///
    /// Only the outermost scope refills, so a command that loads a file spends
    /// its own budget on it. What counts as one entry is the editor's to say,
    /// which is why this is here and not in `FuelMeter`.
    pub(crate) fn begin_command(&self) -> FuelScope {
        // The meter is cloned out and the lock given back: a scope lasts as
        // long as the command, across the interpreter.
        self.runtime(|runtime| runtime.fuel()).begin()
    }

    /// Returns the meter behind [`Self::begin_command`], for `risp::measure`,
    /// which holds a scope of its own while it measures.
    #[cfg(test)]
    pub(crate) fn fuel_meter(&self) -> Arc<FuelMeter> {
        self.runtime(|runtime| runtime.fuel())
    }

    /// Sets how much fuel a command starts with, and tops this thread's up to
    /// it: for the tests that want a small budget.
    #[cfg(test)]
    pub(crate) fn set_fuel_budget(&self, budget: u32) {
        self.runtime(|runtime| runtime.fuel()).set_budget(budget);
    }

    /// Returns the call stack captured when the last uncaught error happened,
    /// innermost call first, or nothing when none has happened since
    /// [`Self::clear_backtrace`]. How it is captured, and what a tail call
    /// leaves out of it, is `LispContext::push_call_frame`'s to say.
    pub fn backtrace(&self) -> Vec<String> {
        self.runtime(|runtime| runtime.backtrace())
    }

    /// Forgets the captured backtrace, so that the next error starts from a
    /// clean stack: what reports an error clears it once it has read it, as
    /// [`Self::report_error`] does.
    pub fn clear_backtrace(&self) {
        self.runtime_mut(|runtime| runtime.clear_backtrace());
    }

    // ------------------------------------------------------------------
    // The theme
    // ------------------------------------------------------------------

    /// Puts every face back as the editor ships it: the compiled-in styles,
    /// and those modules gave with `define-face`, whatever themes and
    /// `init.lisp` have done since.
    pub(crate) fn reset_theme(&self) {
        self.runtime_mut(|runtime| runtime.reset_theme());
    }

    /// Makes STYLE the one FACE is drawn with, in every window. The frames
    /// already taken keep the theme they were composed under.
    pub(crate) fn set_face_style(&self, face: Face, style: Style) {
        self.runtime_mut(|runtime| runtime.set_face_style(face, style));
    }

    /// Makes STYLE the one FACE ships with, which [`Self::reset_theme`] puts
    /// back, and the one it is drawn with now.
    pub(crate) fn define_face_style(&self, face: Face, style: Style) {
        self.runtime_mut(|runtime| runtime.define_face_style(face, style));
    }

    /// Returns the style FACE is drawn with.
    pub(crate) fn face_style(&self, face: Face) -> Style {
        self.runtime(|runtime| runtime.face_style(face))
    }

    /// Returns the theme in force.
    pub(crate) fn theme(&self) -> Arc<Theme> {
        self.runtime(|runtime| runtime.theme())
    }
}
