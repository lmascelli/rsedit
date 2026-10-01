//! A keyboard macro: the keys that were pressed, kept so they can be pressed
//! again.
//!
//! # Why this is a compartment and not four fields
//!
//! Because two of the operations touch two of the fields and must not be seen
//! half-done. Ending a recording moves the keys being gathered into the slot
//! the last macro lives in -- held apart, a reader between the two sees either
//! two copies of the macro or none. And whether a key is recorded at all is a
//! question about `recording` *and* `replaying` together: a macro calling a
//! macro while one is being recorded must record the call and not what the
//! call expands to.
//!
//! [`Runtime`](super::Runtime) would have been the other home, and is candid
//! about grouping by lifetime rather than by a shared invariant. These have
//! one, so they are here.
//!
//! # What is recorded, and why it is keys
//!
//! Keys, not commands. Not every key becomes a command: `C-u` and the digits
//! after it are taken by the prefix-argument reader and never reach a keymap,
//! and a command that reads a character of its own -- `zap-to-char`, through
//! `read-key-sequence` -- consumes the character without a second command
//! running. A recorder that watched commands would replay `zap-to-char` as a
//! wait for a character that never arrives.
//!
//! Keys also mean a macro follows whatever its keys mean *now*: rebind a key
//! and the macro does the new thing, which is what Emacs chose and what makes
//! a macro a shorthand for pressing keys rather than a frozen program.
//!
//! And they serialise for nothing. `describe_keys` and `parse_key_sequence`
//! are inverses, so a macro is a string in the syntax `define-key` already
//! takes -- which is the whole of `insert-kbd-macro` and of putting one in a
//! config file.
//!
//! # Nothing here knows what a key does
//!
//! No keymaps, no commands, no Lisp. A key arrives as a [`KeyEvent`] and
//! leaves as one; what it meant when it was pressed, and what it will mean
//! when it is pressed again, are the editor's business both times.
use crate::input::KeyEvent;

/// The commands a recording must not record.
///
/// The key that ends a recording is pressed *while* recording, so without this
/// every macro would end with the keys that stopped it -- and replaying one
/// would stop a recording that was not running, or start one that then never
/// ended.
///
/// By name, and here beside the state rather than in the key handler: which
/// commands are the recorder's own is the recorder's question.
const NOT_RECORDED: [&str; 3] = [
    "kmacro-start-macro",
    "kmacro-end-macro",
    // Ends a recording as much as `kmacro-end-macro` does, so its keys are as
    // much not part of one. Leaving it out of this list made `C-x e' while
    // recording finish a macro whose only content was `C-x e' -- which then
    // replayed itself, for ever.
    "kmacro-end-and-call-macro",
];

/// How many replays may be running on top of each other.
///
/// # Why there has to be a limit
///
/// Because a replay is a real Rust call: a key goes through the key handler,
/// which runs a command, which replays, which goes through the key handler. A
/// macro that calls itself therefore grows the *stack*, and the stack runs out
/// before the execution budget does -- the budget is spent a few steps per
/// level, and there are tens of thousands of levels' worth of it. The symptom
/// is not an error but the process aborting, which is the one outcome an editor
/// must not have.
///
/// Sixteen because legitimate nesting is a macro calling a named macro that
/// calls another, which is two or three deep in anything anybody has written.
const MAX_DEPTH: usize = 16;

#[derive(Default)]
pub struct Macros {
    /// The keys gathered so far, or `None` when nothing is being recorded.
    recording: Option<Vec<KeyEvent>>,
    /// The macro most recently finished.
    last: Option<Vec<KeyEvent>>,
    /// How many replays are in progress on top of each other.
    ///
    /// A count rather than a flag: a macro may call a macro, and the one thing
    /// that must be true at every depth is that a replayed key is not recorded
    /// again.
    replaying: usize,
    /// The counter a macro can insert, and which the macro that inserts it
    /// advances.
    counter: i64,
}

impl Macros {
    // ------------------------------------------------------------------
    // Recording
    // ------------------------------------------------------------------

    /// Begin recording. False when one is already being recorded.
    pub fn start(&mut self) -> bool {
        if self.recording.is_some() {
            return false;
        }
        self.recording = Some(Vec::new());
        // Reset here rather than at each replay: a macro that numbers things
        // wants to carry on counting across its repetitions, and wants to
        // start again when a new macro is recorded.
        self.counter = 0;
        true
    }

    /// Finish recording, and keep what was gathered as the last macro.
    ///
    /// `None` when nothing was being recorded, or when what was gathered is
    /// empty -- an empty macro is not a macro, and keeping one would make
    /// `C-x e` do nothing with no way to tell why.
    ///
    /// One operation rather than a read and a clear, because between them a
    /// second caller sees the keys in both places or in neither.
    pub fn finish(&mut self) -> Option<Vec<KeyEvent>> {
        let keys = self.recording.take()?;
        if keys.is_empty() {
            return None;
        }
        self.last = Some(keys.clone());
        Some(keys)
    }

    /// Abandon a recording without keeping it. True when there was one.
    pub fn cancel(&mut self) -> bool {
        self.recording.take().is_some()
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// How many keys have been recorded so far.
    pub fn recorded(&self) -> usize {
        self.recording.as_ref().map(Vec::len).unwrap_or(0)
    }

    /// Note that KEYS were pressed, if anything is listening.
    ///
    /// COMMAND is what they ran, or `None` when they ran nothing. The whole
    /// decision is here: nothing is recorded unless a recording is running,
    /// nothing is recorded during a replay, and the recorder's own commands
    /// are never recorded.
    pub fn observe(&mut self, keys: &[KeyEvent], command: Option<&str>) {
        if self.replaying > 0 {
            return;
        }
        if command.is_some_and(|name| NOT_RECORDED.contains(&name)) {
            return;
        }
        if let Some(recording) = self.recording.as_mut() {
            recording.extend_from_slice(keys);
        }
    }

    // ------------------------------------------------------------------
    // Replaying
    // ------------------------------------------------------------------

    /// The macro most recently recorded.
    pub fn last(&self) -> Option<Vec<KeyEvent>> {
        self.last.clone()
    }

    /// Keep KEYS as the last macro, as though they had just been recorded.
    ///
    /// For a named macro being made the current one, and for a macro read back
    /// out of a config file.
    pub fn set_last(&mut self, keys: Vec<KeyEvent>) {
        self.last = Some(keys);
    }

    /// Note that a replay is starting. False when they are nested too deeply
    /// to be anything but a macro calling itself -- see [`MAX_DEPTH`].
    pub fn begin_replay(&mut self) -> bool {
        if self.replaying >= MAX_DEPTH {
            return false;
        }
        self.replaying += 1;
        true
    }

    pub fn end_replay(&mut self) {
        self.replaying = self.replaying.saturating_sub(1);
    }

    pub fn is_replaying(&self) -> bool {
        self.replaying > 0
    }

    // ------------------------------------------------------------------
    // The counter
    // ------------------------------------------------------------------

    pub fn counter(&self) -> i64 {
        self.counter
    }

    pub fn set_counter(&mut self, value: i64) {
        self.counter = value;
    }

    /// The counter now, with the counter advanced past it.
    ///
    /// One operation, because the two halves are what "insert the counter"
    /// means and a caller that did them separately could insert a number it
    /// then failed to move past.
    pub fn take_counter(&mut self, by: i64) -> i64 {
        let value = self.counter;
        self.counter = self.counter.saturating_add(by);
        value
    }
}
