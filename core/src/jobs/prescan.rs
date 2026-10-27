//! The prescanner: the job that walks each buffer's scan of expressions
//! forward, a chunk of lines at a step, and keeps a snapshot every few lines
//! for the motions to carry on from. It scans under the buffer's read lock,
//! stores a turn only if the buffer is still the text and mode it read, and
//! stamps the snapshots with the table they were taken with.
//!
//! Design: `doc/architettura-editor.typ, <riprendere>`.
use crate::{
    BufferTrait, EditorState,
    buffer::scan::checkpoint_line,
    buffer::syntax::LINES_PER_TURN,
    editor::{BUFFER_CHANGED, SYNTAX_CHANGED},
    modes::{
        SyntaxTable,
        sexp::{Resume, Scan},
    },
    work::{Class, Job, JobError, Spec, Step, StepContext},
};

/// Walks whichever buffer's scan has fallen behind, a chunk at a time, for as
/// long as one is behind -- then waits for the next change.
pub struct Prescanner;

impl Prescanner {
    /// Returns how it is scheduled: in the high class, as the highlighter,
    /// since a motion waits on what it works out, and woken by the same two
    /// topics: an edit moves the text under the checkpoints, and a change of
    /// syntax table the lexer that took them.
    pub(crate) fn spec() -> Spec {
        Spec::new("prescanner")
            .class(Class::High)
            .unit("lines")
            .on(BUFFER_CHANGED)
            .on(SYNTAX_CHANGED)
    }
}

impl<B: BufferTrait> Job<EditorState<B>> for Prescanner {
    fn step(&mut self, cx: &mut StepContext<'_, EditorState<B>>) -> Result<Step, JobError> {
        match cx.state.prescan_one_turn() {
            Some((buffer, lines)) => {
                cx.worked(lines as u64, &buffer);
                Ok(Step::Continue)
            }
            None => Ok(Step::Wait),
        }
    }
}

/// One turn's work for one buffer: which checkpoints, of what, under what.
/// Everything but the text, which is read under the buffer's read lock when
/// the turn runs.
pub(crate) struct PrescanTurn {
    pub(crate) buffer: String,
    pub(crate) version: u64,
    pub(crate) mode: String,
    pub(crate) table: SyntaxTable,
    /// The first checkpoint this turn computes.
    pub(crate) first_index: usize,
    /// The checkpoint before it, which this one carries on from.
    pub(crate) from: Option<Resume>,
    /// The line it carries on from: where that checkpoint is, or the top.
    pub(crate) first_line: usize,
    /// The line this turn stops at, whatever it has reached.
    pub(crate) last_line: usize,
}

/// Returns the checkpoints TURN asks for, by index, scanning TEXT as far as
/// they need. A function of the text alone, so that the lock it is read under
/// is taken where it can be seen, in `run_prescan`.
pub(crate) fn checkpoints_of<B: BufferTrait>(turn: &PrescanTurn, text: &B) -> Vec<(usize, Resume)> {
    // `usize::MAX` as the position this scan will be asked about: `Scan::begin`
    // refuses a snapshot from beyond the question, and this scan's question is
    // the whole rest of the file.
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
            // The text ran out before the line asked for: nothing to record,
            // and nothing further to try.
            break;
        }
        out.push((index, scan.snapshot()));
        index += 1;
    }
    out
}

impl<B: BufferTrait> EditorState<B> {
    /// Advances one buffer's scan by at most [`LINES_PER_TURN`] lines, and
    /// returns which buffer and how many lines the turn covered, or `None`
    /// when every buffer is up to date.
    pub(crate) fn prescan_one_turn(&self) -> Option<(String, usize)> {
        let turn = self.next_prescan_turn()?;
        // A buffer that has moved on since the turn was planned is not
        // scanned: the next turn plans against what it is now.
        if let Some(scanned) = self.run_prescan(&turn) {
            self.store_prescan(&turn, scanned);
        }
        Some((turn.buffer, turn.last_line - turn.first_line))
    }

    /// Runs TURN under the buffer's read lock, and returns the checkpoints, or
    /// `None` when the buffer has moved on since the turn was planned.
    pub(crate) fn run_prescan(&self, turn: &PrescanTurn) -> Option<Vec<(usize, Resume)>> {
        self.with_buffer(&turn.buffer, |buf| {
            if buf.version != turn.version || buf.current_mode != turn.mode {
                return None;
            }
            Some(checkpoints_of(turn, &buf.text))
        })?
    }

    /// Stores what TURN produced, if the buffer is still the text and mode the
    /// turn read. Apart from the scanning, so that a test can change the
    /// buffer between the two.
    pub(crate) fn store_prescan(&self, turn: &PrescanTurn, scanned: Vec<(usize, Resume)>) {
        self.with_buffer_mut(&turn.buffer, |buf| {
            // Checked again: the lock was given back between the scanning and
            // this, and the text or the mode may have changed.
            if buf.version != turn.version || buf.current_mode != turn.mode {
                return;
            }
            let table = turn.table.edition();
            if buf.scan.version() != turn.version
                || buf.scan.mode() != turn.mode
                || buf.scan.table() != table
            {
                // The cache describes something else. This turn started from
                // index 0, as `frontier_for` says in that case, so clearing it
                // makes the indices below mean what the turn meant.
                buf.scan.reset(turn.version, &turn.mode, table);
            }
            for (index, resume) in scanned {
                if buf.scan.record(index, resume).is_err() {
                    // Something else advanced the cache past this checkpoint,
                    // and what it wrote is at least as fresh, so it stands.
                    break;
                }
            }
        });
    }

    /// Returns the next chunk of work, the focused buffer's first, or `None`
    /// when every buffer is up to date.
    fn next_prescan_turn(&self) -> Option<PrescanTurn> {
        for name in self.buffer_names_focused_first() {
            if let Some(turn) = self.prescan_turn_for(&name) {
                return Some(turn);
            }
        }
        None
    }

    /// Returns NAME's next chunk of work, if it has one.
    pub(crate) fn prescan_turn_for(&self, name: &str) -> Option<PrescanTurn> {
        let mode = self.with_buffer(name, |buf| buf.current_mode.clone())?;
        // The table comes from the mode registry, another lock, so the
        // buffer's is given back first.
        let table = self.syntax_table(&mode);

        self.with_buffer(name, |buf| {
            // Read again: the lock was given back in between, and the buffer
            // may be in another mode now.
            if buf.current_mode != mode {
                return None;
            }
            let line_count = buf.text.line_count();
            let (first_index, from) = buf.scan.frontier_for(&mode, buf.version, table.edition());
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
                first_index,
                from: from.cloned(),
                first_line: start_line,
                last_line: (start_line + LINES_PER_TURN).min(line_count),
                table,
            })
        })?
    }
}
