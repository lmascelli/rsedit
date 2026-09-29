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

// ---------------------------------------------------------------------------
// Scanning for every match
// ---------------------------------------------------------------------------
//
// # Why this is not `search_forward` in a loop
//
// `search_forward` answers "where is the next one" and is called once per
// keystroke, which is what it is shaped for. On the regexp path it stringifies
// the whole text *every time it is called* -- see the comment there -- so
// asking it for every match in a buffer costs a pass over the buffer per
// match. On a file with a thousand hits that is a thousand passes, and the
// cost grows with the square of the file.
//
// Scanning is the other question, and it is asked by things that mean to walk
// everything: occur, grep, a linter reading its own output. One pass, one
// stringification, and the line each match is on worked out as the pass goes
// by rather than by searching backwards for a newline afterwards.

/// One match, with where it sits in the text it was found in.
///
/// Offsets are in *characters*, because that is what the rest of the editor
/// means by a position -- `goto-char` takes one, and a match reported in bytes
/// would land in the middle of a letter in any file that is not ASCII.
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
    /// Where the match starts within [`Found::line_text`], in characters.
    ///
    /// The same number as [`Found::column`], named for what a view does with
    /// it: highlight the match inside the line it is showing.
    pub fn in_line(&self) -> usize {
        self.column
    }

    /// Where the match ends within [`Found::line_text`], clamped to it.
    ///
    /// A match may run past the end of its first line -- a regexp with a `\n`
    /// in it, or `(?s).` -- and a view highlighting the line it shows must not
    /// be handed an end beyond the text it has.
    pub fn end_in_line(&self) -> usize {
        let length = self.line_text.chars().count();
        (self.column + (self.end - self.start)).min(length)
    }
}

/// What a scan found, and whether it stopped before the end.
///
/// `truncated` is not a detail: a caller that ignores it shows a partial list
/// as though it were everything, which is how a search comes to quietly not
/// find what is there. The same shape `directory-files-recursive` answers in,
/// for the same reason.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub found: Vec<Found>,
    pub truncated: bool,
}

/// Walks a string once, converting byte offsets into character offsets, lines
/// and columns as it goes.
///
/// The whole reason a scan is linear. Matches arrive in increasing order, so
/// the cursor only ever moves forward: across a whole file it counts each
/// character exactly once, where asking "which line is byte N on" per match
/// would re-count the file from the start every time.
struct Cursor<'a> {
    haystack: &'a str,
    byte: usize,
    chars: usize,
    line: usize,
    /// Byte offset of the first character of the line the cursor is on.
    line_start: usize,
}

impl<'a> Cursor<'a> {
    fn new(haystack: &'a str) -> Self {
        Self {
            haystack,
            byte: 0,
            chars: 0,
            line: 1,
            line_start: 0,
        }
    }

    /// Advance to BYTE, which must be at or after the cursor.
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

    /// Characters between the start of this line and the cursor.
    fn column(&self) -> usize {
        self.haystack[self.line_start..self.byte].chars().count()
    }

    /// The line the cursor is on, without its newline.
    fn line_text(&self) -> &'a str {
        let rest = &self.haystack[self.line_start..];
        match rest.find('\n') {
            Some(at) => &rest[..at],
            None => rest,
        }
    }
}

impl Pattern {
    /// Every match in HAYSTACK, in order, at most LIMIT of them.
    ///
    /// Matches do not overlap: the scan resumes at the end of the one it just
    /// found, and one character past an *empty* match -- a pattern like `x*`
    /// matches the empty string at every position, and a loop that did not
    /// step past it would stay there for ever. The same rule the replace loop
    /// follows, for the same reason.
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

    /// The next match at or after byte FROM, as (start, end, groups) in bytes.
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

/// The byte length of CHARS at the start of TEXT, or `None` if it is not there.
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

// ---------------------------------------------------------------------------
// Replacing
// ---------------------------------------------------------------------------

/// How a replacement's letters should be cased, taken from what was matched.
///
/// # Why this is worth having at all
///
/// Because a case-insensitive search that replaced literally is a search that
/// is right about *where* and wrong about *what*: replacing `colour` with
/// `color` through a document turns `Colour` at the start of a sentence into
/// `color`, and every one of those has to be fixed by hand afterwards. It is
/// the single thing people most notice missing from a replace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Casing {
    /// The match had no letters, or its letters disagree about case. The
    /// replacement goes in exactly as it was written.
    AsWritten,
    /// Every cased letter in the match was upper: `FOO` -> `BAR`.
    Upper,
    /// The first cased letter was upper and the rest were not: `Foo` -> `Bar`.
    Capitalised,
}

impl Casing {
    /// What TEXT's own casing is.
    ///
    /// Only a match written *entirely* one way says anything. `fooBar` is
    /// somebody's identifier and re-casing it would be vandalism, so it reads
    /// as [`Casing::AsWritten`] -- as does anything with no letters in it at
    /// all, which is most matches in a regexp replace.
    pub fn of(text: &str) -> Self {
        let mut letters = text.chars().filter(|c| c.is_alphabetic());
        let Some(first) = letters.next() else {
            return Casing::AsWritten;
        };
        let rest: Vec<char> = letters.collect();
        if first.is_uppercase() {
            if rest.iter().all(|c| c.is_uppercase()) && !rest.is_empty() {
                return Casing::Upper;
            }
            if rest.iter().all(|c| c.is_lowercase()) {
                return Casing::Capitalised;
            }
        }
        Casing::AsWritten
    }

    /// Apply this casing to TEXT.
    pub fn apply(self, text: &str) -> String {
        match self {
            Casing::AsWritten => text.to_string(),
            Casing::Upper => text.to_uppercase(),
            Casing::Capitalised => {
                let mut chars = text.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                    None => String::new(),
                }
            }
        }
    }
}

/// What REPLACEMENT becomes for this particular match: group references
/// filled in, and the casing taken from the matched text if asked for.
///
/// `\1` to `\9` are the capture groups, `\0` and `\&` the whole match, and
/// `\\` a literal backslash. A reference to a group that did not take part --
/// or to any group at all, under a literal pattern, which has none -- expands
/// to nothing, which is the only answer that is never wrong.
///
/// MATCHED is the text the pattern actually found. It is passed in rather than
/// read out of `found.groups`, because a literal match records no groups at
/// all: taking the whole match from group zero would have made `\&` and
/// case-preservation work under a regexp and silently do nothing under a plain
/// search, which is the one people use.
pub fn expand_replacement(
    replacement: &str,
    found: &Match,
    matched: &str,
    preserve_case: bool,
) -> String {
    let mut out = String::with_capacity(replacement.len());
    let mut chars = replacement.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            // The whole match, taken from the text rather than from group
            // zero: a literal pattern has no groups at all, and `\&' means the
            // same thing whichever kind of pattern found it.
            Some('&') | Some('0') => out.push_str(matched),
            Some(digit) if digit.is_ascii_digit() => {
                push_group(&mut out, found, digit as usize - '0' as usize)
            }
            // A backslash before anything else is a backslash. Refusing would
            // mean a replacement containing a Windows path could not be typed.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    if preserve_case {
        return Casing::of(matched).apply(&out);
    }
    out
}

fn push_group(out: &mut String, found: &Match, index: usize) {
    if let Some(Some(text)) = found.groups.get(index) {
        out.push_str(text);
    }
}

/// Where one accepted replacement began, and what the scan's limit was before
/// it happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub from: usize,
    pub limit: usize,
}

/// A replace in progress: what to look for, what to put there, and how far it
/// has got.
///
/// # Why this is here and not beside the editor
///
/// The same reason [`Isearch`] is: it needs a haystack that can hand over
/// characters by position and nothing else. Keeping the arithmetic of a
/// replace -- where the next scan starts, how a replacement moves everything
/// after it, when to stop -- above the editor means it can be tested against a
/// plain buffer, and means the interactive and non-interactive replaces cannot
/// drift into two ideas of what a match is.
///
/// # Why it is a session and not a loop
///
/// Because a *view* drives it. A terminal asks with single keys, a graphical
/// one with buttons, and neither can be expressed as a loop that reads an
/// answer -- there is no blocking read in this editor, and a button press is
/// not a read at all. So the loop is turned inside out: this holds the place
/// in it, and whoever is asking calls one verb per answer.
#[derive(Clone, Debug)]
pub struct Replace {
    /// The buffer being changed. Not the current buffer -- while the prompt
    /// that collected the arguments was open, the current buffer was that
    /// prompt.
    pub buffer: String,
    pub pattern: Pattern,
    /// As typed, with its group references still in it: they are filled in per
    /// match, so this is the template rather than any particular answer.
    pub replacement: String,
    /// Where point was when it began, restored if it is abandoned.
    pub origin: usize,
    /// Where the next scan starts.
    pub from: usize,
    /// How far the replace may reach: the region's end, or the buffer's.
    ///
    /// Moved by every replacement, because a replacement that is longer or
    /// shorter than what it replaced moves everything after it -- including
    /// the end of the region being worked in. Without that a replace confined
    /// to a region would creep past its end, or stop short of it.
    pub limit: usize,
    /// The match being offered, if there is one.
    pub found: Option<Match>,
    pub replaced: usize,
    /// Where each accepted replacement began, and what the limit was before
    /// it, most recent last.
    ///
    /// What makes stepping back possible. Undoing puts the text back; this
    /// puts the *scan* back, which the text cannot say -- the position a
    /// replacement started at is gone the moment its replacement is a
    /// different length.
    pub history: Vec<Step>,
    /// Whether the letters of the replacement follow the match's.
    pub preserve_case: bool,
    /// Set once there is nothing left to find.
    pub done: bool,
}

impl Replace {
    pub fn new(
        buffer: String,
        pattern: Pattern,
        replacement: String,
        origin: usize,
        limit: usize,
        preserve_case: bool,
    ) -> Self {
        Self {
            buffer,
            pattern,
            replacement,
            origin,
            from: origin,
            limit,
            found: None,
            replaced: 0,
            history: Vec::new(),
            preserve_case,
            done: false,
        }
    }

    /// What the current match should become, given the text it actually
    /// matched.
    pub fn expansion(&self, matched: &str) -> Option<String> {
        self.found
            .as_ref()
            .map(|found| expand_replacement(&self.replacement, found, matched, self.preserve_case))
    }

    /// Look for the next match from `from`, and record it.
    pub fn seek<B: BufferTrait>(&mut self, text: &B) {
        self.found = self.pattern.search_forward(text, self.from, self.limit);
        if self.found.is_none() {
            self.done = true;
        }
    }

    /// Account for the current match having been replaced by NEW_LEN
    /// characters: the scan resumes after it, and everything after it moved.
    ///
    /// The resume point is past the *replacement*, not past the match, and it
    /// never stands still. An empty match -- which `a*` finds everywhere --
    /// would otherwise be found at the same position forever, and a
    /// replacement that contains what it replaced would be replaced again by
    /// the next scan.
    pub fn accept(&mut self, new_len: usize) {
        let Some(found) = self.found.take() else {
            return;
        };
        let old_len = found.end - found.start;
        self.history.push(Step {
            from: found.start,
            limit: self.limit,
        });
        self.limit = (self.limit + new_len).saturating_sub(old_len);
        self.from = if new_len == 0 && old_len == 0 {
            found.start + 1
        } else {
            found.start + new_len
        };
        self.replaced += 1;
    }

    /// Take the last accepted replacement back: the scan returns to where it
    /// began and the count goes down. False when there is none.
    ///
    /// Puts back only what *this* knows about. The text is the caller's to
    /// restore, because the undo history is the buffer's and this file knows
    /// nothing about buffers.
    pub fn unaccept(&mut self) -> bool {
        let Some(step) = self.history.pop() else {
            return false;
        };
        self.from = step.from;
        self.limit = step.limit;
        self.replaced = self.replaced.saturating_sub(1);
        self.found = None;
        self.done = false;
        true
    }

    /// Leave the current match alone and resume after it.
    pub fn decline(&mut self) {
        let Some(found) = self.found.take() else {
            return;
        };
        // Past a real match, and one character past an empty one -- which is
        // the only way to leave a pattern like `a*` behind, since it matches
        // at every position including the one just declined.
        self.from = if found.end > found.start {
            found.end
        } else {
            found.start + 1
        };
    }
}
