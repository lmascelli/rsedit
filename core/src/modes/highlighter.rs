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
//! **Never store a result for a version that has moved on** -- nor for a
//! *mode* or a *grammar* that has. The buffer can change while the work is in
//! flight, and spans computed from the old text describe columns that no
//! longer mean anything. The version is checked again at the moment of
//! storing, not merely at the start. And neither a mode switch nor a rule
//! added to a grammar touches the text, so the cache also remembers which
//! mode and which edition of the grammars it was computed under, and a cache
//! that describes another is coloured again from the top.
use crate::{
    BufferTrait, EditorState,
    background::ScheduledTask,
    buffer::syntax::LINES_PER_TURN,
    modes::{Grammar, SyntaxSpan, SyntaxState, highlight_line},
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
    /// The mode the buffer was in, and the edition of the grammars `grammar`
    /// was read at -- what the cache is stamped with when this is stored.
    pub mode: String,
    pub grammar_epoch: u64,
    /// The line the first of `lines` sits at.
    pub first_line: usize,
    pub lines: Vec<String>,
    pub entering: SyntaxState,
    pub grammar: Grammar,
}

/// One line as a turn coloured it, with the states either side of it.
///
/// The state *leaving* the line is carried as well as the one entering it,
/// because it is the one the next line -- and, for the last line of a turn,
/// the next turn -- has to start from.
pub(crate) struct ColouredLine {
    pub line: usize,
    pub entering: SyntaxState,
    pub spans: Vec<SyntaxSpan>,
    pub leaving: SyntaxState,
}

impl Turn {
    /// Colour the lines, off any lock.
    pub(crate) fn run(&self) -> Vec<ColouredLine> {
        let mut out = Vec::with_capacity(self.lines.len());
        let mut state = self.entering.clone();
        for (offset, line) in self.lines.iter().enumerate() {
            let (spans, leaving) = highlight_line(&self.grammar, line, &state);
            out.push(ColouredLine {
                line: self.first_line + offset,
                entering: std::mem::replace(&mut state, leaving.clone()),
                spans,
                leaving,
            });
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
    pub(crate) fn store_turn(&self, turn: &Turn, coloured: Vec<ColouredLine>) {
        self.with_buffer_mut(&turn.buffer, |buf| {
            // Checked here, not only when the lines were read: the text may
            // have changed while this turn was being computed, and spans from
            // the old text would be painted at columns that have moved. The
            // mode likewise -- a turn lexed with another mode's grammar is not
            // this buffer's colouring whatever the text says.
            if buf.version != turn.version || buf.current_mode != turn.mode {
                return;
            }
            if !buf.syntax.describes(&turn.mode, turn.grammar_epoch) {
                // The cache was computed in another mode, or under a grammar
                // that has changed since. `turn_for` started this turn from the
                // top for that reason, and only a turn from the top can be the
                // beginning of the colouring that replaces it.
                if turn.first_line != 0 {
                    return;
                }
                buf.syntax
                    .restamp(turn.version, &turn.mode, turn.grammar_epoch);
            }
            for line in coloured {
                if buf
                    .syntax
                    .record(line.line, &line.entering, line.spans, &line.leaving)
                    .is_err()
                {
                    // Somebody else advanced the cache past this line. Whatever
                    // they wrote is at least as fresh as this, so it stands.
                    break;
                }
            }
        });
    }

    /// The next chunk of work, or nothing when every buffer is up to date.
    fn next_highlight_turn(&self) -> Option<Turn> {
        // The focused buffer first, then the rest in whatever order they are
        // held -- the point is only that what is on screen is not last.
        for name in self.buffer_names_focused_first() {
            if let Some(turn) = self.turn_for(&name) {
                return Some(turn);
            }
        }
        None
    }

    /// Whether NAME's colouring has fallen behind its text.
    ///
    /// # Why this is its own function
    ///
    /// Two things ask it, for different reasons. The worker asks so it knows
    /// whether to do a turn; a frame asks so the renderer knows whether to go
    /// back to sleep -- see [`crate::ui::FrameSnapshot::colouring_pending`].
    /// If they answered it separately they could disagree, and a renderer that
    /// believed the work was finished while the worker believed otherwise
    /// would sleep through exactly the colour it was waiting for.
    ///
    /// The cheap half is asked first. Whether the cache has caught up costs one
    /// buffer lock; whether the mode has a grammar at all costs the registry
    /// too, and is only worth asking about a buffer that is actually behind.
    /// Getting that order wrong would put the registry in the path of every
    /// frame, forever, for a file that is fully coloured.
    ///
    /// "Caught up" includes "under the grammar there is now". The edition of
    /// the grammars is a counter beside the registry rather than in it, which
    /// is what lets that be asked without the registry's lock.
    pub(crate) fn colouring_behind(&self, name: &str) -> bool {
        let grammar_epoch = self.grammar_epoch();
        let Some((mode, behind)) = self.with_buffer(name, |buf| {
            (
                buf.current_mode.clone(),
                !buf.syntax.describes(&buf.current_mode, grammar_epoch)
                    || buf.syntax.valid_to() < buf.text.line_count(),
            )
        }) else {
            return false;
        };
        if !behind {
            return false;
        }
        // A mode with no grammar never catches up, because there is nothing to
        // catch up *to*. Without this a plain-text buffer would report itself
        // behind forever, and the renderer would wake every turn for the life
        // of the session to redraw a frame that cannot change.
        self.modes(|modes| {
            modes
                .get(&mode)
                .is_some_and(|mode| !mode.grammar.is_empty())
        })
    }

    /// Whether any buffer's colouring has fallen behind.
    ///
    /// Every buffer, not only the visible ones: a file being coloured in a
    /// window that is not on screen is still work in flight, and the turn it
    /// takes is a turn the visible buffer does not get. The renderer waking for
    /// it is the honest reflection of that, and it stops as soon as the work
    /// does.
    pub(crate) fn colouring_pending(&self) -> bool {
        // The names are copied out and the table let go before any buffer is
        // locked: `colouring_behind` takes one per name.
        self.buffer_names()
            .iter()
            .any(|name| self.colouring_behind(name))
    }

    /// One buffer's next chunk, if it has one.
    pub(crate) fn turn_for(&self, name: &str) -> Option<Turn> {
        if !self.colouring_behind(name) {
            return None;
        }
        // The grammar is read from the mode registry, which is a different
        // lock -- so the buffer's is released first. Cloned rather than
        // borrowed because the lexing happens with neither held.
        //
        // The edition is read under the same lock as the grammar, and a
        // grammar is only ever changed with the edition moved under that lock
        // too (see `EditorState::edit_grammar`), so the two always agree.
        let mode = self.with_buffer(name, |buf| buf.current_mode.clone())?;
        let (grammar, grammar_epoch) = self.modes(|modes| {
            modes
                .get(&mode)
                .map(|found| (found.grammar.clone(), self.grammar_epoch()))
        })?;

        self.with_buffer(name, |buf| {
            // Re-read rather than carried over from above: the lock was
            // released in between and the buffer may have been put into
            // another mode.
            if buf.current_mode != mode {
                return None;
            }
            let line_count = buf.text.line_count();
            // Where to carry on from, and the state the first line of this
            // chunk is entered in: what the cache has reached, when what it
            // holds was computed in this mode under this grammar, and the top
            // of the buffer in the empty state when it was not.
            let (first_line, entering) = if buf.syntax.describes(&mode, grammar_epoch) {
                (buf.syntax.valid_to(), buf.syntax.frontier().clone())
            } else {
                (0, SyntaxState::new())
            };
            // Re-checked rather than assumed from `colouring_behind` above: the
            // buffer lock was released in between, so the text may have been
            // coloured, shortened, or emptied since.
            if first_line >= line_count {
                return None;
            }
            let last_line = (first_line + LINES_PER_TURN).min(line_count);
            Some(Turn {
                buffer: name.to_string(),
                version: buf.version,
                mode,
                grammar_epoch,
                first_line,
                lines: buf.text.get_lines(first_line, last_line),
                entering,
                grammar,
            })
        })?
    }
}
