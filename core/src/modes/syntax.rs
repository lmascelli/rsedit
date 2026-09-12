//! The grammar a major mode highlights with, and the one-line lexer that
//! applies it.
//!
//! # Why a line at a time, with a state carried between them
//!
//! A comment that opens on line 10 and closes on line 40 colours everything
//! between, so highlighting cannot look at a line in isolation. The two ways
//! out are to run patterns over the *whole buffer* -- correct, and impossible
//! to make incremental, since a match may begin arbitrarily far back -- or to
//! lex one line at a time carrying a **state** from each line to the next.
//!
//! This is the second. [`highlight_line`] takes the state entering a line and
//! gives back the state leaving it, so the only thing needed to colour line
//! 5000 is the state at line 5000, which a cache can hold. That is what makes
//! re-lexing after an edit proportional to the edit rather than to the file.
//!
//! # Leftmost wins
//!
//! The scan is a single left-to-right pass in which **every active pattern is
//! tried at once and the earliest match wins**. Not "regions first, then
//! rules", which is the obvious implementation and is wrong:
//!
//! ```text
//!     // this /* is not a block comment
//! ```
//!
//! Scanning regions first opens a block comment that swallows the rest of the
//! file. Leftmost-wins matches the line comment at column 4, consumes to the
//! end of the line, and never looks at the `/*`.
//!
//! # What the regular expressions cannot do
//!
//! `regex` has no lookahead, no lookbehind and no backreferences -- that is how
//! it guarantees linear time. Two things in this module exist only because of
//! that:
//!
//! - [`SyntaxRule::group`], because `\b\w+(?=\()` is unavailable. A rule
//!   matches the name *and* the parenthesis, and faces only the group.
//! - [`SyntaxRegion::escape`], because `(?<!\\)"` is unavailable. A string
//!   region names the escape explicitly and the scan steps over it.
use crate::ui::Face;
use regex::Regex;

/// A run of characters to draw with a face, in **character columns** within its
/// line.
///
/// Characters, not bytes: the renderer works in columns and the regexp engine
/// works in bytes, so the conversion happens here, once, rather than at every
/// place a span is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxSpan {
    pub start: usize,
    pub end: usize,
    pub face: Face,
}

/// A pattern that colours what it matches, within one line.
#[derive(Clone, Debug)]
pub struct SyntaxRule {
    pub pattern: Regex,
    pub face: Face,
    /// Which capture group takes the face; 0 is the whole match.
    ///
    /// This is how a rule expresses context it cannot look ahead for: match
    /// `\b([a-z_]\w*)\s*\(` and face group 1, and a function *call* is coloured
    /// without the parenthesis being coloured with it. The scan still steps
    /// over the whole match.
    pub group: usize,
}

/// A construct with a beginning and an end, which may be on different lines.
#[derive(Clone, Debug)]
pub struct SyntaxRegion {
    pub begin: Regex,
    pub end: Regex,
    /// Text that looks like the end but is not -- `\\.` inside a string, so
    /// that `"a \" b"` does not stop at the escaped quote.
    pub escape: Option<Regex>,
    pub face: Face,
    /// Whether `begin` matching inside the region opens another one, so that
    /// `/* /* */ */` closes where it should rather than at the first `*/`.
    pub nestable: bool,
}

/// Everything a major mode knows about colouring its language.
#[derive(Clone, Debug, Default)]
pub struct Grammar {
    pub rules: Vec<SyntaxRule>,
    pub regions: Vec<SyntaxRegion>,
}

impl Grammar {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.regions.is_empty()
    }
}

/// Which regions are open, outermost first.
///
/// Empty on almost every line, which is what makes interning these in the cache
/// worth doing -- see `crate::buffer::syntax::SyntaxCache`.
pub type SyntaxState = Vec<usize>;

/// What the scan found next, and where.
enum Event {
    /// A region opened. Carries its index in `Grammar::regions`.
    Open(usize, usize, usize),
    /// The innermost region closed.
    Close(usize, usize),
    /// Something that looks like a close but is escaped; step over it.
    Skip(usize, usize),
    /// A rule matched. Carries the byte range to *face* and the byte to
    /// resume at, which differ when the rule faces a capture group.
    Rule(Face, usize, usize, usize),
}

impl Event {
    fn start(&self) -> usize {
        match self {
            Event::Open(_, start, _) | Event::Close(start, _) | Event::Skip(start, _) => *start,
            Event::Rule(_, start, _, resume) => (*start).min(*resume),
        }
    }
}

/// Colour one line, given the state entering it, and report the state leaving
/// it.
///
/// The line should not include its newline: a region that ends at end-of-line
/// is expressed by a rule (`//.*`), not by a region whose `end` matches a
/// newline this never sees.
pub fn highlight_line(
    grammar: &Grammar,
    line: &str,
    entering: &SyntaxState,
) -> (Vec<SyntaxSpan>, SyntaxState) {
    let mut state = entering.clone();
    let mut spans = Vec::new();
    let mut pos = 0;
    // Where the current region-faced run began, once one is open. A region that
    // was already open on entry starts its run at the beginning of the line.
    let mut run_start = 0;

    while pos <= line.len() {
        match state.last().copied() {
            Some(open) => {
                let region = &grammar.regions[open];
                match inside_event(region, line, pos, open) {
                    Some(Event::Skip(_, resume)) => pos = resume,
                    Some(Event::Open(id, _, resume)) => {
                        // Nested, and uniformly faced in this version, so the
                        // run simply continues through it.
                        state.push(id);
                        pos = resume;
                    }
                    Some(Event::Close(_, resume)) => {
                        push_span(&mut spans, line, run_start, resume, region.face);
                        state.pop();
                        pos = resume;
                        run_start = resume;
                    }
                    _ => {
                        // Nothing closes it on this line: the rest belongs to
                        // the region, and it stays open for the next line.
                        push_span(&mut spans, line, run_start, line.len(), region.face);
                        break;
                    }
                }
            }
            None => match top_level_event(grammar, line, pos) {
                Some(Event::Open(id, start, resume)) => {
                    state.push(id);
                    run_start = start;
                    pos = resume;
                }
                Some(Event::Rule(face, start, end, resume)) => {
                    push_span(&mut spans, line, start, end, face);
                    // Never stand still: a rule that can match the empty string
                    // would otherwise be found at the same position forever.
                    pos = resume.max(pos + 1);
                }
                _ => break,
            },
        }
    }

    (spans, state)
}

/// Add a span, converting byte offsets to character columns.
///
/// Empty spans are dropped rather than stored: they colour nothing, and a
/// renderer would have to filter them out anyway.
fn push_span(spans: &mut Vec<SyntaxSpan>, line: &str, start: usize, end: usize, face: Face) {
    if start >= end {
        return;
    }
    spans.push(SyntaxSpan {
        start: line[..start].chars().count(),
        end: line[..end.min(line.len())].chars().count(),
        face,
    });
}

/// The next thing that matters while inside REGION: its escape, its end, or --
/// when it nests -- another of itself, whichever comes first.
fn inside_event(region: &SyntaxRegion, line: &str, pos: usize, open: usize) -> Option<Event> {
    if pos > line.len() {
        return None;
    }
    let mut best: Option<Event> = None;
    let mut offer = |event: Event| {
        if best.as_ref().is_none_or(|b| event.start() < b.start()) {
            best = Some(event);
        }
    };
    // The escape is offered first so that it wins a tie against the end: `\"`
    // is *both* an escape at this position and, one byte along, an end -- and
    // treating it as the end is exactly the bug the escape exists to prevent.
    if let Some(escape) = &region.escape
        && let Some(m) = escape.find_at(line, pos)
    {
        offer(Event::Skip(m.start(), m.end()));
    }
    if let Some(m) = region.end.find_at(line, pos) {
        offer(Event::Close(m.start(), m.end()));
    }
    if region.nestable
        && let Some(m) = region.begin.find_at(line, pos)
    {
        offer(Event::Open(open, m.start(), m.end()));
    }
    best
}

/// The next thing that matters outside any region: the earliest region opening
/// or rule match.
///
/// Regions are offered before rules, so that a tie at the same position opens
/// the region. Ties are rare -- two patterns matching at the same column
/// usually means the grammar is ambiguous -- but the answer has to be the same
/// every time.
fn top_level_event(grammar: &Grammar, line: &str, pos: usize) -> Option<Event> {
    if pos > line.len() {
        return None;
    }
    let mut best: Option<Event> = None;
    let mut offer = |event: Event| {
        if best.as_ref().is_none_or(|b| event.start() < b.start()) {
            best = Some(event);
        }
    };
    for (id, region) in grammar.regions.iter().enumerate() {
        if let Some(m) = region.begin.find_at(line, pos) {
            offer(Event::Open(id, m.start(), m.end()));
        }
    }
    for rule in &grammar.rules {
        let Some(captures) = rule.pattern.captures_at(line, pos) else {
            continue;
        };
        let whole = captures.get(0).expect("a capture set has a group zero");
        // A rule whose group did not participate colours nothing, but the scan
        // still steps over what it matched -- otherwise the same match would be
        // found again at the same position.
        let faced = captures.get(rule.group).unwrap_or(whole);
        offer(Event::Rule(
            rule.face,
            faced.start(),
            faced.end(),
            whole.end(),
        ));
    }
    best
}
