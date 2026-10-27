//! The highlighter: the job that colours each buffer a chunk of lines at a
//! step, from the line its colouring has reached, while one is behind, and
//! asleep until a buffer or a grammar changes. It lexes with no lock held,
//! and stores a turn only if the buffer is still the text, mode and grammar
//! it read.
//!
//! Design: `doc/architettura-editor.typ, <lavori-editor>`.
use crate::{
    BufferTrait, EditorState,
    buffer::syntax::LINES_PER_TURN,
    editor::{BUFFER_CHANGED, SYNTAX_CHANGED},
    modes::{Grammar, SyntaxSpan, SyntaxState, highlight_line},
    work::{Class, Job, JobError, Spec, Step, StepContext},
};

/// Colours whichever buffer most needs it, a chunk at a time, for as long as
/// one is behind -- then waits for the next change.
pub struct Highlighter;

impl Highlighter {
    /// Returns how it is scheduled: in the high class, since colour is what
    /// the user is looking at, and woken by any change to a buffer or a
    /// grammar.
    pub(crate) fn spec() -> Spec {
        Spec::new("highlighter")
            .class(Class::High)
            .unit("lines")
            .on(BUFFER_CHANGED)
            .on(SYNTAX_CHANGED)
    }
}

impl<B: BufferTrait> Job<EditorState<B>> for Highlighter {
    fn step(&mut self, cx: &mut StepContext<'_, EditorState<B>>) -> Result<Step, JobError> {
        match cx.state.highlight_one_turn() {
            // Lines coloured, and there may be more.
            Some((buffer, lines)) => {
                cx.worked(lines as u64, &buffer);
                Ok(Step::Continue)
            }
            // Nothing behind: asleep until the next change.
            None => Ok(Step::Wait),
        }
    }
}

/// One turn's work for one buffer: the lines, the grammar and where to start,
/// copied out from under the locks, so that the lexing touches nothing shared.
pub(crate) struct Turn {
    pub buffer: String,
    pub version: u64,
    /// The mode the buffer was in, and the edition of the grammars `grammar`
    /// was read at: what the cache is stamped with when this is stored.
    pub mode: String,
    pub grammar_epoch: u64,
    /// The line the first of `lines` sits at.
    pub first_line: usize,
    pub lines: Vec<String>,
    pub entering: SyntaxState,
    pub grammar: Grammar,
}

/// One line as a turn coloured it, with the states either side of it: the one
/// leaving it is where the next line, or the next turn, starts from.
pub(crate) struct ColouredLine {
    pub line: usize,
    pub entering: SyntaxState,
    pub spans: Vec<SyntaxSpan>,
    pub leaving: SyntaxState,
}

impl Turn {
    /// Colours the lines, with no lock held.
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
    /// Advances by at most [`LINES_PER_TURN`] lines the colouring of a buffer
    /// that has fallen behind, the focused one first, and returns which buffer
    /// and how many lines, or `None` when every buffer is up to date.
    pub(crate) fn highlight_one_turn(&self) -> Option<(String, usize)> {
        let turn = self.next_highlight_turn()?;
        let coloured = turn.run();
        let lines = coloured.len();
        self.store_turn(&turn, coloured);
        Some((turn.buffer, lines))
    }

    /// Stores what TURN produced, if the buffer is still the text, mode and
    /// grammar the turn read. Apart from the computing, so that a test can
    /// change the buffer between the two.
    pub(crate) fn store_turn(&self, turn: &Turn, coloured: Vec<ColouredLine>) {
        self.with_buffer_mut(&turn.buffer, |buf| {
            // Checked here, not only when the lines were read: spans of an
            // older text would be painted at columns that have moved, and a
            // turn lexed with another mode's grammar is not this buffer's
            // colouring whatever the text says.
            if buf.version != turn.version || buf.current_mode != turn.mode {
                return;
            }
            if !buf.syntax.describes(&turn.mode, turn.grammar_epoch) {
                // The cache was computed in another mode, or under a grammar
                // that has changed since. `turn_for` started this turn from the
                // top for that reason, and only a turn from the top can begin
                // the colouring that replaces it.
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
                    // Something else advanced the cache past this line, and
                    // what it wrote is at least as fresh, so it stands.
                    break;
                }
            }
        });
    }

    /// Returns the next chunk of work, or `None` when every buffer is up to
    /// date.
    fn next_highlight_turn(&self) -> Option<Turn> {
        // The focused buffer first, then the rest in the order they are held:
        // what is being looked at is not last.
        for name in self.buffer_names_focused_first() {
            if let Some(turn) = self.turn_for(&name) {
                return Some(turn);
            }
        }
        None
    }

    /// Returns whether NAME's colouring has fallen behind its text, or the
    /// grammar there is now.
    ///
    /// The one answer both the job and a frame
    /// ([`crate::ui::FrameSnapshot::colouring_pending`]) read, so that they
    /// cannot disagree. The cheap half first: whether the cache has caught up
    /// costs a buffer lock, and only a buffer that is behind costs a look in
    /// the registry for whether its mode has a grammar at all.
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
        // A mode with no grammar is never behind: there is nothing to catch up
        // to, and a plain-text buffer would otherwise wake the renderer every
        // turn for good.
        self.modes(|modes| {
            modes
                .get(&mode)
                .is_some_and(|mode| !mode.grammar.is_empty())
        })
    }

    /// Returns whether any buffer's colouring has fallen behind, visible or
    /// not: a buffer coloured off screen is still work in flight.
    pub(crate) fn colouring_pending(&self) -> bool {
        // The names are copied out and the table let go before any buffer is
        // locked: `colouring_behind` takes one per name.
        self.buffer_names()
            .iter()
            .any(|name| self.colouring_behind(name))
    }

    /// Returns NAME's next chunk of work, if it has one.
    pub(crate) fn turn_for(&self, name: &str) -> Option<Turn> {
        if !self.colouring_behind(name) {
            return None;
        }
        // The grammar is read from the mode registry, another lock, so the
        // buffer's is given back first; it is cloned, since the lexing holds
        // neither. The edition is read under the registry's lock, which every
        // change of a grammar moves it under (`EditorState::edit_grammar`),
        // so the two agree.
        let mode = self.with_buffer(name, |buf| buf.current_mode.clone())?;
        let (grammar, grammar_epoch) = self.modes(|modes| {
            modes
                .get(&mode)
                .map(|found| (found.grammar.clone(), self.grammar_epoch()))
        })?;

        self.with_buffer(name, |buf| {
            // Read again: the lock was given back in between, and the buffer
            // may be in another mode now.
            if buf.current_mode != mode {
                return None;
            }
            let line_count = buf.text.line_count();
            // Where to carry on from, and the state the first line is entered
            // in: what the cache has reached, when it was computed in this
            // mode under this grammar, and otherwise the top in the empty
            // state.
            let (first_line, entering) = if buf.syntax.describes(&mode, grammar_epoch) {
                (buf.syntax.valid_to(), buf.syntax.frontier().clone())
            } else {
                (0, SyntaxState::new())
            };
            // Checked again: since `colouring_behind`, the text may have been
            // coloured, shortened or emptied.
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
