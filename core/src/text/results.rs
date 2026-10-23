//! A list of places -- what occur, grep and a compiler's complaints produce --
//! and a cursor that walks it, so that producing a list and walking it are
//! separate jobs. A set lives on the buffer that shows it, in
//! [`crate::buffer::Buffer::data`], and goes when the buffer does.
//!
//! Design: `doc/architettura-editor.typ, <risultati>`.
use crate::text::search::Found;

/// The name a result set is attached to its buffer under.
pub const RESULTS_KEY: &str = "results";

/// The kind of an entry found in a buffer.
pub const KIND_BUFFER: &str = "buffer";
/// The kind of an entry found in a file.
pub const KIND_FILE: &str = "file";

// TODO(RIC-5): an entry keeps a line and an offset, which an edit above the
// place moves away from.

/// One place, and what is there: the shape the `scan` primitive hands to Lisp.
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
    /// The line of the listing this entry is shown on, counting from 1, when
    /// its producer says. A search's listing has a header line and then an
    /// entry per line, so it need not; a compilation's output names places
    /// on whichever lines the compiler wrote them, so it must.
    pub at: Option<usize>,
}

impl Entry {
    /// Returns an entry of KIND for FOUND, which was found in SOURCE.
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
            at: None,
        }
    }

    /// Returns the line a plain view shows: `file:line: text`, as every
    /// grep-like tool prints it.
    pub fn rendered(&self) -> String {
        format!("{}:{}: {}", self.source, self.line, self.text)
    }
}

/// Everything one search found, and how far the walk through it has got.
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
    /// Creates an empty set for PATTERN, searched over OVER.
    pub fn new(pattern: &str, over: &str) -> Self {
        Self {
            pattern: pattern.to_string(),
            over: over.to_string(),
            ..Self::default()
        }
    }

    /// Returns how many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns the entry at INDEX.
    pub fn get(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index)
    }

    /// Returns the index one STEP away from where the walk is, or `None` past
    /// either end. Not wrapped: the end of a list of places says that was the
    /// last one.
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
