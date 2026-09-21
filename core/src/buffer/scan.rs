//! What a buffer remembers about reading its own text as balanced expressions.
//!
//! # Why this is a second cache and not the colouring one
//!
//! Two lexers run over the same text and they are not the same lexer. The
//! *grammar* decides what the text should look like and its state is a stack of
//! open regions; the *syntax table* decides what the text means -- where the
//! strings, comments and lists are -- and its state is
//! [`crate::modes::sexp::Resume`]. `forward-sexp`, the indenter, `syntax-ppss`
//! and electric-pair all read the second one, and none of them could be
//! answered from the first. So the colouring cache next door caches colouring,
//! and this caches meaning.
//!
//! # Why checkpoints rather than a state per line
//!
//! The colouring cache keeps a state for every line and interns them, because
//! almost every line of almost every file is entered in the same empty state. A
//! scan state is not like that: it carries the *positions* of the lists it is
//! inside, so two lines at the same depth still differ, and interning would
//! dedupe nothing.
//!
//! So this keeps one snapshot every [`LINES_PER_CHECKPOINT`] lines. Memory is a
//! sixty-fourth of a state-per-line table, and the cost of an answer is bounded
//! by the distance from a checkpoint rather than by the distance from the top
//! of the file -- which was the whole point.
//!
//! # Why a reader never writes here
//!
//! Filling the cache is the background worker's job (see
//! [`crate::modes::prescan`]), and the motions only ever read it. A reader that
//! filled it as it went would need the buffer's write lock -- on the keystroke
//! path, in the middle of evaluating Lisp -- to make a *performance* cache
//! work. Handing the writing to the worker keeps every reader on the read lock
//! it already holds, and costs only that a file just opened is scanned from the
//! top until the worker has walked it.
//!
//! # Why an edit does not throw it all away
//!
//! An edit on line L cannot change the text above L, so every checkpoint that
//! describes entering a line at or before L still describes it exactly.
//! `invalidate_from` keeps those and drops the rest. That is the whole
//! bookkeeping -- there is none of the line-shift accounting the colouring
//! cache's header says is missing, because a checkpoint is identified by the
//! line it enters and the lines it is kept for are the ones that did not move.
use crate::modes::sexp::Resume;

/// How many lines apart the snapshots are.
///
/// The trade is memory against the length of the fixup: a query resuming from
/// the nearest checkpoint scans at most this many lines to reach its position.
/// Sixty-four lines of scanning is a few microseconds and a snapshot every
/// sixty-four lines is a few kilobytes for a large file.
pub const LINES_PER_CHECKPOINT: usize = 64;

/// The line a checkpoint enters, by index.
///
/// Index 0 is line [`LINES_PER_CHECKPOINT`], not line 0: the state entering
/// line 0 is the base state, which needs no remembering.
pub fn checkpoint_line(index: usize) -> usize {
    (index + 1) * LINES_PER_CHECKPOINT
}

/// A buffer's scan checkpoints, as far as they have been computed.
#[derive(Clone, Debug, Default)]
pub struct ScanCache {
    /// The buffer version these were computed from.
    version: u64,
    /// The major mode they were computed under.
    ///
    /// Kept because the table comes from the mode, and a checkpoint taken with
    /// one table is not a shortcut under another -- it is a wrong answer that
    /// looks like a right one. Changing a buffer's mode does not touch its
    /// text, so nothing else here would notice.
    mode: String,
    /// `points[i]` enters line `checkpoint_line(i)`.
    points: Vec<Resume>,
    /// How many of `points` describe the current version of the text.
    valid: usize,
}

impl ScanCache {
    pub fn version(&self) -> u64 {
        self.version
    }

    /// How many checkpoints currently describe the text.
    pub fn checkpoints(&self) -> usize {
        self.valid
    }

    /// How far down the buffer the checkpoints reach.
    pub fn valid_to(&self) -> usize {
        match self.valid.checked_sub(1) {
            None => 0,
            Some(last) => checkpoint_line(last),
        }
    }

    /// Forget everything: another version entirely, or another mode.
    pub fn reset(&mut self, version: u64, mode: &str) {
        self.version = version;
        mode.clone_into(&mut self.mode);
        self.points.clear();
        self.valid = 0;
    }

    /// An edit landed on LINE: keep the checkpoints above it, drop the rest.
    ///
    /// "Above it" includes a checkpoint entering LINE itself. It was computed
    /// from the text strictly before that line, and an edit *on* the line did
    /// not touch any of it.
    pub fn invalidate_from(&mut self, version: u64, line: usize) {
        self.version = version;
        // Keep every `i` with `checkpoint_line(i) <= line`, which is
        // `(i + 1) * N <= line`, which is `i < line / N`.
        self.valid = self.valid.min(line / LINES_PER_CHECKPOINT);
        self.points.truncate(self.valid);
    }

    /// Record the checkpoint at INDEX.
    ///
    /// Only the next one is accepted. Each is computed by carrying on from the
    /// one before it, so a checkpoint written out of order would be a state
    /// reached from somewhere nobody can name.
    pub fn record(&mut self, index: usize, resume: Resume) -> Result<(), ()> {
        if index != self.valid {
            return Err(());
        }
        if self.points.len() <= index {
            self.points.push(resume);
        } else {
            self.points[index] = resume;
        }
        self.valid = index + 1;
        Ok(())
    }

    /// The checkpoint a scan that will be asked about LINE may carry on from,
    /// when the cache describes this exact text in this exact mode.
    ///
    /// The version and the mode are checked here rather than trusted from the
    /// caller because this is the one place a wrong answer could get in, and
    /// the cost of refusing is a full scan -- the behaviour that existed before
    /// there was a cache at all.
    pub fn resume_for(&self, mode: &str, version: u64, line: usize) -> Option<&Resume> {
        if version != self.version || mode != self.mode {
            return None;
        }
        // The checkpoint entering the largest multiple of the interval at or
        // below LINE, and no further down than what has been computed.
        let index = (line / LINES_PER_CHECKPOINT).min(self.valid);
        self.points.get(index.checked_sub(1)?)
    }

    /// The major mode these were computed under.
    pub fn mode(&self) -> &str {
        &self.mode
    }

    /// What the next turn of the worker should carry on from: the index to
    /// compute, and the snapshot to start it from.
    ///
    /// Starts over when the cache describes another version or another mode,
    /// rather than reporting a frontier reached under a table that is no longer
    /// in force. The worker's store step resets the cache in the same case, so
    /// the two agree about what index 0 means.
    pub fn frontier_for(&self, mode: &str, version: u64) -> (usize, Option<&Resume>) {
        if version != self.version || mode != self.mode {
            return (0, None);
        }
        (
            self.valid,
            self.valid
                .checked_sub(1)
                .and_then(|last| self.points.get(last)),
        )
    }
}
