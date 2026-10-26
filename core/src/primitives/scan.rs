//! `scan-buffer`: every match of a pattern in a buffer, found in one pass and
//! answered as the entries of a list of results.
//!
//! Design: `doc/architettura-editor.typ, <scan-buffer>`.
use super::*;
use crate::primitives::results::entry_form;
use crate::text::results::{Entry, KIND_BUFFER};
use crate::text::search::{Pattern, Scan};

/// How many matches a scan collects before it gives up, when nothing says.
const DEFAULT_LIMIT: usize = 10_000;

/// Returns SCAN as Lisp sees it, (TRUNCATED ENTRIES): each entry the list
/// `entry_form` makes of a match found in SOURCE, of KIND.
fn answer<B: BufferTrait>(kind: &str, source: &str, scan: &Scan) -> ELispExp<B> {
    ELispExp::proper_list(vec![
        ELispExp::boolean(scan.truncated),
        ELispExp::proper_list(
            scan.found
                .iter()
                .map(|found| entry_form(&Entry::new(kind, source, found)))
                .collect(),
        ),
    ])
}

/// Returns the limit at INDEX in ARGS, or the default when there is no
/// number of at least 0 there.
fn limit_arg<B: BufferTrait>(args: &[ELispExp<B>], index: usize) -> usize {
    match args.get(index) {
        Some(ELispExp::Number(limit)) if *limit >= 0.0 => *limit as usize,
        _ => DEFAULT_LIMIT,
    }
}

pub const SCAN_BUFFER_DOC: &str = "(scan-buffer PATTERN &optional REGEXP LIMIT BUFFER): Return \
         every match of PATTERN in BUFFER, or in the current buffer, as (TRUNCATED ENTRIES). An \
         entry is (KIND SOURCE LINE COLUMN OFFSET TEXT MATCH-START MATCH-END), the last two being \
         where the match is in its line, TEXT. TRUNCATED is t when LIMIT, 10000 if not given, \
         stopped the scan. With REGEXP non-nil, PATTERN is a regular expression; case follows \
         `case-fold-search'.\n\n\
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
    let pattern = Pattern::new(&pattern, regexp, crate::feature::isearch::case_fold(&env))
        .map_err(EvalError::RuntimeMessage)?;
    let Some(haystack) = ctx.with_buffer(&source, |buf| buf.text.to_string()) else {
        ctx.log_diagnostic(&format!("scan-buffer: no buffer called {source}"));
        return Ok(ELispExp::nil());
    };
    // Priced by the characters walked, not by the call: a megabyte for one
    // unit would let a loop do unbounded work inside a budget meant to bound
    // it.
    ctx.consume_fuel(u32::try_from(haystack.chars().count()).unwrap_or(u32::MAX))?;
    let scan = pattern.scan(&haystack, limit);
    Ok(answer(KIND_BUFFER, &source, &scan))
});

/// Registers `scan-buffer`.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("scan-buffer", scan_buffer, SCAN_BUFFER_DOC);
}
