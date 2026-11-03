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

/// A match of one pattern: where it begins and ends and, for a rule, the
/// part of it that takes the face, `None` when the rule's group took no part.
#[derive(Clone, Copy)]
struct Found {
    start: usize,
    end: usize,
    faced: Option<(usize, usize)>,
}

/// What a search for one pattern found, and the position it searched from.
type Slot = Option<(usize, Option<Found>)>;

/// The next match of every pattern of a grammar on one line, as last found.
///
/// The scan only moves forward, so a match found from an earlier position
/// that begins at or after the current one is still the next, and a pattern
/// that found nothing from there finds nothing from here. Each pattern is
/// asked again only once the scan has passed its match: a line costs its
/// length once per pattern, not once per pattern and event.
struct Ahead {
    begins: Vec<Slot>,
    ends: Vec<Slot>,
    escapes: Vec<Slot>,
    rules: Vec<Slot>,
}

impl Ahead {
    fn new(grammar: &Grammar) -> Self {
        Ahead {
            begins: vec![None; grammar.regions.len()],
            ends: vec![None; grammar.regions.len()],
            escapes: vec![None; grammar.regions.len()],
            rules: vec![None; grammar.rules.len()],
        }
    }
}

/// Returns the next match from POS that SLOT remembers, or what SEARCH finds
/// from POS when the slot has none still ahead, which the slot then keeps.
fn ask(slot: &mut Slot, pos: usize, search: impl FnOnce(usize) -> Option<Found>) -> Option<Found> {
    if let Some((from, found)) = *slot
        && from <= pos
        && found.is_none_or(|found| found.start >= pos)
    {
        return found;
    }
    let found = search(pos);
    *slot = Some((pos, found));
    found
}

/// Returns the first match of PATTERN in LINE from POS that is not empty.
///
/// For a begin and an escape: one that matched nothing would open a region,
/// or step over an escape, without moving, and the scan would find it there
/// again forever.
fn find_nonempty(pattern: &Regex, line: &str, mut pos: usize) -> Option<Found> {
    while pos <= line.len() {
        let found = pattern.find_at(line, pos)?;
        if !found.is_empty() {
            return Some(Found {
                start: found.start(),
                end: found.end(),
                faced: None,
            });
        }
        let width = line[found.start()..].chars().next()?.len_utf8();
        pos = found.start() + width;
    }
    None
}

/// Returns the first match of PATTERN in LINE from POS, empty or not.
fn find_any(pattern: &Regex, line: &str, pos: usize) -> Option<Found> {
    pattern.find_at(line, pos).map(|found| Found {
        start: found.start(),
        end: found.end(),
        faced: None,
    })
}

/// Returns the spans GRAMMAR colours LINE with, entering it in the state
/// ENTERING, and the state leaving it.
///
/// The line comes without its newline: what ends at the end of a line is a
/// rule (`//.*`), not a region whose `end` matches a newline this never sees.
///
/// Every turn of the loop moves forward or closes a region: a begin or an
/// escape counts only where it matches something, a rule moves on by a byte
/// at least, and an end that matches nothing closes one of the regions
/// there are. So the loop ends whatever the patterns.
pub fn highlight_line(
    grammar: &Grammar,
    line: &str,
    entering: &SyntaxState,
) -> (Vec<SyntaxSpan>, SyntaxState) {
    let mut state = entering.clone();
    let mut spans = Vec::new();
    let mut ahead = Ahead::new(grammar);
    let mut columns = Columns::default();
    let mut pos = 0;
    // Where the current region-faced run began, once one is open. A region that
    // was already open on entry starts its run at the beginning of the line.
    let mut run_start = 0;

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
                match inside_event(region, line, pos, open, &mut ahead) {
                    Some(Event::Skip(_, resume)) => pos = resume,
                    Some(Event::Open(id, _, resume)) => {
                        // Nested, and faced as the region around it is, so
                        // the run goes on through it.
                        state.push(id);
                        pos = resume;
                    }
                    Some(Event::Close(_, resume)) => {
                        push_span(
                            &mut spans,
                            &mut columns,
                            line,
                            (run_start, resume),
                            region.face,
                        );
                        state.pop();
                        pos = resume;
                        run_start = resume;
                    }
                    _ => {
                        // Nothing closes it on this line: the rest belongs to
                        // the region, and it stays open for the next line.
                        push_span(
                            &mut spans,
                            &mut columns,
                            line,
                            (run_start, line.len()),
                            region.face,
                        );
                        break;
                    }
                }
            }
            None => match top_level_event(grammar, line, pos, &mut ahead) {
                Some(Event::Open(id, start, resume)) => {
                    state.push(id);
                    run_start = start;
                    pos = resume;
                }
                Some(Event::Rule(face, start, end, resume)) => {
                    push_span(&mut spans, &mut columns, line, (start, end), face);
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

/// The character column of a byte offset in a line, counted on from the last
/// one asked for: spans come in order, and counting each from the start of
/// the line would cost its length once per span.
#[derive(Default)]
struct Columns {
    byte: usize,
    column: usize,
}

impl Columns {
    fn of(&mut self, line: &str, byte: usize) -> usize {
        let byte = byte.min(line.len());
        if byte < self.byte {
            *self = Columns::default();
        }
        self.column += line[self.byte..byte].chars().count();
        self.byte = byte;
        self.column
    }
}

/// Adds to SPANS the span of LINE from START to END, byte offsets, in
/// character columns. An empty span is dropped: it colours nothing.
fn push_span(
    spans: &mut Vec<SyntaxSpan>,
    columns: &mut Columns,
    line: &str,
    (start, end): (usize, usize),
    face: Face,
) {
    if start >= end {
        return;
    }
    let start = columns.of(line, start);
    let end = columns.of(line, end);
    spans.push(SyntaxSpan { start, end, face });
}

/// Returns the next thing that matters inside REGION, the one at OPEN in the
/// grammar, from POS: its escape, its end or, when it nests, another of
/// itself, whichever comes first.
fn inside_event(
    region: &SyntaxRegion,
    line: &str,
    pos: usize,
    open: usize,
    ahead: &mut Ahead,
) -> Option<Event> {
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
        && let Some(m) = ask(&mut ahead.escapes[open], pos, |from| {
            find_nonempty(escape, line, from)
        })
    {
        offer(Event::Skip(m.start, m.end));
    }
    // An end may match nothing -- `$` closes the region at the end of the
    // line -- since closing a region is a step of its own.
    if let Some(m) = ask(&mut ahead.ends[open], pos, |from| {
        find_any(&region.end, line, from)
    }) {
        offer(Event::Close(m.start, m.end));
    }
    if region.nestable
        && let Some(m) = ask(&mut ahead.begins[open], pos, |from| {
            find_nonempty(&region.begin, line, from)
        })
    {
        offer(Event::Open(open, m.start, m.end));
    }
    best
}

/// Returns the next thing that matters outside any region, from POS: the
/// earliest region opening or rule match. Regions are offered first, so that
/// a region wins a tie, the same way every time.
fn top_level_event(grammar: &Grammar, line: &str, pos: usize, ahead: &mut Ahead) -> Option<Event> {
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
        if let Some(m) = ask(&mut ahead.begins[id], pos, |from| {
            find_nonempty(&region.begin, line, from)
        }) {
            offer(Event::Open(id, m.start, m.end));
        }
    }
    for (id, rule) in grammar.rules.iter().enumerate() {
        let Some(m) = ask(&mut ahead.rules[id], pos, |from| {
            let captures = rule.pattern.captures_at(line, from)?;
            let whole = captures.get(0).expect("a capture set has a group zero");
            Some(Found {
                start: whole.start(),
                end: whole.end(),
                faced: captures
                    .get(rule.group)
                    .map(|faced| (faced.start(), faced.end())),
            })
        }) else {
            continue;
        };
        // A rule whose group took no part in the match colours nothing, and
        // the scan still steps over the whole match.
        let (start, end) = m.faced.unwrap_or((m.start, m.start));
        offer(Event::Rule(rule.face, start, end, m.end));
    }
    best
}
