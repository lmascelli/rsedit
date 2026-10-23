//! The kill ring: the last several kills, so that killing something does not
//! lose what was about to be yanked. `yank` reads the current entry, and
//! `yank-pop` walks back through the others. One ring for the editor, not one
//! per buffer: killing in one buffer and yanking in another is the point.
//!
//! Design: `doc/architettura-editor.typ, <kill-ring>`.
use std::collections::VecDeque;

/// How many entries the ring keeps before dropping the oldest.
pub const DEFAULT_KILL_RING_MAX: usize = 60;

/// Which end of the most recent entry a continued kill joins onto: a run of
/// backward kills reads the right way round only if each joins at the front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// The kill ring: its entries, most recent first, and where `yank` reads.
#[derive(Debug)]
pub struct KillRing {
    /// Most recent entry first.
    entries: VecDeque<String>,
    max: usize,
    /// How far back `yank` currently reads. Reset by every new kill, advanced
    /// by [`Self::rotate`].
    index: usize,
}

impl Default for KillRing {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            max: DEFAULT_KILL_RING_MAX,
            index: 0,
        }
    }
}

impl KillRing {
    /// Returns how many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Sets how many entries the ring keeps, dropping the oldest past it.
    pub fn set_max(&mut self, max: usize) {
        self.max = max;
        self.trim();
    }

    /// Adds TEXT as a new entry. Empty text is not worth a slot: killing
    /// nothing should not push what was about to be yanked one step further
    /// away.
    pub fn push(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.entries.push_front(text);
        self.index = 0;
        self.trim();
    }

    /// Joins TEXT onto the most recent entry, or starts one if the ring is
    /// empty: what makes a run of `C-k` yank back as the whole passage.
    pub fn append(&mut self, text: String, direction: Direction) {
        if text.is_empty() {
            return;
        }
        match self.entries.front_mut() {
            Some(front) => {
                match direction {
                    Direction::Forward => front.push_str(&text),
                    Direction::Backward => front.insert_str(0, &text),
                }
                self.index = 0;
            }
            None => self.push(text),
        }
    }

    /// Returns what `yank` would insert, or `None` when nothing has been
    /// killed.
    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.index).map(String::as_str)
    }

    /// Steps one entry further back and returns it: what `yank-pop` inserts.
    /// Wraps round, so walking far enough returns to the most recent kill.
    pub fn rotate(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        self.index = (self.index + 1) % self.entries.len();
        self.current()
    }

    /// Returns the entry N steps back from the most recent, without moving
    /// the ring.
    pub fn nth(&self, n: usize) -> Option<&str> {
        self.entries.get(n).map(String::as_str)
    }

    /// Drops the oldest entries past the maximum.
    fn trim(&mut self) {
        while self.entries.len() > self.max {
            self.entries.pop_back();
        }
        // Dropping entries can leave the read position past the end -- and a
        // ring that yanks nothing because it was trimmed would be a confusing
        // way to lose text.
        if self.index >= self.entries.len() {
            self.index = 0;
        }
    }
}
