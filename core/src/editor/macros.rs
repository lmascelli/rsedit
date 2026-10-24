//! The keyboard macros' half of the facade: recording the keys that ran a
//! command, and pressing them again through the door a keystroke comes in by.
//!
//! Design: `doc/architettura-editor.typ, <macro>`.
use super::*;
use crate::input::KeyEvent;

impl<B: BufferTrait> EditorState<B> {
    /// Counts a failed command, so that a replay in progress can stop.
    pub(crate) fn note_command_error(&self) {
        self.command_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns how many commands have failed since the editor started.
    pub(crate) fn command_errors(&self) -> usize {
        self.command_errors.load(Ordering::Relaxed)
    }

    /// Records KEYS, which ran COMMAND or nothing, if a recording is running.
    ///
    /// The key handler calls it where each key is used up -- by a capture, by
    /// the prefix argument's reader, by a whole sequence -- since only the
    /// handler knows how many keys that was.
    pub(crate) fn record_keys(&self, keys: &[KeyEvent], command: Option<&str>) {
        // A read lock and a comparison before the write lock: this is on the
        // path of every key, and a recording is rarely running. Which keys a
        // running recording keeps is `Macros::observe`'s to say, not this.
        if self.macros(|macros| !macros.is_recording()) {
            return;
        }
        self.macros_mut(|macros| macros.observe(keys, command));
    }

    /// Presses KEYS again, COUNT times, and returns how many repetitions ran
    /// to the end.
    ///
    /// The first command that fails stops them all: every key after it would
    /// be pressed in a state the recording never saw.
    pub(crate) fn replay_keys(
        &self,
        keys: &[KeyEvent],
        count: usize,
        env: &Arc<Env<EditorState<B>>>,
    ) -> usize {
        if keys.is_empty() {
            return 0;
        }
        if !self.macros_mut(|macros| macros.begin_replay()) {
            // A macro calling itself. Each replay is a real call through the
            // key handler, so the stack would run out, aborting the process,
            // long before the execution budget did.
            self.set_echo_message("Keyboard macro is nested too deeply");
            return 0;
        }
        let mut done = 0;
        for _ in 0..count {
            // A stamp of its own for each repetition, held for the whole of
            // it: one undo takes back one repetition.
            crate::buffer::undo::begin_command(false);
            let _group = crate::buffer::undo::OneGroup::begin();
            let before = self.command_errors();
            for key in keys {
                self.handle_key_event(key.clone(), env);
                if self.command_errors() != before {
                    // What failed has said so already. That the macro stopped
                    // is another fact, and the echo area says it too.
                    self.set_echo_message("Macro stopped: a command in it failed");
                    self.macros_mut(|macros| macros.end_replay());
                    return done;
                }
            }
            done += 1;
        }
        self.macros_mut(|macros| macros.end_replay());
        done
    }
}
