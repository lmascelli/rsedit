//! An incremental search between keystrokes: the buffer it searches, where it
//! began, where the next scan starts, what it found and how many matches
//! there are. `isearch-mode`'s post-command hook drives it, a turn for every
//! command run in the prompt.
//!
//! Design: `doc/architettura-editor.typ, <isearch>`, and
//! `doc/architettura-editor.typ, <conteggio>` for the count.
use crate::text::search::{Match, Pattern};
use std::sync::{Arc, OnceLock};

/// Every match of a pattern in the searched text, where each starts, in
/// order: at most [`COUNT_LIMIT`], `truncated` when there were more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchCount {
    pub starts: Vec<usize>,
    pub truncated: bool,
}

/// How many matches a count keeps: enough for any number worth reading, and a
/// bound on the memory a pattern that matches everywhere can take.
pub const COUNT_LIMIT: usize = 100_000;

/// A count asked for: of which pattern, as typed, in which version of the
/// text, and the count itself once it is made. The count is shared with the
/// thread that makes it, which fills it once.
#[derive(Clone, Debug)]
pub struct Counting {
    pub typed: String,
    pub version: u64,
    pub result: Arc<OnceLock<MatchCount>>,
}

impl Counting {
    /// Returns a count of TYPED in the text at VERSION, not made yet.
    pub fn new(typed: &str, version: u64) -> Self {
        Self {
            typed: typed.to_string(),
            version,
            result: Arc::new(OnceLock::new()),
        }
    }

    /// Returns whether this is the count of TYPED at VERSION.
    pub fn is_for(&self, typed: &str, version: u64) -> bool {
        self.typed == typed && self.version == version
    }
}

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
    /// What was last searched for, compiled: what the other matches on
    /// screen are drawn from. `None` while nothing is typed, and while what
    /// is typed is not yet a valid regexp.
    pub pattern: Option<Pattern>,
    /// Whether the last scan failed. A repeat while failing is what wraps --
    /// see `Isearch::wrap`.
    pub failing: bool,
    /// Whether the scan has been round the end of the buffer, for the report.
    pub wrapped: bool,
    /// The count of the matches, asked for or made; `None` while nothing is
    /// typed, and when counting is off.
    pub counting: Option<Counting>,
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
            pattern: None,
            failing: false,
            wrapped: false,
            counting: None,
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

    /// Returns where the search is among the matches, as `3/17 `: the match
    /// shown, counted from the top of the buffer, and how many there are; `?`
    /// for the first when no match is shown, or when it lies past the matches
    /// a truncated count kept, and a `+` after the second when the count was
    /// truncated. Empty while the count is not made.
    pub fn count_label(&self) -> String {
        let Some(count) = self.counting.as_ref().and_then(|c| c.result.get()) else {
            return String::new();
        };
        let total = count.starts.len();
        let more = if count.truncated { "+" } else { "" };
        let at = self.found.as_ref().and_then(|found| {
            let last = *count.starts.last()?;
            (!count.truncated || found.start <= last)
                .then(|| count.starts.partition_point(|&start| start <= found.start))
        });
        match at {
            _ if total == 0 => "0/0 ".to_string(),
            Some(at) => format!("{at}/{total}{more} "),
            None => format!("?/{total}{more} "),
        }
    }

    /// Returns how the search reads in the echo area: where it is among the
    /// matches, when they are counted, what it is, and the pattern.
    pub fn report(&self, pattern: &str) -> String {
        let mut label = self.count_label();
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
