//! The scanner of balanced expressions: a forward lex, driven by a mode's
//! syntax table, that counts a delimiter only while it reads it as code, and
//! answers where an expression ends or begins and what a position is inside.
//! A scan starts at the top of the buffer, or at a [`Resume`] taken earlier.
//!
//! Design: `doc/architettura-editor.typ, <scanner>`.
use crate::BufferTrait;
use crate::modes::{CommentStyle, SyntaxClass, SyntaxTable};
use std::collections::VecDeque;

/// One expression: where it begins, prefix included, and where it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sexp {
    pub start: usize,
    pub end: usize,
}

/// What a position is inside, as `syntax-ppss` reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// How many lists are open.
    pub depth: usize,
    /// Where the innermost still-open list begins.
    pub innermost: Option<usize>,
    /// Where that list's delimiter is, as opposed to where its expression
    /// begins: `'(a b)` begins at the quote. Motion wants the first, so that a
    /// kill takes the quote with it, and indentation the second.
    pub innermost_delimiter: Option<usize>,
    /// Where the string the position is in begins, if it is in one.
    pub string_start: Option<usize>,
    /// Where the comment the position is in begins, if it is in one.
    pub comment_start: Option<usize>,
}

/// What the scan is reading: code, a string, a comment, or a string whose
/// delimiters are more than one character.
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
    /// string, a triple-quoted one. Only the closer ends it, and a backslash
    /// escapes only in a style that says so.
    StyledStr {
        style: usize,
        start: usize,
    },
}

/// One level of nesting, and the last few expressions completed inside it.
#[derive(Clone, Debug)]
struct Frame {
    /// Where the expression opening this level begins: at its prefix, if it
    /// has one, rather than at the delimiter, as `'(a b)` opens at the quote.
    start: usize,
    /// Where the delimiter itself is. See `Context::innermost_delimiter`.
    delimiter: usize,
    /// The last `keep` of them, which is what `backward-sexp` with a count
    /// of N needs, rather than a whole file's at every keystroke.
    children: VecDeque<Sexp>,
}

/// Everything a scan needs to carry on from an offset it did not start at:
/// the whole state, not a depth, without the children of the frames, which
/// depend on the `keep` of the scan that took it. So [`backward`], which
/// reads them, never resumes.
#[derive(Clone, Debug)]
pub struct Resume {
    pos: usize,
    state: State,
    stack: Vec<Frame>,
    prefix: Option<usize>,
}

impl Resume {
    /// Returns where a scan carrying this on begins: not always the start of
    /// a line, since a step may consume several characters, but the first
    /// position at or after it that the scan stopped on.
    pub fn offset(&self) -> usize {
        self.pos
    }
}

/// What one step of the scan did.
enum Step {
    /// An expression finished, at this nesting depth.
    Completed(usize, Sexp),
    /// Something was consumed; nothing finished.
    Advanced,
    /// The text ran out.
    End,
}

/// A forward scan of a text with a syntax table: what it is reading, the
/// lists open, and the last expressions completed in each.
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
    /// Returns a scan of TEXT with TABLE, to be asked about BEFORE, keeping
    /// the last KEEP expressions of each list: carried on from RESUME when it
    /// is at or before BEFORE, and from the top otherwise. The one place a
    /// scan's start is chosen, so a snapshot that does not fit costs a full
    /// scan rather than a wrong answer.
    pub fn begin(
        text: &'a B,
        table: &'a SyntaxTable,
        before: usize,
        resume: Option<&Resume>,
        keep: usize,
    ) -> Self {
        if let Some(from) = resume.filter(|from| from.pos <= before) {
            return Self {
                text,
                table,
                pos: from.pos,
                state: from.state.clone(),
                stack: from.stack.clone(),
                prefix: from.prefix,
                keep: keep.max(1),
            };
        }
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

    /// Returns how many lists are open.
    fn depth(&self) -> usize {
        self.stack.len() - 1
    }

    /// Records the expression from START to END in the innermost list, which
    /// keeps the last `keep`, and reports it completed.
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

    /// Returns where the expression starting at AT begins, taking the prefix
    /// waiting in front of it, if there is one.
    fn opening(&mut self, at: usize) -> usize {
        self.prefix.take().unwrap_or(at)
    }

    /// Returns whether LITERAL is in the text at AT.
    fn literal_at(&self, at: usize, literal: &str) -> bool {
        literal
            .chars()
            .enumerate()
            .all(|(i, c)| self.text.at(at + i) == Some(c))
    }

    /// Returns the string style that opens at AT, if any: the one with the
    /// longest opener, as in `comment_at`, so that a raw string's opener wins
    /// over the plain quote it begins with.
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

    /// Returns the comment style that opens at AT, if any.
    fn comment_at(&self, at: usize) -> Option<usize> {
        let c = self.text.at(at)?;
        if !self.table.may_start_comment(c) {
            return None;
        }
        // The longest opener wins, so that `//` is not read as a shorter one
        // that shares its first character.
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

    /// Takes one step: reads what is at the scan's position in the state it
    /// is in, and says what that completed.
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
                                // Passed over, as a line comment is and as
                                // whitespace is: not an expression.
                                self.state = State::Code;
                                return Step::Advanced;
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
                if self.table.strings()[style].escaped
                    && matches!(self.table.class_of(c), SyntaxClass::Escape)
                {
                    self.pos += 2;
                    return Step::Advanced;
                }

                let closer = &self.table.strings()[style].closer;
                if self.literal_at(self.pos, closer) {
                    self.pos += closer.chars().count();
                    self.state = State::Code;
                    return self.complete(start, self.pos);
                }
                self.pos += 1;
                Step::Advanced
            }

            State::Str { quote, start } => {
                match self.table.class_of(c) {
                    // Two characters at once, so that `"a \" b"` does not end
                    // at the middle quote. Escapes count inside strings only:
                    // a `\` at the end of a `//` comment does not continue it.
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

                // A string whose delimiters are more than one character comes
                // before the class dispatch, and before the character literal,
                // since an opener may begin with a quote: the quote in a raw
                // string's opener is a string quote, and dispatching on it
                // would open a plain string, ending at the first quote inside.
                if let Some(style) = self.string_at(self.pos) {
                    let start = self.opening(self.pos);
                    self.pos += self.table.strings()[style].opener.chars().count();
                    self.state = State::StyledStr { style, start };
                    return Step::Advanced;
                }

                // A character literal comes before the class dispatch, since
                // the character that opens one has no single class: `'` quotes
                // a character in `'a'`, begins a lifetime in `'static` and is
                // an apostrophe in prose. The whole literal is one atom, so
                // the `"` in `let c = '"';` opens no string.
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
                        // it, and a closer with nothing open is stray text: a
                        // file is unbalanced all the while it is being typed.
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
                        // Never stand still. The branch is reached when the
                        // character is a symbol constituent, so `end` is past
                        // it already; the loop does not rest on that alone.
                        self.pos = end.max(self.pos + 1);
                        self.complete(start, end)
                    }
                    SyntaxClass::Punctuation => {
                        // Whitespace after a prefix detaches it: in `' foo` the
                        // quote is not part of `foo`, and a kill of `foo` does
                        // not take it.
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

    /// Returns the scan's state here, for carrying on from later, with the
    /// frames copied without their children: see [`Resume`].
    pub fn snapshot(&self) -> Resume {
        Resume {
            pos: self.pos,
            state: self.state.clone(),
            stack: self
                .stack
                .iter()
                .map(|frame| Frame {
                    start: frame.start,
                    delimiter: frame.delimiter,
                    children: VecDeque::new(),
                })
                .collect(),
            prefix: self.prefix,
        }
    }

    /// Returns where the scan has reached.
    pub fn offset(&self) -> usize {
        self.pos
    }

    /// Runs the scan until it reaches LIMIT or the end of the text, and
    /// returns the expression its last step completed across LIMIT, with the
    /// depth it was completed at, if it did.
    ///
    /// A step consumes a whole symbol, so the scan may stop past LIMIT: from
    /// inside `foo`, the step that reads it completes it, and an expression
    /// that LIMIT is inside is the first that ends after it.
    pub(crate) fn run_to(&mut self, limit: usize) -> Option<(usize, Sexp)> {
        while self.pos < limit {
            match self.step() {
                Step::End => break,
                Step::Completed(depth, sexp) if sexp.end > limit => return Some((depth, sexp)),
                _ => {}
            }
        }
        None
    }

    /// Returns what the scan's position is inside.
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
                // Both kinds: `syntax-ppss` is asked whether the position is
                // in a string, and a raw one is a string.
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

/// Returns where `forward-sexp` lands from FROM with COUNT: the end of the
/// COUNTth expression that ends after FROM at FROM's depth, or FROM when the
/// list FROM is in, or the text, ends first.
///
/// From inside a symbol the symbol is the first: the step that brings the
/// scan to FROM completes it, and `run_to` hands it back. The list ending
/// stops the count, so that the end of one list is not a way into the next:
/// as in Emacs, where it is an error, nothing moves.
pub fn forward<B: BufferTrait>(
    text: &B,
    table: &SyntaxTable,
    resume: Option<&Resume>,
    from: usize,
    count: usize,
) -> usize {
    if count == 0 {
        return from;
    }
    let mut scan = Scan::begin(text, table, from, resume, 1);
    let across = scan.run_to(from);
    let start_depth = scan.depth();

    let mut remaining = count;
    if let Some((depth, sexp)) = across
        && depth == start_depth
    {
        remaining -= 1;
        if remaining == 0 {
            return sexp.end;
        }
    }
    loop {
        let step = scan.step();
        if scan.depth() < start_depth {
            return from;
        }
        match step {
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

/// Returns where `backward-sexp` lands from FROM with COUNT: the first
/// character of the COUNTth expression before it, prefix included, or FROM
/// when the level has fewer.
pub fn backward<B: BufferTrait>(text: &B, table: &SyntaxTable, from: usize, count: usize) -> usize {
    if count == 0 {
        return from;
    }
    // Scanned up to `from` only: the innermost frame left open there is the
    // list `from` is in, and its children are the expressions behind it, the
    // most recent last. No resume, since a snapshot has no children: see
    // `Resume`.
    let mut scan = Scan::begin(text, table, from, None, count);
    scan.run_to(from);
    let frame = scan.stack.last().expect("the root frame is never popped");
    if frame.children.len() < count {
        return from;
    }
    frame.children[frame.children.len() - count].start
}

/// Returns where `down-list` lands: just inside the first list the scan
/// enters deeper than FROM's depth, or `None` when there is none before the
/// list FROM is in ends. An opener completes nothing, so the scan watches the
/// depth, and stops where it falls below FROM's: past there, a list deeper
/// than FROM's is one nested in a list that follows.
pub fn down<B: BufferTrait>(
    text: &B,
    table: &SyntaxTable,
    resume: Option<&Resume>,
    from: usize,
) -> Option<usize> {
    let mut scan = Scan::begin(text, table, from, resume, 1);
    scan.run_to(from);
    let start_depth = scan.depth();
    loop {
        match scan.step() {
            Step::End => return None,
            _ if scan.depth() > start_depth => return Some(scan.pos),
            _ if scan.depth() < start_depth => return None,
            _ => {}
        }
    }
}

/// Returns what POS is inside, as `syntax-ppss` reports it.
pub fn context_at<B: BufferTrait>(
    text: &B,
    table: &SyntaxTable,
    resume: Option<&Resume>,
    pos: usize,
) -> Context {
    let mut scan = Scan::begin(text, table, pos, resume, 1);
    scan.run_to(pos);
    scan.context()
}

/// Returns the first offset after POS at which every list open at POS has
/// been closed, or `None` when the text runs out with one still open.
///
/// Not whether the buffer balances: a function that is closed stays closed
/// while the one below it is still being typed, and the scan stops at the
/// first point of balance rather than at the end of the buffer.
pub fn balance_point<B: BufferTrait>(
    text: &B,
    table: &SyntaxTable,
    resume: Option<&Resume>,
    pos: usize,
) -> Option<usize> {
    let mut scan = Scan::begin(text, table, pos, resume, 1);
    scan.run_to(pos);
    loop {
        if scan.depth() == 0 {
            return Some(scan.offset());
        }
        if matches!(scan.step(), Step::End) {
            return None;
        }
    }
}

/// Returns the innermost list POS is inside, whole, up to the end of the
/// text when it is not closed, or `None` at the top level.
pub fn enclosing<B: BufferTrait>(
    text: &B,
    table: &SyntaxTable,
    resume: Option<&Resume>,
    pos: usize,
) -> Option<Sexp> {
    let mut scan = Scan::begin(text, table, pos, resume, 1);
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

/// A delimiter, and the one it pairs with: what `show-paren-mode` draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    /// The delimiter next to point.
    pub at: usize,
    /// The delimiter that closes or opens it, or `None` when the text has
    /// none: an opener never closed, a closer with nothing open.
    pub partner: Option<usize>,
    /// Whether the two are of one kind, `(` with `)`; false when there is no
    /// partner.
    pub matched: bool,
}

/// Returns the delimiter next to POINT, with its partner: the opener just
/// after POINT, or else the closer just before it, read as code rather than
/// as text in a string or a comment. `None` when there is neither, and when
/// the scan would read more than REACH characters on one side of POINT: a
/// question asked at every frame must not cost a whole file.
pub fn pairing<B: BufferTrait>(
    text: &B,
    table: &SyntaxTable,
    resume: Option<&Resume>,
    point: usize,
    reach: usize,
) -> Option<Pair> {
    opener_pairing(text, table, resume, point, reach).or_else(|| {
        let at = point.checked_sub(1)?;
        closer_pairing(text, table, resume, at, reach)
    })
}

/// Returns a scan of TEXT stopped at AT, when AT is where a step of code
/// begins, carried on from RESUME and within REACH characters of it; `None`
/// when AT is inside a string, a comment or a token that began before it.
fn scan_to_code<'a, B: BufferTrait>(
    text: &'a B,
    table: &'a SyntaxTable,
    resume: Option<&Resume>,
    at: usize,
    reach: usize,
) -> Option<Scan<'a, B>> {
    let start = resume
        .filter(|from| from.pos <= at)
        .map_or(0, |from| from.pos);
    if at - start > reach {
        return None;
    }
    let mut scan = Scan::begin(text, table, at, resume, 1);
    scan.run_to(at);
    (scan.pos == at && scan.state == State::Code).then_some(scan)
}

/// Returns the opener at AT and its closer, if AT is an opener in code.
fn opener_pairing<B: BufferTrait>(
    text: &B,
    table: &SyntaxTable,
    resume: Option<&Resume>,
    at: usize,
    reach: usize,
) -> Option<Pair> {
    let SyntaxClass::Open(closer) = table.class_of(text.at(at)?) else {
        return None;
    };
    let mut scan = scan_to_code(text, table, resume, at, reach)?;
    let outside = scan.stack.len();
    // The step decides, not the class alone: `(*` may open a comment.
    scan.step();
    if scan.stack.len() != outside + 1 {
        return None;
    }
    loop {
        if scan.pos - at > reach {
            return None;
        }
        if matches!(scan.step(), Step::End) {
            return Some(Pair {
                at,
                partner: None,
                matched: false,
            });
        }
        if scan.stack.len() == outside {
            // A closer pops a frame a step after reading it, one character.
            let partner = scan.pos - 1;
            return Some(Pair {
                at,
                partner: Some(partner),
                matched: text.at(partner) == Some(closer),
            });
        }
    }
}

/// Returns the closer at AT and its opener, if AT is a closer in code.
fn closer_pairing<B: BufferTrait>(
    text: &B,
    table: &SyntaxTable,
    resume: Option<&Resume>,
    at: usize,
    reach: usize,
) -> Option<Pair> {
    let SyntaxClass::Close(opener) = table.class_of(text.at(at)?) else {
        return None;
    };
    let mut scan = scan_to_code(text, table, resume, at, reach)?;
    let inside = scan.stack.len();
    let partner = (inside > 1)
        .then(|| scan.stack.last().map(|frame| frame.delimiter))
        .flatten();
    // A closer in code always steps as one, unless a comment or a string
    // begins with it.
    scan.step();
    if scan.pos != at + 1 || scan.state != State::Code {
        return None;
    }
    Some(Pair {
        at,
        partner,
        matched: partner.and_then(|partner| text.at(partner)) == Some(opener),
    })
}
