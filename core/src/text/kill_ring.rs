//! The kill ring: what killed and copied text goes into, and what yank reads.
//!
//! # Why a ring and not a clipboard
//!
//! A clipboard holds the last thing you cut. A ring holds the last several, so
//! that killing something does not destroy what you were about to paste. That
//! is the whole difference, and it is why [`KillRing::rotate`] exists: after a
//! yank, `yank-pop` walks back through earlier kills in place, replacing what
//! was just inserted rather than adding to it.
//!
//! # Why it is not per-buffer
//!
//! Killing in one buffer and yanking in another is the point. The ring lives
//! on the editor, not the buffer, so the text crosses.
use std::collections::VecDeque;

/// How many entries the ring keeps before dropping the oldest.
pub const DEFAULT_KILL_RING_MAX: usize = 60;

/// Which end of the most recent entry a continued kill joins onto.
///
/// Repeated `C-k` should end up with one entry reading forwards, and repeated
/// backward kills with one entry reading backwards; a kill that ignored
/// direction would assemble the text in the order it was deleted, which for a
/// backward kill is inside out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

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
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn set_max(&mut self, max: usize) {
        self.max = max;
        self.trim();
    }

    /// Add TEXT as a new entry.
    ///
    /// Empty text is not worth a slot: killing nothing should not push the
    /// thing you wanted to yank one step further away.
    pub fn push(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.entries.push_front(text);
        self.index = 0;
        self.trim();
    }

    /// Join TEXT onto the most recent entry, or start one if the ring is
    /// empty.
    ///
    /// This is what makes a run of `C-k` yank back as the whole passage
    /// instead of only its last line.
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

    /// What `yank` would insert, or `None` when nothing has been killed.
    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.index).map(String::as_str)
    }

    /// Step one entry further back and return it -- what `yank-pop` inserts.
    ///
    /// Wraps around, so walking far enough returns to the most recent kill
    /// rather than running out.
    pub fn rotate(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        self.index = (self.index + 1) % self.entries.len();
        self.current()
    }

    /// The entry N steps back from the most recent, without moving the ring.
    pub fn nth(&self, n: usize) -> Option<&str> {
        self.entries.get(n).map(String::as_str)
    }

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
