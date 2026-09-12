//! Finding text in a buffer: what to look for, and the scan that looks.
//!
//! # Why this is its own module and not part of the primitives
//!
//! Searching has nothing to do with the editor. It needs a haystack that can
//! hand over characters by position and nothing else -- no point, no mark, no
//! undo history, no locks. Keeping it here, above [`BufferTrait`] and below
//! everything that knows what a buffer *is*, means the scan can be tested
//! against a plain buffer with no editor around it, and means incremental
//! search and any later replace cannot drift into two subtly different ideas
//! of what counts as a match.
//!
//! # Offsets are characters, everywhere
//!
//! Every position in this module -- and in every primitive that calls it -- is
//! a character offset, because that is what point, mark and the undo history
//! are counted in. The regular-expression engine works in *bytes*, so the
//! regexp path converts on the way in and on the way back out. Getting that
//! wrong is invisible in ASCII and corrupts the first accented letter it meets,
//! which is why the conversion happens in exactly one place here rather than at
//! each call site.
//!
//! # What this costs
//!
//! A literal scan reads the haystack through [`BufferTrait::at`] and allocates
//! nothing, so searching a large buffer for a short string costs one pass and
//! no copy. The regexp scan cannot: `regex` matches `&str`, so it materialises
//! the buffer once per call -- and an incremental search re-scans on every
//! keystroke, so a regexp isearch over a large buffer costs one copy per key.
//! It is the honest, simple version; the scalable fix -- when a buffer large
//! enough to notice turns up -- is a rope whose chunks can be fed to
//! `regex_automata`'s streaming search, which is a change to this module and
//! to nothing else.
use crate::buffer::BufferTrait;
use regex::{Regex, RegexBuilder};

/// One match, in character offsets, together with what its groups captured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    /// What each capture group matched, group 0 being the whole match. Empty
    /// for a literal pattern, which has no groups -- so a `\1` in a literal
    /// replacement expands to nothing rather than to something arbitrary.
    pub groups: Vec<Option<String>>,
}

impl Match {
    fn literal(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            groups: Vec::new(),
        }
    }

    /// Whether this match covers no characters.
    ///
    /// A regexp like `x*` can match the empty string anywhere. Nothing is
    /// wrong with reporting that, but a *loop* that replaces every match has
    /// to step past it by hand or it will sit on one position forever -- see
    /// how the replace loop advances.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// What to look for.
#[derive(Clone, Debug)]
pub enum Pattern {
    /// A plain string, compared character by character.
    Literal { chars: Vec<char>, fold: bool },
    /// A regular expression, compiled once and reused for every step of a
    /// replace loop.
    Regex(Regex),
}

impl Pattern {
    /// Compile SOURCE, as a regexp when REGEXP is set and as a literal string
    /// otherwise, folding case when FOLD is set.
    ///
    /// An empty pattern is refused rather than accepted. It would match at
    /// every position, so a search would never move and a replace would never
    /// terminate: making it an error here means no caller has to remember the
    /// guard.
    pub fn new(source: &str, regexp: bool, fold: bool) -> Result<Self, String> {
        if source.is_empty() {
            return Err("Searching for the empty string would never finish".into());
        }
        if regexp {
            // `multi_line` so that `^` and `$` mean the beginning and end of a
            // *line*, which is what they mean to anyone typing a pattern into
            // an editor. Without it they would anchor to the ends of the whole
            // buffer and a pattern like `^;;` would match at most once.
            RegexBuilder::new(source)
                .case_insensitive(fold)
                .multi_line(true)
                .build()
                .map(Pattern::Regex)
                .map_err(|why| format!("Invalid regexp: {why}"))
        } else {
            Ok(Pattern::Literal {
                chars: source.chars().collect(),
                fold,
            })
        }
    }

    /// The first match starting at or after FROM and ending at or before
    /// LIMIT.
    ///
    /// LIMIT is what confines a replace to the region: the caller passes the
    /// region's end instead of the buffer's, and a match that would straddle
    /// the boundary is simply not found.
    pub fn search_forward<B: BufferTrait>(
        &self,
        text: &B,
        from: usize,
        limit: usize,
    ) -> Option<Match> {
        let limit = limit.min(text.len());
        if from > limit {
            return None;
        }
        match self {
            Pattern::Literal { chars, fold } => {
                // `+ 1` because a match may start at the last possible offset:
                // searching for "ab" in "ab" has to try start 0, and
                // `limit - len` is 0.
                let last_start = limit.checked_sub(chars.len())?;
                (from..=last_start)
                    .find(|&start| matches_at(text, start, chars, *fold))
                    .map(|start| Match::literal(start, start + chars.len()))
            }
            Pattern::Regex(regex) => {
                let haystack = text.to_string();
                // `find_at` rather than searching a slice: it starts the scan
                // at the given byte while still seeing what came before it, so
                // `^` and a look-behind mean what they should. Slicing the
                // haystack would make every resumed search look like the start
                // of a line.
                let byte_from = char_to_byte(&haystack, from)?;
                let captures = regex.captures_at(&haystack, byte_from)?;
                let found = to_match(&haystack, &captures);
                (found.end <= limit).then_some(found)
            }
        }
    }

    /// The last match ending at or before TO and starting at or after FLOOR.
    ///
    /// Backwards, so that `C-r`-style searching and a backward `replace` walk
    /// the buffer in the direction the user is reading it.
    pub fn search_backward<B: BufferTrait>(
        &self,
        text: &B,
        to: usize,
        floor: usize,
    ) -> Option<Match> {
        let to = to.min(text.len());
        if floor > to {
            return None;
        }
        match self {
            Pattern::Literal { chars, fold } => {
                let last_start = to.checked_sub(chars.len())?;
                (floor..=last_start)
                    .rev()
                    .find(|&start| matches_at(text, start, chars, *fold))
                    .map(|start| Match::literal(start, start + chars.len()))
            }
            Pattern::Regex(_) => {
                // The engine only searches forwards, so "the last one" means
                // walking every match and keeping the final one that fits.
                // Backward regexp search is rare enough that paying a forward
                // pass for it is better than carrying a reversed automaton.
                let mut best = None;
                let mut at = floor;
                while let Some(found) = self.search_forward(text, at, to) {
                    at = if found.is_empty() {
                        found.end + 1
                    } else {
                        found.end
                    };
                    best = Some(found);
                }
                best
            }
        }
    }
}

/// Whether CHARS appears at offset AT.
fn matches_at<B: BufferTrait>(text: &B, at: usize, chars: &[char], fold: bool) -> bool {
    chars.iter().enumerate().all(|(offset, &wanted)| {
        text.at(at + offset)
            .is_some_and(|got| chars_equal(got, wanted, fold))
    })
}

/// Compare two characters, optionally ignoring case.
///
/// `to_lowercase` rather than `eq_ignore_ascii_case`, so that a search for
/// "straße" or "ÅNGSTRÖM" folds the way the rest of the text does. It yields an
/// iterator because one character can lower-case to several, which is exactly
/// the case a cheaper comparison would get wrong.
fn chars_equal(a: char, b: char, fold: bool) -> bool {
    a == b || (fold && a.to_lowercase().eq(b.to_lowercase()))
}

/// Byte offset of the CHARS'th character, or the string's length when CHARS is
/// exactly its character count.
///
/// `None` past that: a caller asking to resume a search beyond the end of the
/// text has no match to find, and saying so is better than clamping it to the
/// end and reporting a match it did not ask for.
fn char_to_byte(haystack: &str, chars: usize) -> Option<usize> {
    if chars == 0 {
        return Some(0);
    }
    haystack
        .char_indices()
        .nth(chars)
        .map(|(byte, _)| byte)
        .or_else(|| (haystack.chars().count() == chars).then_some(haystack.len()))
}

/// Character offset of a byte offset.
fn byte_to_char(haystack: &str, byte: usize) -> usize {
    haystack[..byte].chars().count()
}

/// A regexp capture set, converted into the character-offset [`Match`] the rest
/// of the editor speaks.
fn to_match(haystack: &str, captures: &regex::Captures<'_>) -> Match {
    let whole = captures
        .get(0)
        .expect("a capture set always has a group zero");
    Match {
        start: byte_to_char(haystack, whole.start()),
        end: byte_to_char(haystack, whole.end()),
        groups: captures
            .iter()
            .map(|group| group.map(|m| m.as_str().to_string()))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// A search in progress
// ---------------------------------------------------------------------------

/// Which way an incremental search is currently travelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

impl Direction {
    pub fn reversed(self) -> Self {
        match self {
            Direction::Forward => Direction::Backward,
            Direction::Backward => Direction::Forward,
        }
    }
}

/// An incremental search part-way through.
///
/// # Why the editor holds this rather than a loop holding it
///
/// Incremental search is a loop over keystrokes -- read a key, extend the
/// pattern, search again -- and there is nothing here to read a key *from*:
/// keys arrive one per turn of the event loop, so a command that waited for one
/// would be waiting inside the handler that delivers it.
///
/// So the loop is turned inside out, exactly as `PendingCommand` turns argument
/// collection inside out. What a loop would have kept in its locals lives here
/// between keystrokes, and the thing that drives it is `isearch-mode`'s
/// `post-command-hook`: every command run while the prompt is open -- which is
/// every character typed into it -- gets one turn of the loop.
///
/// Nothing in here refers to a buffer, a window or a lock: only a buffer's
/// *name* and offsets into its text. That is what lets the session be read and
/// written without holding anything, and what lets it describe a search in a
/// buffer that is not the current one -- which, while the prompt is open, is
/// always the case.
#[derive(Clone, Debug)]
pub struct Isearch {
    /// The buffer being searched. Not the current buffer: while the prompt is
    /// open the current buffer is the minibuffer, which is why every position
    /// below is meaningless without this name beside it.
    pub buffer: String,
    /// Where point was when the search began. Restored if the search is
    /// abandoned -- that restoration is the whole reason this is recorded.
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

    /// Move the scan past the current match, so a repeat finds the next one.
    ///
    /// `start + 1` rather than `end`, so that overlapping matches are all
    /// reachable: repeating a search for "aa" through "aaa" should find the
    /// match at 1, and resuming at the first match's end would step over it.
    pub fn advance(&mut self) {
        let Some(found) = &self.found else { return };
        self.from = match self.direction {
            Direction::Forward => found.start + 1,
            Direction::Backward => found.end.saturating_sub(1),
        };
    }

    /// Start again from the far end of the buffer.
    ///
    /// Only ever reached by repeating a search that is already failing, which
    /// is Emacs' rule and worth keeping: a search that silently wrapped the
    /// moment it ran out would quietly take you somewhere you did not ask to
    /// go, and the "Failing" report is the warning that makes the second press
    /// a decision rather than an accident.
    pub fn wrap(&mut self, buffer_len: usize) {
        self.from = match self.direction {
            Direction::Forward => 0,
            Direction::Backward => buffer_len,
        };
        self.wrapped = true;
        self.failing = false;
    }

    /// How the search reads in the echo area.
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
