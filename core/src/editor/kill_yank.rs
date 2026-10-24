//! The kill ring's half of the facade: a kill, with the Lisp setting that
//! sends it to the system clipboard read here, since no compartment takes an
//! `Env`; and the ring's operations under the editor's names.
//!
//! Design: `doc/architettura-editor.typ, <coppie>`.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Saves TEXT as killed text: joined onto the newest entry, at the end
    /// DIRECTION says, when the command before this one killed too, so that a
    /// run of `C-k` yanks back as one passage.
    pub(crate) fn kill(&self, text: String, direction: Direction, env: &Arc<Env<Self>>) {
        // Read before the lock is taken: no lock of the editor is held while
        // the interpreter is touched.
        let to_clipboard = Self::clipboard_sync_enabled(env);
        self.kill_yank_mut(|kills| kills.kill(text, direction, to_clipboard));
    }

    /// Returns whether killed text goes to the system clipboard too:
    /// `clipboard-sync`, which `clipboard.lisp` sets, and no when it is
    /// unbound, as in this crate's tests, which load no Lisp.
    fn clipboard_sync_enabled(env: &Arc<Env<Self>>) -> bool {
        env.get_variable("clipboard-sync")
            .is_some_and(|flag| flag.is_truthy())
    }

    /// Takes the text owed to the system clipboard. [`Self::snapshot`] takes it
    /// once per frame, so that redrawing an unchanged frame does not send it
    /// again.
    pub(crate) fn take_pending_clipboard(&self) -> Option<String> {
        self.kill_yank_mut(|kills| kills.take_pending_clipboard())
    }

    /// Returns what `yank` would insert, if anything.
    pub(crate) fn current_kill(&self) -> Option<String> {
        self.kill_yank(|kills| kills.current())
    }

    /// Returns the entry N kills back, without moving the ring.
    pub(crate) fn nth_kill(&self, n: usize) -> Option<String> {
        self.kill_yank(|kills| kills.nth(n))
    }

    /// Steps the ring back one entry, and returns what is now current.
    pub(crate) fn rotate_kill_ring(&self) -> Option<String> {
        self.kill_yank_mut(|kills| kills.rotate())
    }

    /// Sets how many entries the ring keeps.
    pub(crate) fn set_kill_ring_max(&self, max: usize) {
        self.kill_yank_mut(|kills| kills.set_max(max));
    }

    /// Returns how many entries the ring holds.
    pub(crate) fn kill_ring_len(&self) -> usize {
        self.kill_yank(|kills| kills.len())
    }

    /// Records that a yank put LEN characters at AT, for `yank-pop` to take
    /// back out.
    pub(crate) fn note_yank(&self, at: usize, len: usize) {
        self.kill_yank_mut(|kills| kills.note_yank(at, len));
    }

    /// Returns what the previous command yanked, when it was a yank: the only
    /// text `yank-pop` can safely replace is the text a yank has just put in.
    pub(crate) fn yank_to_replace(&self) -> Option<(usize, usize)> {
        self.kill_yank(|kills| kills.yank_to_replace())
    }

    /// Moves this command's kill and yank flags into the previous command's,
    /// as a command starts.
    pub(super) fn roll_over_command_flags(&self) {
        self.kill_yank_mut(|kills| kills.roll_over());
    }
}
