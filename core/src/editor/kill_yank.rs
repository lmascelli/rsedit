//! The kill-ring half of the facade.
//!
//! The compartment is [`crate::managers::KillYank`]. The Lisp setting that
//! decides whether a kill also reaches the system clipboard is read here,
//! because no compartment takes an `Env`.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Save TEXT as killed text.
    ///
    /// A run of kill commands accumulates into one entry rather than filling
    /// the ring with fragments -- that is what makes repeated `C-k` yank back
    /// as the whole passage. DIRECTION says which end of the entry a
    /// continued kill joins onto, so a backward kill does not assemble its
    /// text inside out.
    pub(crate) fn kill(&self, text: String, direction: Direction, env: &Arc<Env<Self>>) {
        // Asked before the lock is taken. It is a Lisp variable, and no
        // compartment may be holding a lock when the interpreter is touched.
        let to_clipboard = Self::clipboard_sync_enabled(env);
        self.kill_yank_mut(|kills| kills.kill(text, direction, to_clipboard));
    }

    /// Ask the kill ring something. Same rules as [`EditorState::windows`].
    pub(crate) fn kill_yank<R>(&self, f: impl FnOnce(&KillYank) -> R) -> R {
        f(&self.kill_yank.read().expect("read lock on kill_yank"))
    }

    /// Change it -- kill, yank, rotate, or roll the flags over.
    pub(crate) fn kill_yank_mut<R>(&self, f: impl FnOnce(&mut KillYank) -> R) -> R {
        f(&mut self.kill_yank.write().expect("write lock on kill_yank"))
    }

    /// Whether killed text should also reach the system clipboard.
    ///
    /// Unbound means no. The variable is set by `clipboard.lisp`, so the
    /// editor comes up with it on; a harness that loads no Lisp -- which is
    /// every test in this crate -- gets the old behaviour untouched rather
    /// than queueing a clipboard payload on every kill it makes.
    fn clipboard_sync_enabled(env: &Arc<Env<Self>>) -> bool {
        env.get_variable("clipboard-sync")
            .is_some_and(|flag| flag.is_truthy())
    }

    /// Take the text owed to the system clipboard, leaving nothing behind.
    ///
    /// Called once per frame by [`Self::snapshot`]. Taking rather than reading
    /// is what stops a redraw of an unchanged frame from re-sending the same
    /// escape.
    pub(crate) fn take_pending_clipboard(&self) -> Option<String> {
        self.kill_yank_mut(|kills| kills.take_pending_clipboard())
    }

    /// What `yank` would insert, if anything.
    pub(crate) fn current_kill(&self) -> Option<String> {
        self.kill_yank(|kills| kills.current())
    }

    /// The entry N kills back, without moving the ring.
    pub(crate) fn nth_kill(&self, n: usize) -> Option<String> {
        self.kill_yank(|kills| kills.nth(n))
    }

    /// Step the ring back one entry and return what is now current.
    pub(crate) fn rotate_kill_ring(&self) -> Option<String> {
        self.kill_yank_mut(|kills| kills.rotate())
    }

    pub(crate) fn set_kill_ring_max(&self, max: usize) {
        self.kill_yank_mut(|kills| kills.set_max(max));
    }

    pub(crate) fn kill_ring_len(&self) -> usize {
        self.kill_yank(|kills| kills.len())
    }

    /// Remember that a yank put LEN characters at AT, so `yank-pop` knows what
    /// to take back out.
    pub(crate) fn note_yank(&self, at: usize, len: usize) {
        self.kill_yank_mut(|kills| kills.note_yank(at, len));
    }

    /// What the previous command yanked, if the previous command was a yank.
    ///
    /// `yank-pop` replaces the text a yank just inserted, so it is only
    /// meaningful directly after one; anything else in between and there is
    /// nothing it would be safe to remove.
    pub(crate) fn yank_to_replace(&self) -> Option<(usize, usize)> {
        self.kill_yank(|kills| kills.yank_to_replace())
    }

    /// Roll "this command" into "the previous command" for the flags that a
    /// command needs to ask about its predecessor.
    pub(super) fn roll_over_command_flags(&self) {
        self.kill_yank_mut(|kills| kills.roll_over());
    }
}
