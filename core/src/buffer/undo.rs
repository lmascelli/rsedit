//! Per-buffer undo history.
//!
//! # What is recorded
//!
//! Changes, not snapshots. Each entry says what an edit did to the text, so
//! the memory cost is proportional to the text *changed* rather than to the
//! size of the document: typing a hundred characters into a hundred-thousand
//! line file costs a hundred characters, not a copy of the file.
//!
//! # How undo and redo are the same operation
//!
//! Applying the inverse of a group turns the text back into what it was, and
//! produces the group that would undo *that*. So redo is not a second
//! mechanism -- it is the same [`UndoHistory::apply_inverse`] run against the
//! other stack. That is also what solves the awkward part of redoing an
//! insertion: the text to re-insert is captured at the moment undo removes it,
//! rather than being stored up front against the possibility of a redo that
//! may never come.
use crate::buffer::BufferTrait;

/// One edit, described by what it did.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// `len` characters were inserted at `at`. Undoing deletes them.
    Inserted { at: usize, len: usize },
    /// `text` was deleted from `at`. Undoing puts it back.
    Deleted { at: usize, text: String },
}

impl Change {
    fn weight(&self) -> usize {
        match self {
            Change::Inserted { .. } => 0,
            Change::Deleted { text, .. } => text.len(),
        }
    }
}

/// The changes one command made, undone and redone as a unit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub changes: Vec<Change>,
    /// Where point was before the group's first change, so undo puts the
    /// cursor back where the edit happened instead of wherever it drifted to.
    pub point: usize,
}

/// How much deleted text to keep before dropping the oldest history.
///
/// Only deletions carry text, so this bounds the one part of the history that
/// can grow with the size of the document rather than with the number of edits.
pub const DEFAULT_UNDO_LIMIT: usize = 1 << 20;

#[derive(Debug)]
pub struct UndoHistory {
    /// Completed groups, most recent last.
    done: Vec<Group>,
    /// Groups that have been undone and can be redone, most recent last.
    undone: Vec<Group>,
    /// The group currently being accumulated, if a command is mid-edit.
    open: Option<Group>,
    /// Bytes of deleted text held in `done`, against [`Self::limit`].
    bytes: usize,
    limit: usize,
}

impl Default for UndoHistory {
    fn default() -> Self {
        Self {
            done: Vec::new(),
            undone: Vec::new(),
            open: None,
            bytes: 0,
            limit: DEFAULT_UNDO_LIMIT,
        }
    }
}

impl UndoHistory {
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        self.trim();
    }

    /// Note that `len` characters were inserted at `at`.
    pub fn record_insert(&mut self, at: usize, len: usize, point: usize) {
        self.push(Change::Inserted { at, len }, point);
    }

    /// Note that `text` was deleted from `at`.
    pub fn record_delete(&mut self, at: usize, text: String, point: usize) {
        self.push(Change::Deleted { at, text }, point);
    }

    fn push(&mut self, change: Change, point: usize) {
        // A fresh edit makes the redo branch unreachable. This is the one
        // cost of a plain two-stack model over a tree, and it is the behaviour
        // people expect from an editor.
        self.undone.clear();
        self.bytes += change.weight();
        let group = self.open.get_or_insert_with(|| Group {
            changes: Vec::new(),
            point,
        });
        // Typing arrives one character at a time, and a hundred adjacent
        // one-character insertions describe exactly the same edit as one
        // hundred-character insertion. Merging them here keeps a run of typing
        // at one entry rather than one per keystroke -- which matters on the
        // keystroke path, where this runs between the key and the screen.
        if let Change::Inserted { at, len } = change
            && let Some(Change::Inserted {
                at: prev_at,
                len: prev_len,
            }) = group.changes.last_mut()
            && *prev_at + *prev_len == at
        {
            *prev_len += len;
            return;
        }
        group.changes.push(change);
    }

    /// Close the group being accumulated, if any. Called between commands --
    /// this is what makes one command undo as one unit.
    pub fn boundary(&mut self) {
        if let Some(group) = self.open.take()
            && !group.changes.is_empty()
        {
            self.done.push(group);
            self.trim();
        }
    }

    /// How many characters the open group has inserted so far, for deciding
    /// whether a run of self-inserts has grown long enough to break up.
    pub fn open_insert_len(&self) -> usize {
        self.open
            .as_ref()
            .map(|group| {
                group
                    .changes
                    .iter()
                    .map(|change| match change {
                        Change::Inserted { len, .. } => *len,
                        Change::Deleted { .. } => 0,
                    })
                    .sum()
            })
            .unwrap_or(0)
    }

    /// Undo the most recent group. Returns where point should go, or `None`
    /// when there is nothing left to undo.
    pub fn undo<B: BufferTrait>(&mut self, text: &mut B) -> Option<usize> {
        self.boundary();
        let group = self.done.pop()?;
        self.bytes = self
            .bytes
            .saturating_sub(group.changes.iter().map(Change::weight).sum());
        let point = group.point;
        let inverse = self.apply_inverse(text, group);
        self.undone.push(inverse);
        Some(point)
    }

    /// Redo the most recently undone group.
    pub fn redo<B: BufferTrait>(&mut self, text: &mut B) -> Option<usize> {
        let group = self.undone.pop()?;
        let point = group.point;
        let inverse = self.apply_inverse(text, group);
        self.bytes += inverse.changes.iter().map(Change::weight).sum::<usize>();
        self.done.push(inverse);
        Some(point)
    }

    /// Apply the inverse of `group` to the text, and return the group that
    /// would in turn undo what was just done.
    ///
    /// Changes are inverted in reverse order: a group's later edits are
    /// described in terms of the text its earlier edits produced, so they have
    /// to come apart in the opposite order to the one they went together in.
    ///
    /// Replaying history cannot record itself as new history, and not because
    /// a flag says so: this reaches for [`apply_delete`] and [`apply_insert`]
    /// directly, below the layer that writes the history. There is no switch
    /// to forget to set.
    fn apply_inverse<B: BufferTrait>(&mut self, text: &mut B, group: Group) -> Group {
        let mut inverse = Vec::with_capacity(group.changes.len());

        for change in group.changes.iter().rev() {
            match change {
                Change::Inserted { at, len } => {
                    // Capture the text as it is removed -- this is what makes
                    // the corresponding redo possible without having stored a
                    // copy in advance.
                    let removed: String = (*at..at + len).filter_map(|i| text.at(i)).collect();
                    apply_delete(text, *at, at + len);
                    inverse.push(Change::Deleted {
                        at: *at,
                        text: removed,
                    });
                }
                Change::Deleted { at, text: content } => {
                    apply_insert(text, *at, content);
                    inverse.push(Change::Inserted {
                        at: *at,
                        len: content.chars().count(),
                    });
                }
            }
        }

        Group {
            changes: inverse,
            // Where point was before this application, so undoing the undo
            // returns to where the user was standing.
            point: group
                .changes
                .first()
                .map(|c| match c {
                    Change::Inserted { at, .. } | Change::Deleted { at, .. } => *at,
                })
                .unwrap_or(group.point),
        }
    }

    /// Drop the oldest groups until the retained deleted text is under the
    /// limit. Always keeps at least one group, so a single enormous deletion
    /// stays undoable rather than being discarded for being too big.
    fn trim(&mut self) {
        while self.bytes > self.limit && self.done.len() > 1 {
            let dropped = self.done.remove(0);
            self.bytes = self
                .bytes
                .saturating_sub(dropped.changes.iter().map(Change::weight).sum());
        }
    }
}

/// Remove `[from, to)` from the text, leaving point at `from`.
///
/// The unrecorded application step: history is written by the editing layer in
/// [`crate::primitives::edits`], which notes what an edit is about to do and
/// then calls this. Undo and redo reuse it directly, since replaying history
/// must not record itself as new history.
pub(crate) fn apply_delete<B: BufferTrait>(text: &mut B, from: usize, to: usize) {
    let end = to.min(text.len());
    if from >= end {
        return;
    }
    seek(text, end);
    for _ in 0..(end - from) {
        text.delete();
    }
}

/// Insert `content` at offset `at`, leaving point after it.
pub(crate) fn apply_insert<B: BufferTrait>(text: &mut B, at: usize, content: &str) {
    seek(text, at.min(text.len()));
    for c in content.chars() {
        text.insert(c);
    }
}

/// Put point at OFFSET, doing nothing if it is already there.
///
/// The guard is not a micro-optimisation: an edit at point is the common case
/// -- it is what typing is -- and seeking to a place you are already standing
/// costs an offset-to-line-and-column conversion and a gap move, on the one
/// code path a user feels between pressing a key and seeing the character.
fn seek<B: BufferTrait>(text: &mut B, offset: usize) {
    if text.cursor_pos_1d() == offset {
        return;
    }
    let (line, col) = text.cursor_1d_to_2d(offset);
    text.cursor_move(line, col);
}
