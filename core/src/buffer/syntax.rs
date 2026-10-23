//! The colouring cache: per line, the spans to draw and the lexer state
//! entering the line, filled by the highlighter [`LINES_PER_TURN`] lines at a
//! time. After an edit the old spans are drawn until new ones replace them.
//!
//! Design: `doc/architettura-editor.typ, <cache-testo>`.
use crate::modes::{SyntaxSpan, SyntaxState};

/// How many lines one turn of the highlighter, or of the prescanner, goes
/// through before yielding: a large file costs many short turns rather than
/// one long one.
pub const LINES_PER_TURN: usize = 500;

/// A buffer's colouring, as far as it has been computed.
#[derive(Clone, Debug, Default)]
pub struct SyntaxCache {
    /// The buffer version this was computed from. A result arriving from the
    /// highlighter for any other version describes text that no longer exists.
    version: u64,
    /// The major mode this was computed in, and which edition of the editor's
    /// grammars it was computed under. See [`Self::describes`].
    mode: String,
    grammar: u64,
    /// Interned lexer state entering each computed line.
    states: Vec<u32>,
    /// Spans for each computed line, in character columns.
    spans: Vec<Vec<SyntaxSpan>>,
    /// The lines before this one have been computed against the current
    /// version. From this one on, a line may still hold older spans, which
    /// are drawn until replaced.
    valid_to: usize,
    /// The state entering line `valid_to`: the one the next line to be
    /// computed starts in. See [`Self::frontier`].
    frontier: SyntaxState,
    /// The distinct states seen so far: interned because almost every line is
    /// entered in the empty state, and a state stored per line would make the
    /// cache grow with the file.
    table: Vec<SyntaxState>,
}

impl SyntaxCache {
    /// Returns the spans for LINE, or nothing if it has never been computed.
    ///
    /// Whether the line is still valid does not matter: what has been computed
    /// is drawn until something better arrives.
    pub fn spans(&self, line: usize) -> &[SyntaxSpan] {
        self.spans.get(line).map(Vec::as_slice).unwrap_or_default()
    }

    /// Returns how far the colouring is known to be current.
    pub fn valid_to(&self) -> usize {
        self.valid_to
    }

    /// Returns the buffer version this was computed from.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Returns the state entering LINE, when it is known.
    pub fn state_at(&self, line: usize) -> Option<SyntaxState> {
        let id = *self.states.get(line)?;
        self.table.get(id as usize).cloned()
    }

    /// Returns the state the first line not yet computed is entered in: the
    /// one the next turn of the highlighter starts from.
    ///
    /// Kept rather than looked up: `states` holds the state *entering* each
    /// line, and the next turn needs the state *leaving* the line above, which
    /// differs on a line that opens or closes a region.
    pub fn frontier(&self) -> &SyntaxState {
        &self.frontier
    }

    /// Returns whether this was computed in MODE, under the edition of the
    /// grammars the editor calls GRAMMAR.
    ///
    /// The version is not enough: putting a buffer into another mode does not
    /// touch its text, and neither does adding a rule to a mode's grammar. It
    /// also keeps a state from another grammar out of the lexer: a state names
    /// regions by their place in its grammar.
    pub fn describes(&self, mode: &str, grammar: u64) -> bool {
        self.grammar == grammar && self.mode == mode
    }

    /// Starts again in MODE under GRAMMAR, from the top.
    ///
    /// The spans are kept, for the reason [`Self::invalidate_from`] keeps them:
    /// the old colour is drawn until the new arrives, rather than the whole
    /// buffer flashing to plain text while a mode's file is being loaded.
    pub fn restamp(&mut self, version: u64, mode: &str, grammar: u64) {
        self.version = version;
        mode.clone_into(&mut self.mode);
        self.grammar = grammar;
        self.valid_to = 0;
        self.frontier.clear();
    }

    /// Forgets everything: a different buffer version entirely, or a mode
    /// change that means the old colouring describes the wrong grammar.
    pub fn reset(&mut self, version: u64) {
        self.version = version;
        self.mode.clear();
        self.grammar = 0;
        self.states.clear();
        self.spans.clear();
        self.table.clear();
        self.valid_to = 0;
        self.frontier.clear();
    }

    /// Stops trusting anything from LINE on, where an edit landed.
    ///
    /// The spans are *kept*. They are wrong below the edit -- a line inserted
    /// above shifts every one of them -- but wrong for a few milliseconds beats
    /// blank, and the highlighter is already on its way.
    pub fn invalidate_from(&mut self, version: u64, line: usize) {
        self.version = version;
        if line < self.valid_to {
            // The state entering LINE was worked out from the lines above it,
            // and the edit touched none of those.
            self.frontier = self.state_at(line).unwrap_or_default();
            self.valid_to = line;
        }
    }

    /// Records the colouring of LINE, computed from the state entering it and
    /// yielding the state leaving it.
    ///
    /// Lines have to arrive in order -- each one's entering state is the
    /// previous one's exit -- so anything out of order is refused rather than
    /// written somewhere misleading. And so is a line entered in any state but
    /// the one the cache says it is entered in: it was lexed from a start that
    /// is not this text's.
    pub fn record(
        &mut self,
        line: usize,
        entering: &SyntaxState,
        spans: Vec<SyntaxSpan>,
        leaving: &SyntaxState,
    ) -> Result<(), ()> {
        if line != self.valid_to || *entering != self.frontier {
            return Err(());
        }
        let id = self.intern(entering);
        if self.states.len() <= line {
            self.states.resize(line + 1, 0);
            self.spans.resize(line + 1, Vec::new());
        }
        self.states[line] = id;
        self.spans[line] = spans;
        self.valid_to = line + 1;
        leaving.clone_into(&mut self.frontier);
        Ok(())
    }

    /// Drops everything past LINE_COUNT, for a buffer that got shorter.
    pub fn truncate(&mut self, line_count: usize) {
        if line_count < self.valid_to {
            // Read before the states it is read from are dropped.
            self.frontier = self.state_at(line_count).unwrap_or_default();
            self.valid_to = line_count;
        }
        self.states.truncate(line_count);
        self.spans.truncate(line_count);
    }

    /// Returns STATE's number in the table, adding it when it is new.
    fn intern(&mut self, state: &SyntaxState) -> u32 {
        if let Some(id) = self.table.iter().position(|known| known == state) {
            return id as u32;
        }
        self.table.push(state.clone());
        (self.table.len() - 1) as u32
    }

    /// Returns how many distinct states have been seen. For the tests.
    pub fn distinct_states(&self) -> usize {
        self.table.len()
    }
}
