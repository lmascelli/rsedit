//! What a buffer remembers about its own colouring.
//!
//! # Why the cache lives on the buffer
//!
//! Two properties decide it. The thing that invalidates a cached colouring is
//! an *edit*, and edits happen in exactly one place -- `edits::insert_text` and
//! `edits::delete_range`, the same two doors undo relies on. And a colouring
//! describes a particular text, so it must not outlive it: a side table in the
//! editor would have to be tidied when a buffer closes, and would be wrong
//! rather than absent if it ever were not.
//!
//! # What is kept
//!
//! Per line, two things: the spans to draw, and the lexer state *entering* that
//! line. The states are what make the whole scheme work -- with them, colouring
//! line 5000 needs only the state at line 5000, and re-lexing can start
//! anywhere a state is known rather than at the top of the file.
//!
//! # Stale colour is drawn, not cleared
//!
//! An edit marks everything below it untrusted, but the spans stay and keep
//! being drawn until the worker replaces them. Clearing instead would flash the
//! rest of the file to plain text on every keystroke, which is a far worse
//! thing to look at than a few milliseconds of colour that is slightly wrong.
//!
//! # What is deliberately not here yet
//!
//! **Resync.** After an edit, a recompute re-lexes forward to the end of the
//! file rather than stopping as soon as the state it computes agrees with the
//! state that was cached there. Stopping early is what makes an edit cost
//! O(lines until the state converges) instead of O(rest of the file), and it is
//! the reason a stateful lexer was worth building at all.
//!
//! It is left out because it needs the previous generation's states kept
//! alongside the new ones *and* an accounting of how line numbers shifted when
//! the edit added or removed lines -- and without that accounting it silently
//! compares the wrong lines. The bounded work per tick below already keeps the
//! editor responsive; this is about how much work there is in total.
use crate::modes::{SyntaxSpan, SyntaxState};

/// How many lines one turn of the worker colours before yielding.
///
/// The number that matters is not "how long is a file" but "how long may the
/// editor be unable to answer". Work in chunks and a huge file costs many short
/// turns rather than one long one, so nothing ever waits on the whole of it.
pub const LINES_PER_TURN: usize = 500;

/// A buffer's colouring, as far as it has been computed.
#[derive(Clone, Debug, Default)]
pub struct SyntaxCache {
    /// The buffer version this was computed from. A result arriving from the
    /// worker for any other version describes text that no longer exists.
    version: u64,
    /// The major mode this was computed in, and which edition of the editor's
    /// grammars it was computed under. See [`Self::describes`].
    mode: String,
    grammar: u64,
    /// Interned lexer state entering each computed line.
    states: Vec<u32>,
    /// Spans for each computed line, in character columns.
    spans: Vec<Vec<SyntaxSpan>>,
    /// Lines below this have been computed against the current version.
    /// Lines above it may still hold older spans, which are drawn until
    /// replaced -- see the module header.
    valid_to: usize,
    /// The state entering line `valid_to`: the one the next line to be
    /// computed starts in. See [`Self::frontier`].
    frontier: SyntaxState,
    /// The distinct states seen so far.
    ///
    /// Interned because almost every line in almost every file is entered in
    /// the empty state, and storing that `Vec` per line would make the cache
    /// proportional to the file rather than to the parts of it that are
    /// interesting.
    table: Vec<SyntaxState>,
}

impl SyntaxCache {
    /// The spans for LINE, or nothing if it has never been computed.
    ///
    /// Deliberately indifferent to whether the line is still *valid*: what has
    /// been computed is drawn until something better arrives.
    pub fn spans(&self, line: usize) -> &[SyntaxSpan] {
        self.spans.get(line).map(Vec::as_slice).unwrap_or_default()
    }

    /// How far the colouring is known to be current.
    pub fn valid_to(&self) -> usize {
        self.valid_to
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// The state entering LINE, when it is known.
    pub fn state_at(&self, line: usize) -> Option<SyntaxState> {
        let id = *self.states.get(line)?;
        self.table.get(id as usize).cloned()
    }

    /// The state the first line not yet computed is entered in -- the one the
    /// next turn of the worker starts from.
    ///
    /// # Why it is kept rather than looked up
    ///
    /// `states` holds the state *entering* each computed line, and a turn that
    /// starts at `valid_to` needs the state *leaving* the line above it. The
    /// two are the same on every line except one that opens or closes a
    /// region, which is exactly where it matters: starting from the line
    /// above's entering state carried a comment straight on past the line that
    /// closed it -- or left out one the line above had just opened -- and the
    /// rest of the file was painted inside out until an edit further up
    /// happened to start a turn from somewhere right.
    pub fn frontier(&self) -> &SyntaxState {
        &self.frontier
    }

    /// Whether this was computed in MODE, under the edition of the grammars the
    /// editor calls GRAMMAR.
    ///
    /// # Why the version is not enough
    ///
    /// The version moves when the *text* does. Putting a buffer into another
    /// mode does not touch its text, and neither does adding a rule to a mode's
    /// grammar -- `risp-refresh-vocabulary`, or a mode's file evaluated again
    /// -- so a colouring computed before either would otherwise stand for
    /// ever, wrong, until an edit happened to untrust it; and then only from
    /// that edit down.
    ///
    /// It is also what keeps a state from another grammar out of the lexer. A
    /// state names regions by their place in its grammar, and `make-mode` --
    /// the first line of every mode's file -- empties the grammar it remakes.
    pub fn describes(&self, mode: &str, grammar: u64) -> bool {
        self.grammar == grammar && self.mode == mode
    }

    /// Start again in MODE under GRAMMAR, from the top.
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

    /// Forget everything: a different buffer version entirely, or a mode change
    /// that means the old colouring describes the wrong grammar.
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

    /// An edit landed on LINE: nothing from there on is trusted any more.
    ///
    /// The spans are *kept*. They are wrong below the edit -- a line inserted
    /// above shifts every one of them -- but wrong for a few milliseconds beats
    /// blank, and the worker is already on its way.
    pub fn invalidate_from(&mut self, version: u64, line: usize) {
        self.version = version;
        if line < self.valid_to {
            // The state entering LINE was worked out from the lines above it,
            // and the edit touched none of those.
            self.frontier = self.state_at(line).unwrap_or_default();
            self.valid_to = line;
        }
    }

    /// Record the colouring of LINE, computed from the state entering it and
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

    /// Drop everything past LINE_COUNT, for a buffer that got shorter.
    pub fn truncate(&mut self, line_count: usize) {
        if line_count < self.valid_to {
            // Read before the states it is read from are dropped.
            self.frontier = self.state_at(line_count).unwrap_or_default();
            self.valid_to = line_count;
        }
        self.states.truncate(line_count);
        self.spans.truncate(line_count);
    }

    fn intern(&mut self, state: &SyntaxState) -> u32 {
        if let Some(id) = self.table.iter().position(|known| known == state) {
            return id as u32;
        }
        self.table.push(state.clone());
        (self.table.len() - 1) as u32
    }

    /// How many distinct states have been seen. For tests, and for anyone
    /// wondering whether interning is earning its keep.
    pub fn distinct_states(&self) -> usize {
        self.table.len()
    }
}
