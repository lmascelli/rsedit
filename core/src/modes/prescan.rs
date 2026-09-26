//! The background job that walks a buffer's balanced-expression state forward,
//! and the rule for accepting what it produces.
//!
//! # Why the worker does this and the motions do not
//!
//! Every scan used to start at the top of the buffer, because the top is the
//! only offset whose state the scanner knows without being told. Telling it
//! means having somewhere to keep what it was told, and filling that store from
//! the motions themselves would mean taking the buffer's *write* lock on the
//! keystroke path, in the middle of evaluating Lisp, to make a cache work.
//!
//! So the shape is the highlighter's, for the highlighter's reasons: a job that
//! wakes on a timer and does a bounded amount of work, and readers that take
//! whatever has been worked out and scan the rest themselves. A reader is never
//! wrong for want of a checkpoint; it is only slower, which is what it was
//! before this existed.
//!
//! # Where this departs from the highlighter, and why
//!
//! The highlighter copies its lines out and lexes with no lock held. This scans
//! with the buffer's *read* lock held, which is a real difference and worth
//! stating rather than glossing.
//!
//! It is not a choice about tidiness. The scan works in absolute offsets and a
//! snapshot records them, so it cannot be handed a detached list of lines the
//! way `highlight_line` can; the only way to hand it the text is to hand it the
//! whole buffer, and [`crate::buffer::gap_buffer::GapBuffer`] deliberately
//! refuses to be cloned. What makes it safe instead is the size of the work: a
//! turn is [`LINES_PER_TURN`] lines of a character-at-a-time lex with no
//! allocation and no regex, which is microseconds, and a read lock blocks only
//! writers -- an edit arriving in that window, not a frame being drawn.
//!
//! # The rule that does not change
//!
//! **Never store a result for a version that has moved on** -- and here, nor
//! for a *mode* that has. The lock is dropped between the scanning and the
//! storing, so the text may have been edited or the buffer put into another
//! mode in between. A checkpoint is a state reached under one syntax table;
//! stored under another it is not a shortcut but a wrong answer with a
//! plausible shape.
use crate::{
    BufferTrait, EditorState,
    buffer::scan::checkpoint_line,
    buffer::syntax::LINES_PER_TURN,
    modes::{
        SyntaxTable,
        sexp::{Resume, Scan},
    },
    task::ScheduledTask,
};

/// Walks whichever buffer's scan has fallen behind, a chunk at a time, forever.
pub struct Prescanner;

impl<B: BufferTrait> ScheduledTask<B> for Prescanner {
    fn execute(&mut self, state: &EditorState<B>) -> bool {
        state.prescan_one_turn();
        // Never finishes, for the reason the highlighter never does.
        true
    }
}

/// One turn's work for one buffer: which checkpoints, of what, under what.
///
/// Everything except the text, which is read under the lock when the turn runs
/// -- see the module header.
pub(crate) struct PrescanTurn {
    pub(crate) buffer: String,
    pub(crate) version: u64,
    pub(crate) mode: String,
    pub(crate) table: SyntaxTable,
    /// The first checkpoint this turn computes.
    pub(crate) first_index: usize,
    /// The checkpoint before it, which this one carries on from.
    pub(crate) from: Option<Resume>,
    /// The line this turn stops at, whatever it has reached.
    pub(crate) last_line: usize,
}

/// Scan `text` far enough to take the checkpoints this turn asked for.
///
/// A free function over the text so that the lock, and the decision about how
/// long to hold it, stay at the call site where they can be seen.
pub(crate) fn checkpoints_of<B: BufferTrait>(turn: &PrescanTurn, text: &B) -> Vec<(usize, Resume)> {
    // `usize::MAX` as the position this scan will be asked about: the guard in
    // `Scan::begin` exists to refuse a snapshot from *beyond* the question, and
    // this scan's question is the whole rest of the file.
    let mut scan = Scan::begin(text, &turn.table, usize::MAX, turn.from.as_ref(), 1);
    let mut out = Vec::new();
    let mut index = turn.first_index;
    loop {
        let line = checkpoint_line(index);
        if line >= turn.last_line {
            break;
        }
        let target = text.cursor_2d_to_1d(line, 0);
        scan.run_to(target);
        if scan.offset() < target {
            // The text ran out before the line asked for. Nothing to record
            // here, and nothing further to try.
            break;
        }
        out.push((index, scan.snapshot()));
        index += 1;
    }
    out
}

impl<B: BufferTrait> EditorState<B> {
    /// Advance one buffer's scan by at most [`LINES_PER_TURN`] lines.
    pub(crate) fn prescan_one_turn(&self) {
        let Some(turn) = self.next_prescan_turn() else {
            return;
        };
        let Some(scanned) = self.run_prescan(&turn) else {
            return;
        };
        self.store_prescan(&turn, scanned);
    }

    /// Run a turn: take the read lock, scan, give it back.
    ///
    /// Answers nothing when the buffer has moved on since the turn was planned,
    /// so that work is not done against text nobody will accept.
    pub(crate) fn run_prescan(&self, turn: &PrescanTurn) -> Option<Vec<(usize, Resume)>> {
        self.with_buffer(&turn.buffer, |buf| {
            if buf.version != turn.version || buf.current_mode != turn.mode {
                return None;
            }
            Some(checkpoints_of(turn, &buf.text))
        })?
    }

    /// Store what a turn produced, if the buffer still looks the way it did.
    ///
    /// Separated from the scanning above so the refusal can be tested on its
    /// own: the rule only matters when the buffer changes *between* the two,
    /// which a test inside one call could not arrange.
    pub(crate) fn store_prescan(&self, turn: &PrescanTurn, scanned: Vec<(usize, Resume)>) {
        self.with_buffer_mut(&turn.buffer, |buf| {
            // Checked again here: the lock was released between the scanning
            // and this, and the text may have been edited or the buffer put
            // into another mode in that window.
            if buf.version != turn.version || buf.current_mode != turn.mode {
                return;
            }
            if buf.scan.version() != turn.version || buf.scan.mode() != turn.mode {
                // The cache describes something else. This turn started from
                // index 0 -- `frontier_for` says so in the same case -- so
                // clearing it is what makes the indices below mean what the
                // turn thought they meant.
                buf.scan.reset(turn.version, &turn.mode);
            }
            for (index, resume) in scanned {
                if buf.scan.record(index, resume).is_err() {
                    // Somebody else advanced the cache past this checkpoint.
                    // Whatever they wrote is at least as fresh, so it stands.
                    break;
                }
            }
        });
    }

    /// The next chunk of work, or nothing when every buffer is up to date.
    fn next_prescan_turn(&self) -> Option<PrescanTurn> {
        // The focused buffer first: it is the one whose motions somebody is
        // waiting on.
        for name in self.buffer_names_focused_first() {
            if let Some(turn) = self.prescan_turn_for(&name) {
                return Some(turn);
            }
        }
        None
    }

    /// One buffer's next chunk, if it has one.
    pub(crate) fn prescan_turn_for(&self, name: &str) -> Option<PrescanTurn> {
        let mode = self.with_buffer(name, |buf| buf.current_mode.clone())?;
        // The table comes from the mode registry, a different lock, so the
        // buffer's is released before this is asked for.
        let table = self.syntax_table(&mode);

        self.with_buffer(name, |buf| {
            // Re-read rather than carried over from above: the lock was released in
            // between and the buffer may have been put into another mode.
            if buf.current_mode != mode {
                return None;
            }
            let line_count = buf.text.line_count();
            let (first_index, from) = buf.scan.frontier_for(&mode, buf.version);
            if checkpoint_line(first_index) >= line_count {
                return None;
            }
            let start_line = match first_index.checked_sub(1) {
                None => 0,
                Some(previous) => checkpoint_line(previous),
            };
            Some(PrescanTurn {
                buffer: name.to_string(),
                version: buf.version,
                mode,
                table,
                first_index,
                from: from.cloned(),
                last_line: (start_line + LINES_PER_TURN).min(line_count),
            })
        })?
    }
}
