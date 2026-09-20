//! Balanced-expression scanning, driven by a mode's syntax table.
//!
//! # Why this cannot be a paren count
//!
//! Three of the four parentheses in `(message "close it with )")` are text.
//! Any answer that doesn't know where strings and comments are is wrong, and
//! wrong *silently*  the cursor just lands somewhere strange. So this lexes:
//! a three-state machine that never counts a delimiter it is not currently
//! reading as code.
//!
//! # Why every scan starts at the top of the buffer
//!
//! Moving forward could be done locally. Moving backward cannot: whether the
//! `)` before point is code or text depends on what came before it, arbitrarily
//! far back  a string opened two hundred lines up makes it text  and the same
//! characters read right-to-left are ambiguous.
//!
//! So both directions run the same forward lex from the top. That is linear in
//! the distance from the start of the buffer rather than the distance moved,
//! which is the price of being right. [`Scan::begin`] is the single place that
//! decides, so when this is worth optimising the fix goes there and no caller
//! changes: `crate::buffer::syntax` already caches a lexer state per line for
//! colouring, and the same trick applies here.
use crate::BufferTrait;
use crate::modes::{CommentStyle, SyntaxClass, SyntaxTable};
use std::collections::VecDeque;

/// One expression: where it begins  prefix included  and where it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sexp {
    pub start: usize,
    pub end: usize,
}

/// What point is inside. This is what `syntax-ppss` reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub depth: usize,
    /// Where the innermost still-open list begins.
    pub innermost: Option<usize>,
    /// Where that list's *delimiter* is, as opposed to where its expression
    /// begins.
    ///
    /// The two differ by any prefix: `'(a b)` is one expression starting at
    /// the quote, but a line inside it lines up against the parenthesis.
    /// Motion wants the first -- a kill should take the quote with it --
    /// and indentation wants the second, and neither can be worked out from
    /// the other without scanning again.
    pub innermost_delimiter: Option<usize>,
    pub string_start: Option<usize>,
    pub comment_start: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Code,
    Str {
        quote: char,
        start: usize,
    },
    Comment {
        style: usize,
        depth: usize,
        start: usize,
    },
    /// Inside a string whose delimiters are more than one character: a raw
    /// string, a triple-quoted one. Nothing is escaped in here; only the
    /// closer ends it.
    StyledStr {
        style: usize,
        start: usize,
    },
}

/// One level of nesting, and the last few expressions completed inside it.
struct Frame {
    /// Where the expression opening this level begins  the prefix, if there
    /// was one, rather than the delimiter: `'(a b)` opens at the quote.
    start: usize,
    /// Where the delimiter itself is. See `Context::innermost_delimiter`.
    delimiter: usize,
    /// Bounded, and that is the point: a whole file's expressions would be
    /// hundreds of thousands of entries rebuilt on every keystroke, while
    /// `backward-sexp` with a count of N needs exactly the last N.
    children: VecDeque<Sexp>,
}

/// What one step of the scan did.
enum Step {
    /// An expression finished, at this nesting depth.
    Completed(usize, Sexp),
    /// Something was consumed; nothing finished.
    Advanced,
    End,
}

pub struct Scan<'a, B: BufferTrait> {
    text: &'a B,
    table: &'a SyntaxTable,
    pos: usize,
    state: State,
    stack: Vec<Frame>,
    prefix: Option<usize>,
    keep: usize,
}

impl<'a, B: BufferTrait> Scan<'a, B> {
    /// Start a scan that will be asked about `before`.
    ///
    /// The one place that chooses a starting offset. Today it is always the top
    /// of the buffer in the base state, because that is the only offset whose
    /// state the scanner knows without having been told.
    pub fn begin(text: &'a B, table: &'a SyntaxTable, _before: usize, keep: usize) -> Self {
        Self {
            text,
            table,
            pos: 0,
            state: State::Code,
            // The outermost frame is the buffer itself. It is never popped, so
            // `stack.last_mut()` cannot be None and no caller has to invent an
            // answer for a file with more `)` than `(`.
            stack: vec![Frame {
                start: 0,
                delimiter: 0,
                children: VecDeque::new(),
            }],
            prefix: None,
            keep: keep.max(1),
        }
    }

    fn depth(&self) -> usize {
        self.stack.len() - 1
    }

    fn complete(&mut self, start: usize, end: usize) -> Step {
        let keep = self.keep;
        let frame = self
            .stack
            .last_mut()
            .expect("the root frame is never popped");
        frame.children.push_back(Sexp { start, end });
        while frame.children.len() > keep {
            frame.children.pop_front();
        }
        Step::Completed(self.stack.len() - 1, Sexp { start, end })
    }

    /// Where the expression starting at `at` really begins, taking any prefix
    /// waiting in front of it.
    fn opening(&mut self, at: usize) -> usize {
        self.prefix.take().unwrap_or(at)
    }

    fn literal_at(&self, at: usize, literal: &str) -> bool {
        literal
            .chars()
            .enumerate()
            .all(|(i, c)| self.text.at(at + i) == Some(c))
    }

    /// Which string style, if any, opens here.
    ///
    /// The longest opener wins, so a raw-string opener beats the plain quote
    /// it begins with. Same rule as `comment_at`, for the same reason: a
    /// shorter opener that is a prefix of a longer one would always win by
    /// position and the longer form would be unreachable.
    fn string_at(&self, at: usize) -> Option<usize> {
        let c = self.text.at(at)?;
        if !self.table.could_begin_string(c) {
            return None;
        }
        self.table
            .strings()
            .iter()
            .enumerate()
            .filter(|(_, style)| self.literal_at(at, &style.opener))
            .max_by_key(|(_, style)| style.opener.chars().count())
            .map(|(index, _)| index)
    }

    /// Which comment style, if any, opens here.
    fn comment_at(&self, at: usize) -> Option<usize> {
        let c = self.text.at(at)?;
        if !self.table.may_start_comment(c) {
            return None;
        }
        // Longest opener first, so `//` is not read as the `/` of something
        // shorter that happens to share its first character.
        self.table
            .comments()
            .iter()
            .enumerate()
            .filter(|(_, style)| {
                let opener = match style {
                    CommentStyle::Line { opener } => opener,
                    CommentStyle::Block { opener, .. } => opener,
                };
                self.literal_at(at, opener)
            })
            .max_by_key(|(_, style)| match style {
                CommentStyle::Line { opener } => opener.chars().count(),
                CommentStyle::Block { opener, .. } => opener.chars().count(),
            })
            .map(|(index, _)| index)
    }

    fn step(&mut self) -> Step {
        let len = self.text.len();
        if self.pos >= len {
            return Step::End;
        }
        let Some(c) = self.text.at(self.pos) else {
            return Step::End;
        };

        match self.state.clone() {
            State::Comment {
                style,
                depth,
                start,
            } => {
                match &self.table.comments()[style] {
                    CommentStyle::Line { .. } => {
                        if c == '\n' {
                            self.state = State::Code;
                        }
                        self.pos += 1;
                    }
                    CommentStyle::Block {
                        opener,
                        closer,
                        nestable,
                    } => {
                        if self.literal_at(self.pos, closer) {
                            self.pos += closer.chars().count();
                            if depth == 0 {
                                self.state = State::Code;
                                return self.complete_comment(start, self.pos);
                            }
                            self.state = State::Comment {
                                style,
                                depth: depth - 1,
                                start,
                            };
                        } else if *nestable && self.literal_at(self.pos, opener) {
                            self.pos += opener.chars().count();
                            self.state = State::Comment {
                                style,
                                depth: depth + 1,
                                start,
                            };
                        } else {
                            self.pos += 1;
                        }
                    }
                }
                Step::Advanced
            }

            State::StyledStr { style, start } => {
                let closer = &self.table.strings()[style].closer;
                if self.literal_at(self.pos, closer) {
                    self.pos += closer.chars().count();
                    self.state = State::Code;
                    return self.complete(start, self.pos);
                }
                // No escape handling, deliberately: a raw string exists so
                // that a backslash means a backslash. Only the closer ends it.
                self.pos += 1;
                Step::Advanced
            }

            State::Str { quote, start } => {
                match self.table.class_of(c) {
                    // Two characters at once: the whole reason `"a \" b"` does
                    // not end at the middle quote. Escapes are honoured inside
                    // strings only  a `\` at the end of a `//` comment does
                    // not continue it.
                    SyntaxClass::Escape => self.pos += 2,
                    _ if c == quote => {
                        self.pos += 1;
                        self.state = State::Code;
                        return self.complete(start, self.pos);
                    }
                    _ => self.pos += 1,
                }
                Step::Advanced
            }

            State::Code => {
                if let Some(style) = self.comment_at(self.pos) {
                    let opener = match &self.table.comments()[style] {
                        CommentStyle::Line { opener } => opener,
                        CommentStyle::Block { opener, .. } => opener,
                    };
                    let start = self.pos;
                    self.pos += opener.chars().count();
                    self.state = State::Comment {
                        style,
                        depth: 0,
                        start,
                    };
                    // A prefix with only a comment after it prefixes nothing.
                    self.prefix = None;
                    return Step::Advanced;
                }

                // A string whose delimiters are more than one character,
                // before the class dispatch -- and before the character
                // literal, since an opener may begin with a quote.
                //
                // It has to come first because its opener *contains* things
                // that have classes of their own: the quote in a raw string's
                // opener is a string quote, and dispatching on it would open
                // an ordinary string that ends at the first quote inside. That
                // is precisely how the init.lisp in this editor's own source
                // -- a raw string holding Lisp with quotes in it -- made the
                // rest of the file scan as though half of it were code.
                if let Some(style) = self.string_at(self.pos) {
                    let start = self.opening(self.pos);
                    self.pos += self.table.strings()[style].opener.chars().count();
                    self.state = State::StyledStr { style, start };
                    return Step::Advanced;
                }

                // A character literal, before the class dispatch, because the
                // character that opens one has no single class: `'` quotes a
                // character in `'a'`, begins a lifetime in `'static`, and is
                // an apostrophe in prose. Which it is depends on what follows,
                // so it is decided by looking rather than by looking up.
                //
                // The whole literal is one atom. Without this, the `"` in
                // `let c = '"';` opens a string that never closes, and every
                // bracket in the rest of the file stops counting -- which is
                // how a brace three lines down became invisible to anything
                // asking whether the buffer balanced.
                if let Some(end) = self
                    .table
                    .char_literal_at(self.pos, len, |at| self.text.at(at))
                {
                    let start = self.opening(self.pos);
                    self.pos = end;
                    return self.complete(start, end);
                }

                match self.table.class_of(c) {
                    SyntaxClass::Open(_) => {
                        let start = self.opening(self.pos);
                        self.stack.push(Frame {
                            start,
                            delimiter: self.pos,
                            children: VecDeque::new(),
                        });
                        self.pos += 1;
                        Step::Advanced
                    }
                    SyntaxClass::Close(_) => {
                        // A closer that does not match its opener still closes
                        // it, and a closer with nothing open is stray text.
                        // Every file is unbalanced while it is being typed, and
                        // refusing to scan past that would make the motion
                        // useless exactly when it is being used.
                        self.pos += 1;
                        self.prefix = None;
                        if self.stack.len() > 1 {
                            let frame = self.stack.pop().expect("checked non-empty");
                            return self.complete(frame.start, self.pos);
                        }
                        Step::Advanced
                    }
                    SyntaxClass::StringQuote => {
                        let start = self.opening(self.pos);
                        self.state = State::Str { quote: c, start };
                        self.pos += 1;
                        Step::Advanced
                    }
                    SyntaxClass::Escape => {
                        self.pos += 2;
                        Step::Advanced
                    }
                    SyntaxClass::Prefix => {
                        // Only the first of a run is remembered: `#'foo` is one
                        // expression beginning at the `#`.
                        if self.prefix.is_none() {
                            self.prefix = Some(self.pos);
                        }
                        self.pos += 1;
                        Step::Advanced
                    }
                    SyntaxClass::Symbol => {
                        let start = self.opening(self.pos);
                        let mut end = self.pos;
                        while end < len
                            && self
                                .text
                                .at(end)
                                .map(|c| matches!(self.table.class_of(c), SyntaxClass::Symbol))
                                == Some(true)
                        {
                            end += 1;
                        }
                        // Never stand still. This branch is reached exactly
                        // when the character is a symbol constituent, so `end`
                        // is already past it  but that is an agreement between
                        // two pieces of code, and a loop whose termination
                        // depends on one is a loop that hangs the editor the
                        // day they disagree.
                        self.pos = end.max(self.pos + 1);
                        self.complete(start, end)
                    }
                    SyntaxClass::Punctuation => {
                        // Whitespace after a prefix detaches it: `' foo` quotes
                        // the next thing, but the quote is not part of `foo`'s
                        // text and a kill of `foo` should not take it.
                        if c.is_whitespace() {
                            self.prefix = None;
                        }
                        self.pos += 1;
                        Step::Advanced
                    }
                }
            }
        }
    }

    /// A comment is an expression too, so `forward-sexp` steps over one rather
    /// than stopping dead in front of it.
    fn complete_comment(&mut self, start: usize, end: usize) -> Step {
        self.complete(start, end)
    }

    /// Run until the scan reaches `limit`.
    fn run_to(&mut self, limit: usize) {
        while self.pos < limit {
            if matches!(self.step(), Step::End) {
                break;
            }
        }
    }

    pub fn context(&self) -> Context {
        Context {
            depth: self.depth(),
            innermost: if self.stack.len() > 1 {
                self.stack.last().map(|frame| frame.start)
            } else {
                None
            },
            innermost_delimiter: if self.stack.len() > 1 {
                self.stack.last().map(|frame| frame.delimiter)
            } else {
                None
            },
            string_start: match self.state {
                // Both kinds count: `syntax-ppss` is asked "am I in a string",
                // and a raw one is a string.
                State::Str { start, .. } | State::StyledStr { start, .. } => Some(start),
                _ => None,
            },
            comment_start: match self.state {
                State::Comment { start, .. } => Some(start),
                _ => None,
            },
        }
    }
}

/// Where `forward-sexp` lands. `from` unchanged if there are not that many
/// expressions left at this level  at the end of a list the command does
/// nothing rather than escaping outwards.
pub fn forward<B: BufferTrait>(text: &B, table: &SyntaxTable, from: usize, count: usize) -> usize {
    if count == 0 {
        return from;
    }
    let mut scan = Scan::begin(text, table, from, 1);
    scan.run_to(from);
    let start_depth = scan.depth();

    let mut remaining = count;
    loop {
        match scan.step() {
            Step::End => return from,
            Step::Completed(depth, sexp) if depth == start_depth && sexp.end > from => {
                remaining -= 1;
                if remaining == 0 {
                    return sexp.end;
                }
            }
            _ => {}
        }
    }
}

/// Where `backward-sexp` lands: the first character of the Nth previous
/// expression, its prefix included.
pub fn backward<B: BufferTrait>(text: &B, table: &SyntaxTable, from: usize, count: usize) -> usize {
    if count == 0 {
        return from;
    }
    // Scanning only up to `from` is what makes this work: the innermost frame
    // left open there is the list point is in, and its remembered children are
    // exactly the siblings behind point, most recent last.
    let mut scan = Scan::begin(text, table, from, count);
    scan.run_to(from);
    let frame = scan.stack.last().expect("the root frame is never popped");
    if frame.children.len() < count {
        return from;
    }
    frame.children[frame.children.len() - count].start
}

/// Where `down-list` lands: just inside the next list that opens after `from`.
///
/// `None` when there is no list left to enter. Unlike the motions above this is
/// not looking for a *completed* expression but for an opener, which the scan
/// passes over silently  so it watches the depth: the moment the scan is one
/// level deeper than it began, it has just stepped through one.
pub fn down<B: BufferTrait>(text: &B, table: &SyntaxTable, from: usize) -> Option<usize> {
    let mut scan = Scan::begin(text, table, from, 1);
    scan.run_to(from);
    let start_depth = scan.depth();
    loop {
        match scan.step() {
            Step::End => return None,
            _ if scan.depth() > start_depth => return Some(scan.pos),
            _ => {}
        }
    }
}

pub fn context_at<B: BufferTrait>(text: &B, table: &SyntaxTable, pos: usize) -> Context {
    let mut scan = Scan::begin(text, table, pos, 1);
    scan.run_to(pos);
    scan.context()
}

/// The innermost list point is inside, whole.
pub fn enclosing<B: BufferTrait>(text: &B, table: &SyntaxTable, pos: usize) -> Option<Sexp> {
    let mut scan = Scan::begin(text, table, pos, 1);
    scan.run_to(pos);
    let wanted = scan.depth();
    if wanted == 0 {
        return None;
    }
    let start = scan.stack.last()?.start;
    loop {
        match scan.step() {
            Step::End => {
                return Some(Sexp {
                    start,
                    end: text.len(),
                });
            }
            Step::Completed(depth, sexp) if depth + 1 == wanted && sexp.start == start => {
                return Some(sexp);
            }
            _ => {}
        }
    }
}
