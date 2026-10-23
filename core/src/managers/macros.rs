//! A keyboard macro: the keys pressed while recording, kept so that they can
//! be pressed again, and the counter a macro can insert. Keys rather than
//! commands, and nothing here knows what a key does: that is the editor's
//! business when the key is pressed, both times.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
use crate::input::KeyEvent;

/// The commands whose keys a recording leaves out. The keys that end a
/// recording are pressed while it runs, and a macro ending with them would
/// end a recording when replayed.
const NOT_RECORDED: [&str; 3] = [
    "kmacro-start-macro",
    "kmacro-end-macro",
    // Ends a recording too: recorded, `C-x e` would make a macro that replays
    // itself for ever.
    "kmacro-end-and-call-macro",
];

/// How many replays may run on top of each other. A replay goes through the
/// key handler, so a macro calling itself grows the stack, and the stack runs
/// out -- aborting the process -- long before the execution budget does.
const MAX_DEPTH: usize = 16;

/// The recording in progress, the last macro, how deep replays are nested,
/// and the counter.
#[derive(Default)]
pub struct Macros {
    /// The keys gathered so far, or `None` when nothing is being recorded.
    recording: Option<Vec<KeyEvent>>,
    /// The macro most recently finished.
    last: Option<Vec<KeyEvent>>,
    /// How many replays are running on top of each other: a count, since a
    /// macro may call a macro, and no replayed key is recorded at any depth.
    replaying: usize,
    /// The counter a macro can insert, and which the macro that inserts it
    /// advances.
    counter: i64,
}

impl Macros {
    // ------------------------------------------------------------------
    // Recording
    // ------------------------------------------------------------------

    /// Begins recording, and returns false when a recording is already
    /// running.
    pub fn start(&mut self) -> bool {
        if self.recording.is_some() {
            return false;
        }
        self.recording = Some(Vec::new());
        // Here and not at each replay: a macro that numbers things counts on
        // across its repetitions, and from zero when it is recorded anew.
        self.counter = 0;
        true
    }

    /// Finishes recording, keeps what was gathered as the last macro, and
    /// returns it; `None` when nothing was being recorded, or nothing was
    /// gathered -- an empty macro would make `C-x e` do nothing, unexplained.
    pub fn finish(&mut self) -> Option<Vec<KeyEvent>> {
        let keys = self.recording.take()?;
        if keys.is_empty() {
            return None;
        }
        self.last = Some(keys.clone());
        Some(keys)
    }

    /// Abandons a recording without keeping it, and returns whether there was
    /// one.
    pub fn cancel(&mut self) -> bool {
        self.recording.take().is_some()
    }

    /// Returns whether a recording is running.
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Returns how many keys have been recorded so far.
    pub fn recorded(&self) -> usize {
        self.recording.as_ref().map(Vec::len).unwrap_or(0)
    }

    /// Records KEYS, which ran COMMAND or nothing, when a recording is
    /// running: not during a replay, and not when COMMAND is one of the
    /// recorder's own.
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

    /// Returns the macro most recently recorded.
    pub fn last(&self) -> Option<Vec<KeyEvent>> {
        self.last.clone()
    }

    /// Keeps KEYS as the last macro, as though they had just been recorded:
    /// a named macro made the current one, or a macro read from a
    /// configuration file.
    pub fn set_last(&mut self, keys: Vec<KeyEvent>) {
        self.last = Some(keys);
    }

    /// Notes that a replay is starting, and returns false when replays are
    /// nested too deep to be anything but a macro calling itself: see
    /// [`MAX_DEPTH`].
    pub fn begin_replay(&mut self) -> bool {
        if self.replaying >= MAX_DEPTH {
            return false;
        }
        self.replaying += 1;
        true
    }

    /// Notes that a replay has ended.
    pub fn end_replay(&mut self) {
        self.replaying = self.replaying.saturating_sub(1);
    }

    // ------------------------------------------------------------------
    // The counter
    // ------------------------------------------------------------------

    /// Returns the counter.
    pub fn counter(&self) -> i64 {
        self.counter
    }

    /// Sets the counter to VALUE.
    pub fn set_counter(&mut self, value: i64) {
        self.counter = value;
    }

    /// Returns the counter, and advances it by BY: one operation, so that a
    /// number inserted is always a number moved past.
    pub fn take_counter(&mut self, by: i64) -> i64 {
        let value = self.counter;
        self.counter = self.counter.saturating_add(by);
        value
    }
}
