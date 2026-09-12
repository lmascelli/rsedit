//! The background job that colours buffers, and the rule for accepting what it
//! produces.
//!
//! # Why a scheduled job rather than one fired per edit
//!
//! A job per edit has to be short, or the editor stutters -- and "short" is a
//! hope rather than a property, because it depends on the file. A single job
//! that wakes on a timer and does a *bounded* amount of work each time is
//! responsive by construction: whatever it did not finish is simply what it
//! starts with next time.
//!
//! It also coalesces for free. Twenty keystrokes in a second do not queue
//! twenty jobs; they move the same boundary backwards twenty times, and the
//! next turn picks up wherever it now is.
//!
//! # The two rules that keep it honest
//!
//! **Never hold a buffer lock while lexing.** The lines are copied out under
//! the lock, the lock is dropped, the work is done, and the lock is taken again
//! to store. A worker that lexed under the lock would block every keystroke for
//! as long as it ran, which is the entire thing this design exists to avoid.
//!
//! **Never store a result for a version that has moved on.** The buffer can
//! change while the work is in flight, and spans computed from the old text
//! describe columns that no longer mean anything. The version is checked again
//! at the moment of storing, not merely at the start.
use crate::{
    BufferTrait, EditorState,
    buffer::syntax::LINES_PER_TURN,
    modes::{Grammar, SyntaxState, highlight_line},
    task::ScheduledTask,
};
use std::time::Duration;

/// How often the highlighter wakes.
///
/// Short enough that colour follows typing without a visible lag, long enough
/// that a burst of keystrokes is one turn's work rather than twenty.
pub const TURN_INTERVAL: Duration = Duration::from_millis(40);

/// Colours whichever buffer most needs it, a chunk at a time, forever.
pub struct Highlighter;

impl<B: BufferTrait> ScheduledTask<B> for Highlighter {
    fn execute(&mut self, state: &EditorState<B>) -> bool {
        state.highlight_one_turn();
        // Never finishes: there is always another edit coming.
        true
    }
}

/// One turn's work for one buffer: the lines, the grammar, and where to start.
///
/// Taken out from under the lock as a unit so that the lexing below it touches
/// nothing shared.
pub(crate) struct Turn {
    pub buffer: String,
    pub version: u64,
    /// The line the first of `lines` sits at.
    pub first_line: usize,
    pub lines: Vec<String>,
    pub entering: SyntaxState,
    pub grammar: Grammar,
}

impl Turn {
    /// Colour the lines, off any lock.
    pub(crate) fn run(&self) -> Vec<(usize, SyntaxState, Vec<crate::modes::SyntaxSpan>)> {
        let mut out = Vec::with_capacity(self.lines.len());
        let mut state = self.entering.clone();
        for (offset, line) in self.lines.iter().enumerate() {
            let entering = state.clone();
            let (spans, leaving) = highlight_line(&self.grammar, line, &entering);
            out.push((self.first_line + offset, entering, spans));
            state = leaving;
        }
        out
    }
}

impl<B: BufferTrait> EditorState<B> {
    /// Find a buffer whose colouring has fallen behind and advance it by at
    /// most [`LINES_PER_TURN`] lines.
    ///
    /// The focused buffer first: it is the one being looked at, and colour
    /// arriving where nobody is reading is worth less than colour arriving
    /// where they are.
    pub(crate) fn highlight_one_turn(&self) {
        let Some(turn) = self.next_highlight_turn() else {
            return;
        };
        let coloured = turn.run();

        self.store_turn(&turn, coloured);
    }

    /// Store what a turn produced, if the buffer still looks the way it did
    /// when the turn read it.
    ///
    /// Separated from the computing above so it can be tested on its own: the
    /// rule it enforces only matters when the buffer changes *between* the two,
    /// and a test that had to arrange that inside one call could not arrange it
    /// at all.
    pub(crate) fn store_turn(
        &self,
        turn: &Turn,
        coloured: Vec<(usize, SyntaxState, Vec<crate::modes::SyntaxSpan>)>,
    ) {
        let Some(buffer) = self.get_buffer(&turn.buffer) else {
            return;
        };
        self.mutate_buffer(buffer, |buf| {
            // Checked here, not only when the lines were read: the text may
            // have changed while this turn was being computed, and spans from
            // the old text would be painted at columns that have moved.
            if buf.version != turn.version {
                return;
            }
            for (line, entering, spans) in coloured {
                if buf.syntax.record(line, &entering, spans).is_err() {
                    // Somebody else advanced the cache past this line. Whatever
                    // they wrote is at least as fresh as this, so it stands.
                    break;
                }
            }
        });
    }

    /// The next chunk of work, or nothing when every buffer is up to date.
    fn next_highlight_turn(&self) -> Option<Turn> {
        let focused = self.focused_window_buffer();
        let names: Vec<String> = {
            let buffers = self
                .buffers
                .read()
                .expect("Failed to acquire read lock on buffers");
            // The focused buffer first, then the rest in whatever order they
            // are held -- the point is only that what is on screen is not last.
            focused
                .iter()
                .cloned()
                .chain(buffers.keys().cloned())
                .collect()
        };

        for name in names {
            if let Some(turn) = self.turn_for(&name) {
                return Some(turn);
            }
        }
        None
    }

    /// One buffer's next chunk, if it has one.
    pub(crate) fn turn_for(&self, name: &str) -> Option<Turn> {
        let grammar = {
            let buffer = self.get_buffer(name)?;
            let buf = buffer
                .read()
                .expect("Failed to acquire read lock on buffer for highlighting");
            let mode = buf.current_mode.clone();
            // The grammar is read from the mode registry, which is a different
            // lock -- so the buffer's is released first. Cloned rather than
            // borrowed because the lexing happens with neither held.
            drop(buf);
            let registry = self
                .mode_registry
                .read()
                .expect("Failed to acquire read lock on mode_registry");
            let grammar = registry.get(&mode).map(|mode| mode.grammar.clone())?;
            if grammar.is_empty() {
                return None;
            }
            grammar
        };

        let buffer = self.get_buffer(name)?;
        let buf = buffer
            .read()
            .expect("Failed to acquire read lock on buffer for highlighting");
        let line_count = buf.text.line_count();
        let first_line = buf.syntax.valid_to();
        if first_line >= line_count {
            return None;
        }
        // The state entering the first line of this chunk. At the top of the
        // buffer that is the empty state; anywhere else it is what the previous
        // line left behind, which is in the cache because lines are only ever
        // recorded in order.
        let entering = match first_line.checked_sub(1) {
            None => SyntaxState::new(),
            Some(previous) => buf.syntax.state_at(previous).unwrap_or_default(),
        };
        let last_line = (first_line + LINES_PER_TURN).min(line_count);
        Some(Turn {
            buffer: name.to_string(),
            version: buf.version,
            first_line,
            lines: buf.text.get_lines(first_line, last_line),
            entering,
            grammar,
        })
    }
}
