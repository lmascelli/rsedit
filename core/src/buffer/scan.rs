//! The scan cache: checkpoints of the syntax-table scan, which `forward-sexp`,
//! the indenter, `syntax-ppss` and electric-pair carry on from. One every
//! [`LINES_PER_CHECKPOINT`] lines, written by the prescanner alone; an edit
//! keeps the checkpoints above it.
//!
//! Design: `doc/architettura-editor.typ, <cache-testo>`.
use crate::modes::sexp::Resume;

/// How many lines apart the snapshots are.
///
/// The trade is memory against the length of the fixup: a query resuming from
/// the nearest checkpoint scans at most this many lines to reach its position.
/// Sixty-four lines of scanning is a few microseconds and a snapshot every
/// sixty-four lines is a few kilobytes for a large file.
pub const LINES_PER_CHECKPOINT: usize = 64;

/// Returns the line the checkpoint at INDEX enters.
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
    /// The major mode they were computed under, whose syntax table they were
    /// taken with: under another table a checkpoint is a wrong answer that
    /// looks like a right one. Changing a buffer's mode does not touch its
    /// text, so the version would not notice.
    mode: String,
    /// The edition of the syntax table they were computed with: see
    /// [`crate::modes::SyntaxTable::edition`]. A mode's table can change while
    /// the buffer stays in the mode.
    table: u64,
    /// `points[i]` enters line `checkpoint_line(i)`.
    points: Vec<Resume>,
    /// How many of `points` describe the current version of the text.
    valid: usize,
}

impl ScanCache {
    /// Returns the buffer version these were computed from.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Returns the edition of the syntax table these were computed with.
    pub fn table(&self) -> u64 {
        self.table
    }

    /// Returns how many checkpoints currently describe the text.
    pub fn checkpoints(&self) -> usize {
        self.valid
    }

    /// Returns how far down the buffer the checkpoints reach.
    pub fn valid_to(&self) -> usize {
        match self.valid.checked_sub(1) {
            None => 0,
            Some(last) => checkpoint_line(last),
        }
    }

    /// Forgets everything, for another version entirely, another mode or
    /// another edition of its syntax table.
    pub fn reset(&mut self, version: u64, mode: &str, table: u64) {
        self.version = version;
        mode.clone_into(&mut self.mode);
        self.table = table;
        self.points.clear();
        self.valid = 0;
    }

    /// Keeps the checkpoints above LINE, where an edit landed, and drops the
    /// rest.
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

    /// Records the checkpoint at INDEX.
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

    /// Returns the checkpoint a scan that will be asked about LINE may carry
    /// on from, when the cache describes this exact text, in this exact mode,
    /// scanned with this edition of its table.
    ///
    /// The three are checked here rather than trusted from the caller: this is
    /// the one place a wrong answer could get in, and refusing costs a full
    /// scan.
    pub fn resume_for(&self, mode: &str, version: u64, table: u64, line: usize) -> Option<&Resume> {
        if version != self.version || mode != self.mode || table != self.table {
            return None;
        }
        // The checkpoint entering the largest multiple of the interval at or
        // below LINE, and no further down than what has been computed.
        let index = (line / LINES_PER_CHECKPOINT).min(self.valid);
        self.points.get(index.checked_sub(1)?)
    }

    /// Returns the major mode these were computed under.
    pub fn mode(&self) -> &str {
        &self.mode
    }

    /// Returns what the prescanner's next turn carries on from: the index to
    /// compute, and the snapshot to start it from.
    ///
    /// Starts over when the cache describes another version, another mode or
    /// another edition of the table, rather than reporting a frontier reached
    /// under a table that is no longer in force. The prescanner's store step
    /// resets the cache in the same case, so the two agree about what index 0
    /// means.
    pub fn frontier_for(&self, mode: &str, version: u64, table: u64) -> (usize, Option<&Resume>) {
        if version != self.version || mode != self.mode || table != self.table {
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
