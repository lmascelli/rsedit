//! Finding text: what to look for ([`Pattern`]), the next match from a
//! position, every match in one pass ([`Pattern::scan`]), and the regular
//! expressions `string-match` compiles once. Positions are characters; the
//! regexp engine's bytes are converted here, and nowhere else.
//!
//! Design: `doc/architettura-editor.typ, <ricerca>`.
use crate::buffer::BufferTrait;
use regex::{Regex, RegexBuilder};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Returns a match from START to END with no groups, as a literal finds.
    fn literal(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            groups: Vec::new(),
        }
    }

    /// Returns whether this match covers no characters.
    ///
    /// A regexp like `x*` matches the empty string anywhere: a loop over
    /// matches has to step past an empty one by hand, or it stays on one
    /// position for ever.
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
    /// Compiles SOURCE, as a regexp when REGEXP is set and as a literal
    /// string otherwise, folding case when FOLD is set.
    ///
    /// The empty pattern is refused: it would match at every position, so a
    /// search would never move and a replace would never end.
    pub fn new(source: &str, regexp: bool, fold: bool) -> Result<Self, String> {
        if source.is_empty() {
            return Err("Searching for the empty string would never finish".into());
        }
        if regexp {
            // `multi_line`, so that `^` and `$` mean the beginning and end of
            // a line, as anyone typing a pattern into an editor expects.
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

    /// Returns the first match that starts at or after FROM and ends at or
    /// before LIMIT.
    ///
    /// LIMIT is what confines a replace to the region: the caller passes the
    /// region's end rather than the buffer's.
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
                // Inclusive: a match may start at the last possible offset --
                // "ab" in "ab" starts at 0, which is `limit - len`.
                let last_start = limit.checked_sub(chars.len())?;
                (from..=last_start)
                    .find(|&start| matches_at(text, start, chars, *fold))
                    .map(|start| Match::literal(start, start + chars.len()))
            }
            Pattern::Regex(regex) => {
                // `regex` matches a `&str`: the text is copied, once per call.
                let haystack = text.to_string();
                // `captures_at` rather than a slice: it starts at the given
                // byte while still seeing what came before it, so `^` and a
                // look-behind mean what they should.
                let byte_from = char_to_byte(&haystack, from)?;
                let captures = regex.captures_at(&haystack, byte_from)?;
                let found = to_match(&haystack, &captures);
                // TODO(RIC-4): a match running past LIMIT is dropped, even when
                // a shorter one would fit.
                (found.end <= limit).then_some(found)
            }
        }
    }

    /// Returns the last match that ends at or before TO and starts at or after
    /// FLOOR.
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
                // The engine only searches forwards, so the last match is
                // found by walking every one and keeping the final one.
                // TODO(RIC-2): each step copies the text again, which makes
                // the walk quadratic in the length of the text.
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

// ---------------------------------------------------------------------------
// Scanning for every match
// ---------------------------------------------------------------------------
//
// One pass over a text already copied, with each match's line worked out as
// the pass goes by -- where `search_forward` in a loop would copy the text once
// per match.

/// One match, with where it sits in the text it was found in. Offsets are
/// characters, as everywhere in the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Character offsets of the match in the whole text.
    pub start: usize,
    pub end: usize,
    /// Line the match begins on, counting from 1 -- the numbering `goto-line`
    /// takes, so the two compose.
    pub line: usize,
    /// Character column the match begins at, counting from 0, like
    /// `current-column`.
    pub column: usize,
    /// The whole line the match begins on, without its newline. What a view
    /// shows, so that showing one costs no further reading of the text.
    pub line_text: String,
    /// What each group captured, group 0 being the whole match. Empty for a
    /// literal pattern, which has none.
    pub groups: Vec<Option<String>>,
}

impl Found {
    /// Returns where the match starts within [`Found::line_text`], in
    /// characters: [`Found::column`], named for what a view does with it.
    pub fn in_line(&self) -> usize {
        self.column
    }

    /// Returns where the match ends within [`Found::line_text`], clamped to
    /// it: a match may run past the end of its first line.
    pub fn end_in_line(&self) -> usize {
        let length = self.line_text.chars().count();
        (self.column + (self.end - self.start)).min(length)
    }
}

/// What a scan found, and whether a limit stopped it before the end. A caller
/// that ignores `truncated` shows a partial list as though it were everything.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub found: Vec<Found>,
    pub truncated: bool,
}

/// Walks a string once, turning byte offsets into character offsets, lines and
/// columns as it goes. Matches arrive in order, so the cursor only moves
/// forward, and a whole scan counts each character once.
struct Cursor<'a> {
    haystack: &'a str,
    byte: usize,
    chars: usize,
    line: usize,
    /// Byte offset of the first character of the line the cursor is on.
    line_start: usize,
}

impl<'a> Cursor<'a> {
    /// Returns a cursor at the start of HAYSTACK.
    fn new(haystack: &'a str) -> Self {
        Self {
            haystack,
            byte: 0,
            chars: 0,
            line: 1,
            line_start: 0,
        }
    }

    /// Moves to BYTE, which must be at or after the cursor.
    fn advance_to(&mut self, byte: usize) {
        for (at, c) in self.haystack[self.byte..byte].char_indices() {
            self.chars += 1;
            if c == '\n' {
                self.line += 1;
                self.line_start = self.byte + at + c.len_utf8();
            }
        }
        self.byte = byte;
    }

    /// Returns how many characters lie between the start of this line and the
    /// cursor.
    fn column(&self) -> usize {
        // TODO(RIC-4): counted from the line's start at every match, which is
        // quadratic on one long line with many matches.
        self.haystack[self.line_start..self.byte].chars().count()
    }

    /// Returns the line the cursor is on, without its newline.
    fn line_text(&self) -> &'a str {
        let rest = &self.haystack[self.line_start..];
        match rest.find('\n') {
            Some(at) => &rest[..at],
            None => rest,
        }
    }
}

impl Pattern {
    /// Returns every match in HAYSTACK, in order, at most LIMIT of them.
    ///
    /// Matches do not overlap: the scan resumes at the end of the one it
    /// found, and one character past an empty one, or a pattern like `x*`
    /// would keep it at one position for ever.
    pub fn scan(&self, haystack: &str, limit: usize) -> Scan {
        let mut scan = Scan::default();
        if limit == 0 || haystack.is_empty() {
            return scan;
        }
        let mut cursor = Cursor::new(haystack);
        // The line whose text was last built, so that twenty matches on one
        // line do not cost twenty walks to its end.
        let mut cached: Option<(usize, String)> = None;
        let mut at = 0usize;
        while at <= haystack.len() {
            let Some((start, end, groups)) = self.next_match(haystack, at) else {
                break;
            };
            if scan.found.len() >= limit {
                scan.truncated = true;
                break;
            }
            cursor.advance_to(start);
            let column = cursor.column();
            let line = cursor.line;
            let line_text = match &cached {
                Some((cached_line, text)) if *cached_line == line => text.clone(),
                _ => {
                    let text = cursor.line_text().to_string();
                    cached = Some((line, text.clone()));
                    text
                }
            };
            let start_chars = cursor.chars;
            let end_chars = start_chars + haystack[start..end].chars().count();
            scan.found.push(Found {
                start: start_chars,
                end: end_chars,
                line,
                column,
                line_text,
                groups,
            });
            // Past this match, and past *something* when it was empty.
            at = if end > start {
                end
            } else {
                match haystack[end..].chars().next() {
                    Some(c) => end + c.len_utf8(),
                    None => break,
                }
            };
        }
        scan
    }

    /// Returns the next match at or after byte FROM, as (start, end, groups)
    /// in bytes.
    fn next_match(
        &self,
        haystack: &str,
        from: usize,
    ) -> Option<(usize, usize, Vec<Option<String>>)> {
        match self {
            Pattern::Regex(regex) => {
                // `captures_at` rather than a slice, so `^` and a look-behind
                // still see what came before the resume point.
                let captures = regex.captures_at(haystack, from)?;
                let whole = captures.get(0)?;
                Some((
                    whole.start(),
                    whole.end(),
                    captures
                        .iter()
                        .map(|group| group.map(|m| m.as_str().to_string()))
                        .collect(),
                ))
            }
            Pattern::Literal { chars, fold } => {
                let mut at = from;
                while at <= haystack.len() {
                    if !haystack.is_char_boundary(at) {
                        at += 1;
                        continue;
                    }
                    if let Some(end) = literal_at(&haystack[at..], chars, *fold) {
                        return Some((at, at + end, Vec::new()));
                    }
                    match haystack[at..].chars().next() {
                        Some(c) => at += c.len_utf8(),
                        None => break,
                    }
                }
                None
            }
        }
    }
}

/// Returns the byte length of CHARS at the start of TEXT, or `None` if they
/// are not there.
fn literal_at(text: &str, chars: &[char], fold: bool) -> Option<usize> {
    let mut walked = text.chars();
    let mut bytes = 0;
    for &wanted in chars {
        let got = walked.next()?;
        if !chars_equal(got, wanted, fold) {
            return None;
        }
        bytes += got.len_utf8();
    }
    Some(bytes)
}

// ---------------------------------------------------------------------------
// Compiling the same pattern twice
// ---------------------------------------------------------------------------

/// Returns PATTERN compiled, from this thread's cache when it has been compiled
/// before: `string-match` asks at every call, and modules call it for every
/// element of a list.
///
/// Thread-local, so that matching takes no lock. Cleared whole past
/// [`CACHE_LIMIT`]: the patterns a program uses are tens, and the limit only
/// stops one that makes patterns from its input from growing it without
/// bound.
pub fn compiled(pattern: &str) -> Result<Arc<Regex>, String> {
    CACHE.with(|cache| {
        if let Some(found) = cache.borrow().get(pattern) {
            return Ok(found.clone());
        }
        let compiled = Arc::new(Regex::new(pattern).map_err(|why| why.to_string())?);
        let mut cache = cache.borrow_mut();
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(pattern.to_string(), compiled.clone());
        Ok(compiled)
    })
}

/// How many compiled patterns one thread keeps. See [`compiled`].
const CACHE_LIMIT: usize = 512;

thread_local! {
    /// This thread's compiled patterns, by source.
    static CACHE: RefCell<HashMap<String, Arc<Regex>>> = RefCell::new(HashMap::new());
}

/// Returns whether CHARS appears in TEXT at offset AT.
fn matches_at<B: BufferTrait>(text: &B, at: usize, chars: &[char], fold: bool) -> bool {
    chars.iter().enumerate().all(|(offset, &wanted)| {
        text.at(at + offset)
            .is_some_and(|got| chars_equal(got, wanted, fold))
    })
}

/// Returns whether two characters are equal, ignoring case when FOLD is set.
///
/// `to_lowercase` rather than `eq_ignore_ascii_case`, so that "straße" or
/// "ÅNGSTRÖM" folds as the rest of the text does; one character can lower-case
/// to several.
fn chars_equal(a: char, b: char, fold: bool) -> bool {
    a == b || (fold && a.to_lowercase().eq(b.to_lowercase()))
}

/// Returns the byte offset of the CHARS'th character, or the string's length
/// when CHARS is exactly its character count. `None` past that: a search
/// resumed beyond the end of the text has nothing to find.
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

/// Returns the character offset of a byte offset.
fn byte_to_char(haystack: &str, byte: usize) -> usize {
    haystack[..byte].chars().count()
}

/// Returns a regexp capture set as the character-offset [`Match`] the rest of
/// the editor speaks.
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
