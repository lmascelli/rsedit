//! The facade over the WorkManager: what the host, and the tests, ask of the
//! editor's asynchronous work.
use super::*;

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
