//! Recording keys and pressing them again.
//!
//! The state is in [`Macros`]; this is the half that knows what a key *is*.
//!
//! # Replay is the ordinary key path
//!
//! A replay feeds each recorded key back through
//! [`EditorState::handle_key_event`], the same door a keystroke from the
//! terminal comes through. Not a cheaper imitation of it: a macro that went
//! through a second dispatcher would be a second set of rules about prefix
//! arguments, transient keymaps, key captures and prompts, kept in step with
//! the first by hand.
//!
//! What that costs is re-entrancy, and it is already paid for. The fuel meter
//! counts depth and only the outermost scope refills, so the commands a replay
//! runs spend the budget the replay was given rather than each being handed a
//! fresh one -- which is also what stops a macro that calls itself from running
//! for ever. `pending_keys` is empty by the time a replay starts, because the
//! key that started it was a complete sequence.
//!
//! # What a replay cannot be interrupted by
//!
//! A key. The loop below reads none: it is inside one command, and the thread
//! that would deliver `C-g` is the thread running the loop.
//!
//! So a macro that calls itself is stopped by a depth limit, and not by the
//! execution budget. That was the first thing tried and it does not work: each
//! level of replay is a real call through the key handler, so the *stack* runs
//! out while the budget is barely touched, and a test of it aborted the
//! process rather than reporting an error. See
//! [`Macros::begin_replay`](crate::managers::Macros::begin_replay).
//!
//! That is also the whole of why it draws one frame rather than fifty: the
//! renderer gets its turn when the command returns.
use super::*;
use crate::input::KeyEvent;

impl<B: BufferTrait> EditorState<B> {
    /// Ask the macro state something.
    pub(crate) fn macros<R>(&self, f: impl FnOnce(&Macros) -> R) -> R {
        f(&self.macros.read().expect("read lock on macros"))
    }

    /// Change it.
    pub(crate) fn macros_mut<R>(&self, f: impl FnOnce(&mut Macros) -> R) -> R {
        f(&mut self.macros.write().expect("write lock on macros"))
    }

    /// Note that a command failed, so a replay in progress can stop.
    pub(crate) fn note_command_error(&self) {
        self.command_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// How many commands have failed since the editor started.
    pub(crate) fn command_errors(&self) -> usize {
        self.command_errors.load(Ordering::Relaxed)
    }

    /// Note that KEYS were pressed, running COMMAND.
    ///
    /// Called from the key handler at each of the places a key is consumed --
    /// a capture, the prefix-argument reader, a resolved sequence -- because
    /// those are three different numbers of keys and only the handler knows
    /// which it just used.
    pub(crate) fn record_keys(&self, keys: &[KeyEvent], command: Option<&str>) {
        // A read lock and one comparison before the write lock, because a
        // recording is not running almost always and this is on the path of
        // every keystroke.
        //
        // Only that: whether a key that arrives *while* a recording is running
        // should be kept is [`Macros::observe`]'s decision and is not repeated
        // here. It was, once -- this asked about `is_replaying` too -- and the
        // ablation that removed the rule from `observe` changed nothing,
        // because the copy here was still enforcing it. Two places that agree
        // are one place that can stop agreeing.
        if self.macros(|macros| !macros.is_recording()) {
            return;
        }
        self.macros_mut(|macros| macros.observe(keys, command));
    }

    /// Press KEYS again, COUNT times.
    ///
    /// Returns how many repetitions ran to the end. A repetition that fails
    /// stops everything: every key after a failure is being pressed in a state
    /// the recording never saw, and a macro that carries on from there is how
    /// one does damage.
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
            // A macro calling itself. Refused rather than left to the
            // execution budget: a replay is a real call, so this grows the
            // stack, and the stack runs out long before the budget does. The
            // symptom then is the process aborting rather than an error, which
            // is the one outcome an editor must not have.
            self.set_echo_message("Keyboard macro is nested too deeply");
            return 0;
        }
        let mut done = 0;
        for _ in 0..count {
            // A fresh stamp per repetition, and then held for the whole of it:
            // one undo step takes back one repetition. Twenty of them are
            // twenty steps, so a macro that went wrong on the nineteenth can
            // be walked back to it.
            crate::buffer::undo::begin_command(false);
            let _group = crate::buffer::undo::OneGroup::begin();
            let before = self.command_errors();
            for key in keys {
                self.handle_key_event(key.clone(), env);
                if self.command_errors() != before {
                    // Reported already, by whatever failed. Said again here
                    // because "the macro stopped" is a different fact from
                    // "that command failed", and the second does not imply the
                    // first to somebody reading the echo area.
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
