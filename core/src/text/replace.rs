//! A replace in progress: what to look for, what to put there and how far it
//! has got, and how a replacement's letters follow the match's. A view drives
//! it, one verb per answer; the text itself is changed by the editor.
//!
//! Design: `doc/architettura-editor.typ, <sostituzione>`.
use crate::buffer::BufferTrait;
use crate::text::search::{Match, Pattern};

/// How a replacement's letters are cased, taken from what was matched.
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
    /// Returns TEXT's own casing.
    ///
    /// Only a match written entirely one way says anything: `fooBar` is
    /// somebody's identifier, and reads as [`Casing::AsWritten`], as does
    /// anything with no letters in it.
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

    /// Returns TEXT with this casing applied.
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

/// Returns what REPLACEMENT becomes for this match: group references filled
/// in, and the casing taken from the matched text when PRESERVE_CASE is set.
///
/// `\1` to `\9` are the capture groups, `\0` and `\&` the whole match, and
/// `\\` a literal backslash. A reference to a group that did not take part --
/// or to any group, under a literal pattern, which has none -- expands to
/// nothing.
///
/// MATCHED, the text the pattern found, is passed in rather than read from
/// `found.groups`: a literal match records no groups, and `\&` and the casing
/// have to work under it too.
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

/// Appends to OUT what group INDEX of FOUND captured, if anything.
fn push_group(out: &mut String, found: &Match, index: usize) {
    if let Some(Some(text)) = found.groups.get(index) {
        out.push_str(text);
    }
}

/// Where one accepted replacement began, and what the scan's limit was before
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub from: usize,
    pub limit: usize,
}

/// A replace in progress: what to look for, what to put there, and how far it
/// has got.
///
/// A view drives it, one verb per answer -- single keys in a terminal, buttons
/// in a graphical front-end -- since neither can be a loop that reads an
/// answer.
#[derive(Clone, Debug)]
pub struct Replace {
    /// The buffer being changed. Not the current buffer: while the prompt that
    /// collected the arguments was open, the current buffer was that prompt.
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
    /// Moved by every replacement, which moves everything after it, the end
    /// of the region included.
    pub limit: usize,
    /// The match being offered, if there is one.
    pub found: Option<Match>,
    /// How many replacements have been accepted.
    pub replaced: usize,
    /// Where each accepted replacement began, and what the limit was before
    /// it, most recent last: what puts the scan back on a step back, which the
    /// text cannot say once a replacement has changed its length.
    pub history: Vec<Step>,
    /// Whether the letters of the replacement follow the match's.
    pub preserve_case: bool,
    /// Set once there is nothing left to find.
    pub done: bool,
}

impl Replace {
    /// Creates a replace of PATTERN by REPLACEMENT in BUFFER, from ORIGIN up
    /// to LIMIT.
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

    /// Returns what the current match becomes, given the text it matched.
    pub fn expansion(&self, matched: &str) -> Option<String> {
        self.found
            .as_ref()
            .map(|found| expand_replacement(&self.replacement, found, matched, self.preserve_case))
    }

    /// Looks for the next match from `from`, and records it; when there is
    /// none, the replace is done.
    pub fn seek<B: BufferTrait>(&mut self, text: &B) {
        self.found = self.pattern.search_forward(text, self.from, self.limit);
        if self.found.is_none() {
            self.done = true;
        }
    }

    /// Accounts for the current match having been replaced by NEW_LEN
    /// characters: the scan resumes after the replacement, and the limit moves
    /// by the difference.
    ///
    /// Past the replacement rather than the match, so that a replacement that
    /// contains what it replaced is not replaced again.
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
        // TODO(RIC-1): an empty match replaced by something resumes just after
        // the replacement, where the same empty match is found again.
        self.from = if new_len == 0 && old_len == 0 {
            found.start + 1
        } else {
            found.start + new_len
        };
        self.replaced += 1;
    }

    /// Takes the last accepted replacement back: the scan returns to where it
    /// began and the count goes down. False when there is none.
    ///
    /// Only the scan: the text is the caller's to restore, with the buffer's
    /// undo history.
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

    /// Leaves the current match alone, and resumes after it.
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
