//! The facade over the WorkManager: what the host, and the tests, ask of the
//! editor's asynchronous work, and the topics the editor emits for it.
//!
//! # The topics
//!
//! A topic carries no data: it says "go and look", and what changed is read
//! from the state, where it happened. So each is emitted where the change is
//! made, in the one place every change of its kind goes through -- a change
//! that does not pass there is one no job can be woken for.
//!
//! | Topic | Emitted by |
//! |---|---|
//! | [`BUFFER_CHANGED`] | `edits::changed` (both doors, undo, redo, `replace-back`), `Buffer::adopt_text`, a change of major mode, a buffer joining the table |
//! | [`SYNTAX_CHANGED`] | `set_mode`, `edit_grammar`, `edit_syntax_table` |
use super::*;

/// A buffer's text changed, or a buffer came in: a colouring or a scan may be
/// out of date. Which buffer is for the job to find out, from the buffers'
/// versions.
pub(crate) const BUFFER_CHANGED: &str = "buffer-changed";

/// A grammar or a syntax table changed: what was coloured, or scanned, under
/// the old one may be out of date.
pub(crate) const SYNTAX_CHANGED: &str = "syntax-changed";

impl<B: BufferTrait> EditorState<B> {
    /// Stop the editor's asynchronous work, giving it WITHIN to finish.
    ///
    /// Every job leaves with `Exit::Shutdown`, and every dedicated thread is
    /// asked to stop; what has not finished by then is left behind, named in
    /// the log. For the host to call once, as the editor exits. Nothing calls
    /// it on its own: the executor's thread holds a copy of the editor, which
    /// holds the executor, and `Drop` cannot break that cycle.
    pub fn shutdown(&self, within: Duration) {
        self.work.shutdown(self, within);
    }

    /// The WorkManager, for the tests that drive it by hand.
    #[cfg(test)]
    pub(crate) fn work(&self) -> &WorkManager<Self> {
        &self.work
    }
}
