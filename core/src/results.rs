//! A list of places, and what produced it.
//!
//! # What this is
//!
//! Occur, grep, a compiler's complaints, a linter's: all of them are a list of
//! *somewhere to go*, and every one of them has been written twice in every
//! editor -- once as a way to produce the list and once as a way to walk it.
//! This is the list, said once, so that producing and walking are separate
//! jobs that know nothing about each other.
//!
//! An entry says where, and enough to show the line without reading anything
//! again. The shape is fixed in [`crate::primitives::scan`], where Lisp sees
//! it, and mirrored here.
//!
//! # Where a set lives
//!
//! On the buffer that shows it, in [`crate::buffer::Buffer::data`]. That is
//! not an implementation detail -- it is the lifetime: kill the buffer and the
//! results go with it, because they *are* the buffer's, and nothing has to
//! remember to tidy up. The alternative, a table of sets keyed by buffer name,
//! is wrong the moment a buffer is renamed and still holding memory long after
//! it is killed.
use crate::search::Found;

/// The name a result set is attached to its buffer under.
pub const RESULTS_KEY: &str = "results";

/// What kind of thing an entry is in.
pub const KIND_BUFFER: &str = "buffer";
/// The other kind.
pub const KIND_FILE: &str = "file";

/// One place, and what is there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// `"buffer"` or `"file"` -- a name like `*scratch*` and a path are
    /// otherwise only distinguishable by guessing.
    pub kind: String,
    /// The buffer's name, or the file's path.
    pub source: String,
    /// Counting from 1, the numbering `goto-line` takes.
    pub line: usize,
    /// Counting from 0, like `current-column`.
    pub column: usize,
    /// Character offset in the source, for `goto-char`.
    pub offset: usize,
    /// The whole line, so showing it costs no further reading.
    pub text: String,
    /// Where the match sits within `text`, so a view highlights it without
    /// searching the line again -- and cannot disagree with the scan about
    /// where it was.
    pub start: usize,
    pub end: usize,
}

impl Entry {
    /// An entry for FOUND, which was found in SOURCE.
    pub fn new(kind: &str, source: &str, found: &Found) -> Self {
        Self {
            kind: kind.to_string(),
            source: source.to_string(),
            line: found.line,
            column: found.column,
            offset: found.start,
            text: found.line_text.clone(),
            start: found.in_line(),
            end: found.end_in_line(),
        }
    }

    /// The line a plain view shows, in the `file:line: text` shape every
    /// grep-like tool has printed since the first one.
    pub fn rendered(&self) -> String {
        format!("{}:{}: {}", self.source, self.line, self.text)
    }
}

/// Everything one search found, and where the walk through it has got to.
#[derive(Clone, Debug, Default)]
pub struct Results {
    /// What was searched for, as it was typed. For the header line, and so a
    /// view can say what it is showing.
    pub pattern: String,
    /// What the search was over, in words: `"this buffer"`, a directory.
    pub over: String,
    pub entries: Vec<Entry>,
    /// Whether a limit stopped the search before the end. A caller that
    /// ignores this shows a partial list as though it were everything.
    pub truncated: bool,
    /// Whether the search has finished. False while a scan is still running,
    /// which is what lets a view say so rather than look empty.
    pub done: bool,
    /// Where the mark was last put, so it can be taken off again. The buffer
    /// visited before this one is not the buffer this is called from, and a
    /// mark left behind in a file you have moved on from is a line highlighted
    /// for no reason anybody can remember.
    pub marked: Option<String>,
    /// Which entry was last visited, if any. Walking is relative to this, so
    /// `next-error` continues from where you are rather than from the top.
    pub current: Option<usize>,
}

impl Results {
    pub fn new(pattern: &str, over: &str) -> Self {
        Self {
            pattern: pattern.to_string(),
            over: over.to_string(),
            ..Self::default()
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index)
    }

    /// The index one STEP away from where the walk is, or `None` at the end.
    ///
    /// Not wrapped, deliberately. A list of places is walked *through*: the
    /// end of it is information -- "that was the last one" -- and a walk that
    /// silently started again would have you fixing the first error twice
    /// while believing you were making progress.
    pub fn step(&self, step: isize) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        let next = match self.current {
            // Before anything has been visited, forwards means the first and
            // backwards means the last, so both directions have somewhere to
            // start.
            None if step >= 0 => 0,
            None => self.entries.len() - 1,
            Some(current) => {
                let next = current as isize + step;
                if next < 0 || next as usize >= self.entries.len() {
                    return None;
                }
                next as usize
            }
        };
        Some(next)
    }
}
