//! Virtual text: text shown in a window at a position in the buffer without
//! being in it -- an inlay hint, a preview of what a command would insert.
//! Anchored at a point rather than over a span, so it moves with the text and
//! is never collapsed away.
//!
//! Design: `doc/architettura-editor.typ, <mark-overlay>`.
use crate::ui::Face;
use std::sync::Arc;

/// Some text to show at a position, and what made it.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualText {
    /// The character offset it sits in front of. The buffer's length is its
    /// end; the offset of a newline is the end of that line.
    pub at: usize,
    pub text: Arc<str>,
    pub face: Face,
    /// Higher is drawn first where two sit at the same offset; equal
    /// priorities are drawn oldest first, so the later one follows the
    /// earlier. The rule overlays have.
    pub priority: i32,
    /// What made it. A producer replaces its own batch by category rather than
    /// by remembering every handle it was given, which is what a language
    /// server answering again wants, and what still works after the module
    /// has been reloaded.
    pub category: Arc<str>,
    /// Stable for the life of the entry, so Lisp can hold one.
    pub id: usize,
}

/// A buffer's virtual text, kept in the order it is drawn: by offset, then by
/// descending priority, then by id. So a row is composed by a walk rather than
/// a sort -- for every visible line of every window, at every frame.
#[derive(Clone, Debug, Default)]
pub struct VirtualTextTable {
    entries: Vec<VirtualText>,
    next_id: usize,
}

impl VirtualTextTable {
    /// Adds TEXT at AT, and returns its id.
    pub fn insert(
        &mut self,
        at: usize,
        text: &str,
        face: Face,
        priority: i32,
        category: &str,
    ) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let entry = VirtualText {
            at,
            text: Arc::from(text),
            face,
            priority,
            category: Arc::from(category),
            id,
        };
        let position = self.entries.partition_point(|other| {
            (other.at, -other.priority, other.id) < (entry.at, -entry.priority, entry.id)
        });
        self.entries.insert(position, entry);
        id
    }

    /// Returns everything at AT, in the order it is drawn.
    pub fn at(&self, at: usize) -> impl Iterator<Item = &VirtualText> {
        let first = self.entries.partition_point(|entry| entry.at < at);
        self.entries[first..]
            .iter()
            .take_while(move |entry| entry.at == at)
    }

    /// Returns everything from FROM up to but not including TO, in drawing
    /// order: what a row is composed from, in one walk.
    pub fn between(&self, from: usize, to: usize) -> impl Iterator<Item = &VirtualText> {
        let first = self.entries.partition_point(|entry| entry.at < from);
        self.entries[first..]
            .iter()
            .take_while(move |entry| entry.at < to)
    }

    /// Removes the entry with ID. True when there was one.
    pub fn remove(&mut self, id: usize) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != before
    }

    /// Removes everything in CATEGORY, or everything when it is `None`, and
    /// returns how many went.
    pub fn clear(&mut self, category: Option<&str>) -> usize {
        let before = self.entries.len();
        match category {
            None => self.entries.clear(),
            Some(category) => self.entries.retain(|entry| &*entry.category != category),
        }
        before - self.entries.len()
    }

    /// Returns whether there is no virtual text.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns how many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Moves everything at or after AT along by N characters.
    ///
    /// At, not after: text inserted exactly where a hint sits pushes the hint
    /// along with the character it was sitting in front of.
    pub fn adjust_for_insert(&mut self, at: usize, n: usize) {
        if n == 0 {
            return;
        }
        for entry in &mut self.entries {
            if entry.at >= at {
                entry.at += n;
            }
        }
    }

    /// Moves everything back for the deletion of `[from, to)`: what was inside
    /// the deleted span lands at its start.
    ///
    /// Nothing is dropped, unlike overlays, which keep only what still has
    /// width: a point has no width to lose. A producer whose text is now in
    /// the wrong place clears its category and places it again.
    pub fn adjust_for_delete(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        let shift = to - from;
        for entry in &mut self.entries {
            entry.at = if entry.at <= from {
                entry.at
            } else if entry.at >= to {
                entry.at - shift
            } else {
                from
            };
        }
        // The move can put two entries in the same place, and the order has to
        // stay the one composing relies on.
        self.entries
            .sort_by_key(|entry| (entry.at, -entry.priority, entry.id));
    }
}
