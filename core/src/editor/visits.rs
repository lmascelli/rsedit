//! The visits' half of the facade: a file recorded as visited, point recorded
//! where a file buffer was left, and found again when the file is opened, as
//! `recent-files-length` and `save-place` say.
//!
//! Design: `doc/architettura-editor.typ, <memoria>`.

use super::*;
use crate::managers::DEFAULT_RECENT_FILES_LENGTH;

/// The name of the Lisp variable holding how many files `recent-files` lists.
pub(crate) const RECENT_FILES_LENGTH: &str = "recent-files-length";

/// The name of the Lisp variable that turns remembering point in each file
/// off.
pub(crate) const SAVE_PLACE: &str = "save-place";

/// Returns how many files `recent-files` keeps, as Lisp has it now:
/// `recent-files-length` when it is a number of at least zero, and 50
/// otherwise.
pub(crate) fn recent_files_length<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> usize {
    match env.number_at_least(RECENT_FILES_LENGTH, 0.0) {
        Some(n) => n as usize,
        None => DEFAULT_RECENT_FILES_LENGTH,
    }
}

/// Returns whether point is remembered in each file, as Lisp has it now;
/// unset counts as on.
pub(crate) fn saving_places<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(SAVE_PLACE, true)
}

impl<B: BufferTrait> EditorState<B> {
    /// Records PATH, an absolute path, as the file visited last.
    pub(crate) fn record_visit(&self, path: &str, env: &Arc<Env<EditorState<B>>>) {
        let limit = recent_files_length(env);
        self.visits_mut(|visits| visits.visited(path, limit));
    }

    /// Records where point is in the buffer NAME, when it visits a file and
    /// `save-place` is on: for the next time that file is opened.
    pub(crate) fn record_place(&self, name: &str, env: &Arc<Env<EditorState<B>>>) {
        if !saving_places(env) {
            return;
        }
        // Copied out first: one compartment at a time.
        let Some(Some((path, point))) = self.with_buffer(name, |buf| {
            buf.file_path
                .clone()
                .map(|path| (path, buf.text.cursor_pos_1d()))
        }) else {
            return;
        };
        self.visits_mut(|visits| visits.left_at(&path, point));
    }

    /// Records where point is in every buffer visiting a file, as
    /// [`Self::record_place`] does for one.
    pub(crate) fn record_every_place(&self, env: &Arc<Env<EditorState<B>>>) {
        for name in self.buffer_names() {
            self.record_place(&name, env);
        }
    }

    /// Returns where point was when PATH was last left, when `save-place` is
    /// on and it is remembered.
    pub(crate) fn place_for(&self, path: &str, env: &Arc<Env<EditorState<B>>>) -> Option<usize> {
        if !saving_places(env) {
            return None;
        }
        self.visits(|visits| visits.place(path))
    }
}
