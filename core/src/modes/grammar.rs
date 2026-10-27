//! The grammar a major mode colours with, and `highlight_line`, the lexer
//! that applies it a line at a time, carrying the regions still open from
//! each line to the next.
//!
//! Design: `doc/architettura-editor.typ, <grammatica>`.
use crate::ui::Face;
use regex::Regex;

/// A run of characters to draw with a face, in **character columns** within its
/// line.
///
/// Characters, not bytes: the renderer works in columns and the regexp engine
/// in bytes, so the conversion is made here, once, rather than wherever a span
/// is read.
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
    /// How a rule says what it cannot look ahead for: `\b([a-z_]\w*)\s*\(`
    /// with group 1 colours the name of a call and not its parenthesis. The
    /// scan still steps over the whole match.
    pub group: usize,
    /// The name the rule was added under, if any. A rule added again under
    /// the same name takes this one's place, where it stands among the rest.
    pub name: Option<String>,
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
    /// Returns whether the grammar has neither rules nor regions.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.regions.is_empty()
    }
}

/// Which regions are open, outermost first.
///
/// Empty on almost every line, which is what makes interning them in the cache
/// worth doing: see `crate::buffer::syntax::SyntaxCache`.
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
    /// Returns where the event begins: for a rule, where the text it faces
    /// begins, or where the scan resumes if that is earlier.
    fn start(&self) -> usize {
        match self {
            Event::Open(_, start, _) | Event::Close(start, _) | Event::Skip(start, _) => *start,
            Event::Rule(_, start, _, resume) => (*start).min(*resume),
        }
    }
}

/// Returns the spans GRAMMAR colours LINE with, entering it in the state
/// ENTERING, and the state leaving it.
///
/// The line comes without its newline: what ends at the end of a line is a
/// rule (`//.*`), not a region whose `end` matches a newline this never sees.
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

    // TODO(SIN-1): only a rule is made to move on; an empty begin, end or
    // escape leaves `pos` where it was, and the loop does not end.
    while pos <= line.len() {
        match state.last().copied() {
            Some(open) => {
                // A state carried over from another grammar can name a region
                // this one does not have. The cache's stamps keep that from
                // happening, and this keeps it from being a panic on the
                // executor every background job shares.
                let Some(region) = grammar.regions.get(open) else {
                    state.pop();
                    continue;
                };
                match inside_event(region, line, pos, open) {
                    Some(Event::Skip(_, resume)) => pos = resume,
                    Some(Event::Open(id, _, resume)) => {
                        // Nested, and faced as the region around it is, so
                        // the run goes on through it.
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

/// Adds to SPANS the span of LINE from START to END, byte offsets, in
/// character columns. An empty span is dropped: it colours nothing.
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

/// Returns the next thing that matters inside REGION, from POS: its escape,
/// its end or, when it nests, another of itself, whichever comes first.
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
    // The escape is offered first, so that it wins a tie against the end: `\"`
    // is an escape here and, one byte along, an end, which is what the escape
    // is there to prevent.
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

// TODO(SIN-2): every pattern is asked again from `pos` at every event, so a
// line costs its length times its events.
/// Returns the next thing that matters outside any region, from POS: the
/// earliest region opening or rule match. Regions are offered first, so that
/// a region wins a tie, the same way every time.
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
        // A rule whose group took no part in the match, or that names a group
        // the pattern does not have, colours the whole match. Either way the
        // scan resumes after the whole match.
        // TODO(SIN-7): with GROUP, only that group is to take the face.
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
