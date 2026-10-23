//! A buffer's undo history: the changes each command made, as groups, undone
//! and redone by applying their inverse. The command an edit belongs to is the
//! stamp its thread carries, and a group closes at the first edit made under
//! another stamp.
//!
//! Design: `doc/architettura-editor.typ, <undo>`.
use crate::buffer::BufferTrait;
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Which command an edit belongs to
// ---------------------------------------------------------------------------
//
// Undo works in commands, not in edits: one press of C-k undoes as one step
// even though it deletes many characters. Each command stamps a token into a
// thread-local; a group remembers the token it was opened under; and the
// history closes the group lazily, at the first edit that arrives under a
// different token. A command that edits nothing costs nothing.

/// How many characters a run of `self-insert` amalgamates before the next one
/// starts a fresh undo group: a word or two, as in Emacs. Without a cap, a
/// paragraph typed without pausing would undo in one step.
pub const AMALGAMATION_LIMIT: usize = 20;

/// The command an edit is being made by.
#[derive(Clone, Copy)]
struct Command {
    /// Tells one command from the next. Drawn from a global counter rather
    /// than a per-thread one so that two threads editing the same buffer
    /// cannot collide on a value and have their edits silently merged.
    epoch: u64,
    /// Whether a run of this command amalgamates -- true only for ordinary
    /// typing.
    amalgamating_kind: bool,
    /// Whether this command should join the group the previous one opened.
    joins_previous: bool,
}

/// What edits made outside any command belong to: everything evaluated from
/// Lisp without a key being pressed shares one group, which is what makes a
/// function that edits several times undo as a unit.
const NO_COMMAND: Command = Command {
    epoch: 0,
    amalgamating_kind: false,
    joins_previous: false,
};

/// The epoch the next command is given. From 1: 0 is [`NO_COMMAND`]'s.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// The command running on *this* thread. Thread-local because each thread
    /// that edits has its own -- the WorkManager's executor and a `(spawn ...)`
    /// thread apart from the command thread -- and because reading it must
    /// cost nothing.
    static CURRENT: Cell<Command> = const { Cell::new(NO_COMMAND) };

    /// How many callers are holding the stamp above still: see [`OneGroup`].
    static PINNED: Cell<usize> = const { Cell::new(0) };
}

/// Holds the current command's stamp still for as long as it is alive, so
/// that every edit made under it lands in one undo group however many
/// commands make them: a keyboard macro's replay undoes in one step.
///
/// RAII rather than a pair of calls, so a replay that ends by propagating an
/// error cannot leave the stamp pinned for everything typed afterwards.
/// Nesting is counted, so a macro that calls a macro is still one group.
pub(crate) struct OneGroup;

impl OneGroup {
    /// Starts holding the stamp still, until the value is dropped.
    pub(crate) fn begin() -> Self {
        PINNED.with(|pinned| pinned.set(pinned.get() + 1));
        OneGroup
    }
}

impl Drop for OneGroup {
    fn drop(&mut self) {
        PINNED.with(|pinned| pinned.set(pinned.get().saturating_sub(1)));
    }
}

/// Announces that a new command is about to run on this thread.
///
/// AMALGAMATING_KIND says whether a run of this command should undo as one
/// step; only ordinary typing does. Whether it *actually* joins the previous
/// group also depends on what the previous command was, which is why that is
/// decided here rather than at the call site: this is the only place that sees
/// both.
pub(crate) fn begin_command(amalgamating_kind: bool) {
    // Somebody is holding the stamp still -- see [`OneGroup`]. The commands
    // running under it are a macro's, and they are one thing the user did.
    if PINNED.with(|pinned| pinned.get()) > 0 {
        return;
    }
    CURRENT.with(|current| {
        let previous = current.get();
        current.set(Command {
            epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
            amalgamating_kind,
            joins_previous: amalgamating_kind && previous.amalgamating_kind,
        });
    });
}

/// Returns the command running on this thread.
fn current_command() -> Command {
    CURRENT.with(|current| current.get())
}

/// One edit, described by what it did.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// `len` characters were inserted at `at`. Undoing deletes them.
    Inserted { at: usize, len: usize },
    /// `text` was deleted from `at`. Undoing puts it back.
    Deleted { at: usize, text: String },
}

impl Change {
    /// Returns how many bytes of deleted text this change holds.
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

impl Group {
    /// Returns the earliest offset any of these changes touches.
    ///
    /// Every change, applied or inverted, leaves the text before its own
    /// offset alone, so nothing before the smallest of them moves.
    fn earliest(&self) -> usize {
        self.changes
            .iter()
            .map(|change| match change {
                Change::Inserted { at, .. } | Change::Deleted { at, .. } => *at,
            })
            .min()
            .unwrap_or(self.point)
    }
}

/// What undoing or redoing a group did, for the caller that has to tell the
/// rest of the buffer.
///
/// Undo and redo write below the two doors, or replaying history would record
/// itself as new history. So the caller moves the version and the caches on,
/// as the doors would have, and needs to know from where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    /// Where point should go.
    pub point: usize,
    /// The earliest offset the group touched. Nothing before it changed.
    pub from: usize,
}

/// How many bytes of deleted text a history keeps, unless told otherwise,
/// before dropping its oldest groups.
///
/// Only deletions carry text, so this bounds the one part of the history that
/// can grow with the size of the document rather than with the number of edits.
pub const DEFAULT_UNDO_LIMIT: usize = 1 << 20;

/// A buffer's undo history: the groups done, the groups undone, and the one
/// being accumulated.
#[derive(Debug)]
pub struct UndoHistory {
    /// Completed groups, most recent last.
    done: Vec<Group>,
    /// Groups that have been undone and can be redone, most recent last.
    undone: Vec<Group>,
    /// The group currently being accumulated, if a command is mid-edit.
    open: Option<Group>,
    /// The command [`Self::open`] was opened under, so that the first edit of
    /// the next command can close it.
    open_epoch: u64,
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
            open_epoch: NO_COMMAND.epoch,
            bytes: 0,
            limit: DEFAULT_UNDO_LIMIT,
        }
    }
}

impl UndoHistory {
    /// Returns how many bytes of deleted text this history keeps before
    /// dropping its oldest groups.
    ///
    /// A setting rather than history, which is the distinction that matters
    /// when a buffer's contents are replaced wholesale: the history goes and
    /// the limit stays.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Sets the limit, dropping the oldest groups that are now past it.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        self.trim();
    }

    /// Records that LEN characters were inserted at AT. POINT is where point
    /// was before the edit, kept when the edit opens a group.
    pub fn record_insert(&mut self, at: usize, len: usize, point: usize) {
        self.push(Change::Inserted { at, len }, point);
    }

    /// Records that TEXT was deleted from AT. POINT is as for
    /// [`Self::record_insert`].
    pub fn record_delete(&mut self, at: usize, text: String, point: usize) {
        self.push(Change::Deleted { at, text }, point);
    }

    /// Adds CHANGE to the open group, closing that first when the edit
    /// belongs to another command.
    fn push(&mut self, change: Change, point: usize) {
        // A fresh edit makes the redo branch unreachable. This is the one
        // cost of a plain two-stack model over a tree, and it is the behaviour
        // people expect from an editor.
        self.undone.clear();

        // The group belongs to the command that opened it. An edit arriving
        // under a different command closes it first -- unless the two are a
        // continuing run of typing that has not yet grown past the
        // amalgamation limit.
        let command = current_command();
        if self.open.is_some()
            && command.epoch != self.open_epoch
            && !(command.joins_previous && self.open_insert_len() < AMALGAMATION_LIMIT)
        {
            self.boundary();
        }
        self.open_epoch = command.epoch;

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

    /// Closes the group being accumulated, if any: what makes one command undo
    /// as one unit.
    pub fn boundary(&mut self) {
        if let Some(group) = self.open.take()
            && !group.changes.is_empty()
        {
            self.done.push(group);
            self.trim();
        }
    }

    /// Lets this command continue the open group without ordinary typing's
    /// cap.
    pub fn join_previous_command(&mut self) {
        self.open_epoch = current_command().epoch;
    }

    /// Returns how many characters the open group has inserted so far, for
    /// deciding whether a run of self-inserts has grown long enough to break
    /// up.
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

    // TODO(BUF-1): undo and redo write below the two doors, and their callers
    // only call `edits::changed` after them: a read-only buffer is changed,
    // and overlays and virtual text stay where the old text had them.

    /// Undoes the most recent group. Returns where point should go and where
    /// the text began to change, or `None` when there is nothing left to undo.
    pub fn undo<B: BufferTrait>(&mut self, text: &mut B) -> Option<Applied> {
        self.boundary();
        let group = self.done.pop()?;
        self.bytes = self
            .bytes
            .saturating_sub(group.changes.iter().map(Change::weight).sum());
        let applied = Applied {
            point: group.point,
            from: group.earliest(),
        };
        let inverse = self.apply_inverse(text, group);
        self.undone.push(inverse);
        Some(applied)
    }

    /// Redoes the most recently undone group, and returns what
    /// [`Self::undo`] returns.
    pub fn redo<B: BufferTrait>(&mut self, text: &mut B) -> Option<Applied> {
        let group = self.undone.pop()?;
        let applied = Applied {
            point: group.point,
            from: group.earliest(),
        };
        let inverse = self.apply_inverse(text, group);
        self.bytes += inverse.changes.iter().map(Change::weight).sum::<usize>();
        self.done.push(inverse);
        Some(applied)
    }

    /// Applies the inverse of GROUP to the text, and returns the group that
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
                    let removed = text.slice(*at, at + len);
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

    /// Drops the oldest groups until the deleted text kept is under the
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

/// Removes `[from, to)` from the text without recording it, leaving point at
/// `from`.
///
/// History is written by the editing layer in [`crate::primitives::edits`],
/// which notes what an edit is about to do and then calls this. Undo and redo
/// call it directly, since replaying history must not record itself as new
/// history.
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

/// Inserts CONTENT at offset AT without recording it, leaving point after it.
pub(crate) fn apply_insert<B: BufferTrait>(text: &mut B, at: usize, content: &str) {
    seek(text, at.min(text.len()));
    for c in content.chars() {
        text.insert(c);
    }
}

/// Puts point at OFFSET, doing nothing if it is already there.
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
