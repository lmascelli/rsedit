//! Finding every match, rather than the next one.
//!
//! # What this is for
//!
//! `isearch` and `replace` ask "where is the next one" and get on with it.
//! Occur, grep, a linter reading its own output and anything else that means
//! to *list* what it found ask a different question, and asking it with
//! `search-forward` in a loop is quadratic -- see [`crate::search::Pattern::scan`],
//! which is the one pass this hands to Lisp.
//!
//! # The entry, and why its shape is fixed here
//!
//! Everything that produces a list of places in this editor will answer in the
//! same eight-element list:
//!
//! ```text
//! (KIND SOURCE LINE COLUMN OFFSET TEXT MATCH-START MATCH-END)
//! ```
//!
//! so that a view can be written once and shown results from anywhere. `KIND`
//! tells a buffer from a file, because a name like `*scratch*` and a path are
//! otherwise only distinguishable by guessing. `TEXT` is the whole line and
//! `MATCH-START`/`MATCH-END` are offsets *within it*, so a view highlights the
//! match without searching the line again -- and without being able to
//! disagree with the scan about where it was.
//!
//! Positional rather than named, like every other structured answer here --
//! `(START END)`, `(FILE LINE COLUMN)`, `(KEYS TARGET SOURCE)` -- and written
//! down in one place so the next producer copies it rather than inventing a
//! ninth field.
use super::*;
use crate::search::{Found, Pattern, Scan};

/// What a scan answers when it was asked about a buffer.
///
/// The other kind is `"file"`, which arrives with the verbs that scan one.
/// Named here rather than written out at each use so that the two cannot
/// drift apart by a capital letter.
pub const KIND_BUFFER: &str = "buffer";

/// How many matches a scan collects before it gives up, when nothing says.
const DEFAULT_LIMIT: usize = 10_000;

/// One match as Lisp sees it. See the module header for the shape.
pub(crate) fn entry<B: BufferTrait>(kind: &str, source: &str, found: &Found) -> ELispExp<B> {
    ELispExp::proper_list(vec![
        ELispExp::string(kind.to_string()),
        ELispExp::string(source.to_string()),
        ELispExp::number(found.line as f64),
        ELispExp::number(found.column as f64),
        ELispExp::number(found.start as f64),
        ELispExp::string(found.line_text.clone()),
        ELispExp::number(found.in_line() as f64),
        ELispExp::number(found.end_in_line() as f64),
    ])
}

/// A whole scan as Lisp sees it: (TRUNCATED ENTRIES).
pub(crate) fn answer<B: BufferTrait>(kind: &str, source: &str, scan: &Scan) -> ELispExp<B> {
    ELispExp::proper_list(vec![
        ELispExp::boolean(scan.truncated),
        ELispExp::proper_list(
            scan.found
                .iter()
                .map(|found| entry(kind, source, found))
                .collect(),
        ),
    ])
}

fn limit_arg<B: BufferTrait>(args: &[ELispExp<B>], index: usize) -> usize {
    match args.get(index) {
        Some(ELispExp::Number(limit)) if *limit >= 0.0 => *limit as usize,
        _ => DEFAULT_LIMIT,
    }
}

pub const SCAN_BUFFER_DOC: &str = "(scan-buffer PATTERN &optional REGEXP LIMIT BUFFER): Every \
         match of PATTERN in BUFFER -- the current one when it is omitted -- as \
         (TRUNCATED ENTRIES).\n\n\
         Each entry is (KIND SOURCE LINE COLUMN OFFSET TEXT MATCH-START MATCH-END): KIND is \
         \"buffer\", SOURCE the buffer's name, LINE counts from 1 and COLUMN from 0, OFFSET is \
         where the match begins in the whole buffer, TEXT is the line it is on, and the last \
         two are where the match sits *within that line* -- so a view can show and highlight it \
         without reading the buffer again.\n\n\
         TRUNCATED is t when LIMIT (10000 by default) stopped the scan before the end. A caller \
         that ignores it shows a partial list as though it were everything, which is how a \
         search comes to quietly not find what is there.\n\n\
         With REGEXP non-nil PATTERN is a regular expression, and `\\\\1' onwards are available \
         to whatever reads the entries. Case is folded as `case-fold-search' says, the same \
         variable the incremental search and replace read.\n\n\
         Matches do not overlap, and a pattern that can match nothing -- `x*' -- still \
         terminates: the scan steps one character past an empty match.\n\n\
         One pass over the text, rather than a search per match. That is the whole reason this \
         exists next to `search-forward': the loop is quadratic and this is not.\n\n\
         Example:\n\
         (scan-buffer \"TODO\") => (nil ((\"buffer\" \"notes.txt\" 3 4 57 \"    TODO: this\" 4 8)))";

primitive!(scan_buffer, args, env, ctx, {
    let pattern = match args.first() {
        Some(ELispExp::String(text)) => text.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    let regexp = args.get(1).is_some_and(|value| value.is_truthy());
    let limit = limit_arg(args, 2);
    let source = match args.get(3) {
        Some(ELispExp::String(name)) if !name.is_empty() => name.to_string(),
        Some(ELispExp::Symbol(name)) => name.to_string(),
        _ => ctx.get_current_buffer_name(),
    };
    let pattern = Pattern::new(&pattern, regexp, crate::isearch::case_fold(&env))
        .map_err(EvalError::RuntimeMessage)?;
    let Some(haystack) = ctx.with_buffer(&source, |buf| buf.text.to_string()) else {
        ctx.log_diagnostic(&format!("scan-buffer: no buffer called {source}"));
        return Ok(ELispExp::nil());
    };
    // Priced by the text walked, not by the call: a primitive that scans a
    // megabyte for one unit would let a loop do unbounded work inside a
    // budget meant to bound it. The same rule `directory-files-recursive'
    // follows, which charges for every path it walked past.
    ctx.consume_fuel(u32::try_from(haystack.chars().count()).unwrap_or(u32::MAX))?;
    let scan = pattern.scan(&haystack, limit);
    Ok(answer(KIND_BUFFER, &source, &scan))
});
