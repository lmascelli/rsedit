//! The syntax table: what each character means to the scanner of
//! `modes/sexp.rs`, and the strings, comments and character literals of a
//! language.
//!
//! Design: `doc/architettura-editor.typ, <tabella-sintattica>`.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

/// What one character means to the scanner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxClass {
    Open(char),  // carries the character that closes it
    Close(char), // carries the character that opened it
    StringQuote, // the same character ends it
    Escape,      // the next character is literal
    Prefix,      // attaches to the expression after it: ' ` , #
    Symbol,
    Punctuation, // separates, belongs to nothing
}

/// One way the language writes a string whose delimiters are more than one
/// character: Rust's raw strings, Python's triple quotes.
#[derive(Clone, Debug)]
pub struct StringStyle {
    pub opener: String,
    pub closer: String,
    /// Whether a backslash escapes the character after it, as in Python's
    /// triple quotes and TOML's multiline basic strings; in a raw string it
    /// does not.
    pub escaped: bool,
}

/// One way this language writes a comment.
#[derive(Clone, Debug)]
pub enum CommentStyle {
    Line {
        opener: String,
    },
    Block {
        opener: String,
        closer: String,
        nestable: bool,
    },
}

/// What the characters of a language mean to the scanner, and the strings,
/// comments and character literals the language has.
#[derive(Clone, Debug)]
pub struct SyntaxTable {
    classes: HashMap<char, SyntaxClass>,
    comments: Vec<CommentStyle>,
    /// First character of every comment opener, so the scan asks "could a
    /// comment begin here?" with one set lookup rather than a string compare
    /// per style per position.
    comment_starts: HashSet<char>,
    /// Strings whose delimiters are more than one character, tried before the
    /// single-character `StringQuote` and the longest opener first, so that a
    /// raw string's opener wins over the bare quote it begins with.
    strings: Vec<StringStyle>,
    /// First character of every string-style opener, so the scan asks "could
    /// one begin here?" with one set lookup, as with `comment_starts`.
    string_starts: HashSet<char>,
    /// The character that encloses a single literal character, if the
    /// language has one: `'` in Rust, C, C++ and Java. Not a class, since what
    /// it is depends on what follows it: in Rust, `'static` is a lifetime.
    char_quote: Option<char>,
    /// Which edition of a table this is. See [`SyntaxTable::edition`].
    edition: u64,
}

impl SyntaxTable {
    /// Returns which edition of a table this is: 0 for the default table, and
    /// after every change a number no table has had before.
    ///
    /// Two tables with the same edition are the same table, which is what lets
    /// a scan checkpoint say which table it was taken with and be refused under
    /// any other: see [`crate::buffer::scan::ScanCache`]. The mode cannot say
    /// it, since a mode's table changes under it.
    pub fn edition(&self) -> u64 {
        self.edition
    }

    /// Gives the table an edition no table has had. Every change calls it.
    fn changed(&mut self) {
        static EDITIONS: AtomicU64 = AtomicU64::new(1);
        self.edition = EDITIONS.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns what C means to the scanner: its class in the table, or else a
    /// symbol constituent for a letter, a digit or `_`, and punctuation.
    pub fn class_of(&self, c: char) -> SyntaxClass {
        if let Some(class) = self.classes.get(&c) {
            return *class;
        }
        if c.is_alphanumeric() || c == '_' {
            SyntaxClass::Symbol
        } else {
            SyntaxClass::Punctuation
        }
    }
}

impl Default for SyntaxTable {
    fn default() -> Self {
        let mut classes = HashMap::new();
        classes.insert('(', SyntaxClass::Open(')'));
        classes.insert(')', SyntaxClass::Close('('));
        classes.insert('[', SyntaxClass::Open(']'));
        classes.insert(']', SyntaxClass::Close('['));
        classes.insert('{', SyntaxClass::Open('}'));
        classes.insert('}', SyntaxClass::Close('{'));
        classes.insert('\"', SyntaxClass::StringQuote);
        classes.insert('\\', SyntaxClass::Escape);

        Self {
            classes,
            comments: Vec::new(),
            comment_starts: HashSet::new(),
            strings: Vec::new(),
            string_starts: HashSet::new(),
            char_quote: None,
            edition: 0,
        }
    }
}

impl SyntaxTable {
    /// Makes each pair of characters of PAIRS an opener and the closer that
    /// matches it.
    pub fn set_pairs(&mut self, pairs: &str) {
        let chars: Vec<char> = pairs.chars().collect();
        for pair in chars.chunks(2) {
            if let [open, close] = pair {
                self.classes.insert(*open, SyntaxClass::Open(*close));
                self.classes.insert(*close, SyntaxClass::Close(*open));
            }
        }
        self.changed();
    }

    /// Sets the character that encloses a single literal character, or none.
    pub fn set_char_quote(&mut self, c: Option<char>) {
        self.char_quote = c;
        self.changed();
    }

    /// Returns where the character literal beginning at AT ends, just past its
    /// closing quote, reading the text with CHAR_AT up to LEN.
    ///
    /// `None` when the language has no character quote, when AT does not hold
    /// it, or when it does not close after one character, escaped or not. That
    /// is what tells `'a'` from `'a`: a lifetime never closes, and its quote is
    /// left to be punctuation.
    pub fn char_literal_at<F>(&self, at: usize, len: usize, char_at: F) -> Option<usize>
    where
        F: Fn(usize) -> Option<char>,
    {
        let quote = self.char_quote?;
        if char_at(at) != Some(quote) {
            return None;
        }
        // `'x'` is three characters, `'\n'` is four. Nothing longer is a
        // character literal in any language that has them.
        let escaped = matches!(self.class_of(char_at(at + 1)?), SyntaxClass::Escape);
        let end = if escaped { at + 3 } else { at + 2 };
        if end < len && char_at(end) == Some(quote) {
            Some(end + 1)
        } else {
            None
        }
    }

    /// Gives C the class CLASS.
    pub fn set_class(&mut self, c: char, class: SyntaxClass) {
        self.classes.insert(c, class);
        self.changed();
    }

    /// Sets the strings with delimiters longer than one character that the
    /// language has, in place of those set before.
    pub fn set_strings(&mut self, strings: Vec<StringStyle>) {
        self.string_starts = strings
            .iter()
            .filter_map(|style| style.opener.chars().next())
            .collect();
        self.strings = strings;
        self.changed();
    }

    /// Returns the strings with delimiters longer than one character.
    pub fn strings(&self) -> &[StringStyle] {
        &self.strings
    }

    /// Returns whether a string style could begin with C.
    pub fn could_begin_string(&self, c: char) -> bool {
        self.string_starts.contains(&c)
    }

    /// Sets the comment styles of the language, in place of those set before.
    pub fn set_comments(&mut self, comments: Vec<CommentStyle>) {
        self.comment_starts = comments
            .iter()
            .filter_map(|style| match style {
                CommentStyle::Line { opener } | CommentStyle::Block { opener, .. } => {
                    opener.chars().next()
                }
            })
            .collect();
        self.comments = comments;
        self.changed();
    }

    /// Returns the comment styles of the language.
    pub fn comments(&self) -> &[CommentStyle] {
        &self.comments
    }

    /// Returns whether a comment style could begin with C.
    pub fn may_start_comment(&self, c: char) -> bool {
        self.comment_starts.contains(&c)
    }
}
