//! An incremental search between keystrokes: the buffer it searches, where it
//! began, where the next scan starts and what it found. `isearch-mode`'s
//! post-command hook drives it, a turn for every command run in the prompt.
//!
//! Design: `doc/architettura-editor.typ, <isearch>`.
use crate::text::search::Match;

/// Which way an incremental search is travelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// An incremental search part-way through.
///
/// It holds what a loop over keystrokes would keep in its locals, since there
/// is no read of a key to loop on. Nothing here refers to a buffer, a window or
/// a lock -- only a buffer's name and offsets into its text -- so it can be
/// read and written without holding anything, and describes a search in a
/// buffer that is not the current one, which while the prompt is open is
/// always the case.
#[derive(Clone, Debug)]
pub struct Isearch {
    /// The buffer being searched. Not the current buffer, which while the
    /// prompt is open is the minibuffer.
    pub buffer: String,
    /// Where point was when the search began, restored if it is abandoned.
    pub origin: usize,
    pub direction: Direction,
    pub regexp: bool,
    /// Where the next scan starts. Typing does *not* move it, so extending the
    /// pattern re-searches from the same place and the match grows where the
    /// user is looking; only repeating (`C-s`) advances it.
    pub from: usize,
    /// The match currently being shown, if the pattern matched anything.
    pub found: Option<Match>,
    /// Whether the last scan failed. A repeat while failing is what wraps --
    /// see `Isearch::wrap`.
    pub failing: bool,
    /// Whether the scan has been round the end of the buffer, for the report.
    pub wrapped: bool,
}

impl Isearch {
    /// Creates a search of BUFFER from ORIGIN, travelling in DIRECTION.
    pub fn new(buffer: String, origin: usize, direction: Direction, regexp: bool) -> Self {
        Self {
            buffer,
            origin,
            direction,
            regexp,
            from: origin,
            found: None,
            failing: false,
            wrapped: false,
        }
    }

    /// Moves the scan past the current match, so that a repeat finds the next
    /// one.
    ///
    /// One past the match's start rather than its end, so that overlapping
    /// matches are all reachable: repeating a search for "aa" through "aaa"
    /// finds the match at 1.
    pub fn advance(&mut self) {
        let Some(found) = &self.found else { return };
        self.from = match self.direction {
            Direction::Forward => found.start + 1,
            Direction::Backward => found.end.saturating_sub(1),
        };
    }

    /// Starts again from the far end of the buffer.
    ///
    /// Only for a repeat of a search that is already failing, as in Emacs: the
    /// "Failing" report is the warning that makes the second press a decision
    /// rather than an accident.
    pub fn wrap(&mut self, buffer_len: usize) {
        self.from = match self.direction {
            Direction::Forward => 0,
            Direction::Backward => buffer_len,
        };
        self.wrapped = true;
        self.failing = false;
    }

    /// Returns how the search reads in the echo area.
    pub fn report(&self, pattern: &str) -> String {
        let mut label = String::new();
        if self.failing {
            label.push_str("Failing ");
        }
        if self.wrapped {
            label.push_str("Wrapped ");
        }
        label.push_str(match (self.direction, self.regexp) {
            (Direction::Forward, false) => "I-search",
            (Direction::Backward, false) => "I-search backward",
            (Direction::Forward, true) => "Regexp I-search",
            (Direction::Backward, true) => "Regexp I-search backward",
        });
        format!("{label}: {pattern}")
    }
}
