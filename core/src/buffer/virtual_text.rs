//! Text shown in a window that is not in the buffer.
//!
//! # What this is for
//!
//! An inlay hint from a language server -- a parameter name or an inferred
//! type, shown where it would go if it had been written. A preview of what a
//! command is about to insert, before it has inserted anything. Anything the
//! reader should see at a position in the text without the text having
//! changed.
//!
//! None of that is expressible as an overlay, because an overlay puts a face
//! on characters that are already there, and these have no characters. And
//! none of it can be done by inserting the text for real: the buffer would be
//! modified, the undo history would fill with things the user never typed, and
//! saving would write a parameter name nobody asked for into their file.
//!
//! `man/overlays.txt` used to say this was "not accepted rather than stored
//! and quietly ignored", because it "would need renderer and input support
//! this editor does not have". That was exactly right, and
//! [`crate::ui::layout`] is that support.
//!
//! # A point, not a span
//!
//! Virtual text sits *between* two characters, so it is anchored at one
//! offset rather than over a range -- which is also why it could not simply be
//! a field on [`Overlay`](crate::buffer::Overlay). An overlay with no width is
//! deleted by [`OverlayTable::adjust_for_delete`](crate::buffer::OverlayTable),
//! which keeps only the overlays whose `start < end`; that rule is what stops
//! a highlight lingering after everything it covered has gone, and a point has
//! nothing for it to be true of.
//!
//! So this is its own table, with the same discipline and one different rule:
//! a point is never collapsed away. It moves, and that is all that can happen
//! to it.
//!
//! # Nothing here knows what it is for
//!
//! No diagnostics, no hints, no blame. Those are *categories* a producer
//! chooses, exactly as `isearch` is a category of overlay -- and the whole of
//! what the editor does with a category is let that producer replace its own
//! batch with one call. A mechanism that knew the difference between a hint
//! and a preview would be one that had to be taught about the next kind.
use crate::ui::Face;
use std::sync::Arc;

/// Some text to show at a position, and what made it.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualText {
    /// The character offset it sits in front of. `text.len()` is the end of
    /// the buffer; the offset of a newline is the end of that line.
    pub at: usize,
    pub text: Arc<str>,
    pub face: Face,
    /// Higher is drawn first where two sit at the same offset; equal
    /// priorities are drawn oldest first, so the later one follows the
    /// earlier. The same rule overlays use, for the same reason -- a producer
    /// should be able to say its text comes before somebody else's.
    pub priority: i32,
    /// What made it. A producer replaces its own batch by category rather than
    /// by remembering every handle it was given, which is what a language
    /// server answering again wants, and what still works after the module
    /// has been reloaded.
    pub category: Arc<str>,
    /// Stable for the life of the entry, so Lisp can hold one.
    pub id: usize,
}

/// A buffer's virtual text, kept in the order it is drawn.
///
/// Sorted by offset, then by descending priority, then by id. That is the
/// order a row is composed in, so composing is a walk rather than a sort per
/// frame -- and a frame does this for every visible line of every window.
#[derive(Clone, Debug, Default)]
pub struct VirtualTextTable {
    entries: Vec<VirtualText>,
    next_id: usize,
}

impl VirtualTextTable {
    /// Add TEXT at AT, and hand back its id.
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

    /// Everything at AT, in the order it is drawn.
    pub fn at(&self, at: usize) -> impl Iterator<Item = &VirtualText> {
        let first = self.entries.partition_point(|entry| entry.at < at);
        self.entries[first..]
            .iter()
            .take_while(move |entry| entry.at == at)
    }

    /// Everything from FROM up to but not including TO, in drawing order.
    ///
    /// What a row is composed from: one walk per row rather than a search per
    /// character.
    pub fn between(&self, from: usize, to: usize) -> impl Iterator<Item = &VirtualText> {
        let first = self.entries.partition_point(|entry| entry.at < from);
        self.entries[first..]
            .iter()
            .take_while(move |entry| entry.at < to)
    }

    /// Remove the entry with ID. True when there was one.
    pub fn remove(&mut self, id: usize) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != before
    }

    /// Remove everything in CATEGORY, or everything when it is `None`. Returns
    /// how many went.
    pub fn clear(&mut self, category: Option<&str>) -> usize {
        let before = self.entries.len();
        match category {
            None => self.entries.clear(),
            Some(category) => self
                .entries
                .retain(|entry| &*entry.category != category),
        }
        before - self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Move everything at or after AT along by N characters.
    ///
    /// At, not after: text inserted exactly where a hint sits pushes the hint
    /// along with the character it was sitting in front of. The alternative
    /// leaves the hint in front of whatever was just typed, which is not what
    /// it was placed in front of.
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

    /// Move everything back for the deletion of `[from, to)`.
    ///
    /// Anything inside the deleted span lands at its start -- the character it
    /// was in front of has gone, and the nearest thing to where it was is
    /// where that character used to begin.
    ///
    /// Nothing is dropped. This is the one place this table differs from the
    /// overlays', which keep only what still has width: a point has no width
    /// to lose, so there is no state in which it has stopped meaning
    /// anything. A producer whose text is now in the wrong place says so by
    /// clearing its category and placing it again, which is what a language
    /// server does on every edit anyway.
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
