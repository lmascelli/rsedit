//! Exclusion patterns in the syntax of a .gitignore file: reading the lines
//! of one, and saying whether a path relative to the directory the lines
//! belong to is left out, kept again by a `!` line, or not named at all.
//!
//! Design: `doc/architettura-editor.typ, <indice-file>`.

/// One line of patterns, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pattern {
    segments: Vec<Segment>,
    /// A pattern with no `/` but a trailing one: it names the last part of a
    /// path at any depth, rather than a path from the patterns' directory.
    basename: bool,
    /// A trailing `/`: only a directory matches.
    directory_only: bool,
    /// A leading `!`: what matches is kept, not left out.
    negated: bool,
}

/// One part of a pattern, between two `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// `**` alone: any number of parts, none included, or one at least at
    /// the end of the pattern.
    AnyDepth,
    /// No wildcard: the part is this text.
    Literal(String),
    /// `*` then no wildcard: the part ends with this text.
    Suffix(String),
    Glob(Vec<Token>),
}

/// One character of a part that has wildcards.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Char(char),
    /// `?`.
    Any,
    /// `*`, or several in a row.
    Star,
    /// `[...]`, as inclusive ranges, a single character being a range of one.
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

/// What the patterns say of a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Excluded,
    /// Named by a `!` line after any line that left it out.
    Included,
}

impl Token {
    fn accepts(&self, c: char) -> bool {
        match self {
            Token::Char(x) => *x == c,
            Token::Any => true,
            Token::Star => false,
            Token::Class { negated, ranges } => {
                *negated != ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c))
            }
        }
    }
}

/// Returns the patterns of TEXT, the contents of an ignore file, in order:
/// blank lines and `#` comments are not patterns, and neither is a line this
/// does not understand, which is skipped rather than guessed at.
pub(crate) fn parse(text: &str) -> Vec<Pattern> {
    text.lines().filter_map(parse_line).collect()
}

/// Returns the pattern on LINE, or `None` for a blank line, a comment, or
/// one this does not understand.
pub(crate) fn parse_line(line: &str) -> Option<Pattern> {
    let line = trim_end(line);
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (negated, line) = match line.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    // `\#` and `\!` are the escapes of a name that begins with one of them.
    let line = line
        .strip_prefix('\\')
        .filter(|rest| rest.starts_with(['#', '!']))
        .unwrap_or(line);
    let (directory_only, line) = match line.strip_suffix('/') {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    let anchored = line.starts_with('/');
    let line = line.trim_start_matches('/');
    if line.is_empty() {
        return None;
    }
    let basename = !anchored && !line.contains('/');
    let segments = line
        .split('/')
        .filter(|part| !part.is_empty())
        .map(segment)
        .collect::<Option<Vec<_>>>()?;
    Some(Pattern {
        segments,
        basename,
        directory_only,
        negated,
    })
}

/// LINE without its line ending and the spaces after the pattern, but for a
/// space written `\ `, which is part of it.
fn trim_end(line: &str) -> &str {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut end = line.len();
    while end > 0 && line[..end].ends_with([' ', '\t']) {
        let before = &line[..end - 1];
        if before.ends_with('\\') && !before.ends_with("\\\\") {
            break;
        }
        end -= 1;
    }
    &line[..end]
}

/// Reads one part of a pattern, or `None` when it has something this does
/// not understand: a `[:class:]`.
fn segment(text: &str) -> Option<Segment> {
    if text == "**" {
        return Some(Segment::AnyDepth);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                tokens.push(Token::Char(chars[i + 1]));
                i += 2;
            }
            '*' => {
                if tokens.last() != Some(&Token::Star) {
                    tokens.push(Token::Star);
                }
                i += 1;
            }
            '?' => {
                tokens.push(Token::Any);
                i += 1;
            }
            '[' => match class(&chars[i + 1..])? {
                Some((token, used)) => {
                    tokens.push(token);
                    i += 1 + used;
                }
                // A `[` that is never closed is the character itself.
                None => {
                    tokens.push(Token::Char('['));
                    i += 1;
                }
            },
            c => {
                tokens.push(Token::Char(c));
                i += 1;
            }
        }
    }
    let literal = |tokens: &[Token]| -> Option<String> {
        tokens
            .iter()
            .map(|token| match token {
                Token::Char(c) => Some(*c),
                _ => None,
            })
            .collect()
    };
    Some(if let Some(text) = literal(&tokens) {
        Segment::Literal(text)
    } else if tokens.first() == Some(&Token::Star)
        && let Some(text) = literal(&tokens[1..])
    {
        Segment::Suffix(text)
    } else {
        Segment::Glob(tokens)
    })
}

/// Reads a class from CHARS, which follow its `[`: the class and how many
/// characters it took, its `]` included; `Some(None)` when it is not closed,
/// and `None` when it holds a `[:name:]`.
fn class(chars: &[char]) -> Option<Option<(Token, usize)>> {
    let mut i = 0;
    let negated = matches!(chars.first(), Some('!' | '^'));
    if negated {
        i += 1;
    }
    let mut ranges = Vec::new();
    let mut first = true;
    while i < chars.len() {
        let mut c = chars[i];
        if c == ']' && !first {
            return Some(Some((Token::Class { negated, ranges }, i + 1)));
        }
        if c == '[' && chars.get(i + 1) == Some(&':') {
            return None;
        }
        if c == '\\' && i + 1 < chars.len() {
            i += 1;
            c = chars[i];
        }
        first = false;
        if chars.get(i + 1) == Some(&'-') && chars.get(i + 2).is_some_and(|hi| *hi != ']') {
            let mut hi = chars[i + 2];
            let mut used = 3;
            if hi == '\\' && i + 3 < chars.len() {
                hi = chars[i + 3];
                used = 4;
            }
            ranges.push((c, hi));
            i += used;
        } else {
            ranges.push((c, c));
            i += 1;
        }
    }
    Some(None)
}

/// Whether TOKENS match the whole of TEXT, a part of a path.
fn glob(tokens: &[Token], text: &str) -> bool {
    let (mut t, mut s) = (0, 0);
    // Where the last star was, and how much of TEXT it has taken so far.
    let mut star: Option<(usize, usize)> = None;
    loop {
        if t < tokens.len() {
            if tokens[t] == Token::Star {
                star = Some((t, s));
                t += 1;
                continue;
            }
            if let Some(c) = text[s..].chars().next()
                && tokens[t].accepts(c)
            {
                t += 1;
                s += c.len_utf8();
                continue;
            }
        } else if s == text.len() {
            return true;
        }
        let Some((at, taken)) = star else {
            return false;
        };
        let Some(c) = text[taken..].chars().next() else {
            return false;
        };
        star = Some((at, taken + c.len_utf8()));
        t = at + 1;
        s = taken + c.len_utf8();
    }
}

impl Segment {
    fn matches(&self, part: &str) -> bool {
        match self {
            Segment::AnyDepth => true,
            Segment::Literal(text) => part == text,
            Segment::Suffix(text) => part.ends_with(text.as_str()),
            Segment::Glob(tokens) => glob(tokens, part),
        }
    }
}

/// Whether SEGMENTS match the whole of PARTS.
fn segments_match(segments: &[Segment], parts: &[&str]) -> bool {
    match segments.split_first() {
        None => parts.is_empty(),
        Some((Segment::AnyDepth, [])) => !parts.is_empty(),
        Some((Segment::AnyDepth, rest)) => {
            (0..=parts.len()).any(|skipped| segments_match(rest, &parts[skipped..]))
        }
        Some((segment, rest)) => parts
            .split_first()
            .is_some_and(|(part, others)| segment.matches(part) && segments_match(rest, others)),
    }
}

impl Pattern {
    /// Whether the pattern names PATH, relative to the directory of the
    /// patterns and written with `/`, whose last part is NAME; DIRECTORY says
    /// whether it is one. A `!` is not looked at.
    pub(crate) fn matches(&self, path: &str, name: &str, directory: bool) -> bool {
        if self.directory_only && !directory {
            return false;
        }
        if self.basename {
            return self.segments[0].matches(name);
        }
        let parts: Vec<&str> = path.split('/').collect();
        segments_match(&self.segments, &parts)
    }
}

/// Returns what PATTERNS say of PATH, as [`Pattern::matches`] takes it: the
/// last line that names it decides, and `None` when none does.
pub(crate) fn verdict(
    patterns: &[Pattern],
    path: &str,
    name: &str,
    directory: bool,
) -> Option<Verdict> {
    patterns
        .iter()
        .rev()
        .find(|pattern| pattern.matches(path, name, directory))
        .map(|pattern| {
            if pattern.negated {
                Verdict::Included
            } else {
                Verdict::Excluded
            }
        })
}
