//! `fuzzy-filter`: which candidates a pattern matches as an abbreviation of
//! them, and in what order.
//!
//! Design: `doc/architettura-editor.typ, <completamento>`.
use super::*;
use crate::primitives::completion::candidate_value;

/// Whether the character at INDEX of CHARS begins a word, which is where an
/// abbreviation is made from: `cap` is the starts of `completion-at-point`.
fn is_word_start(chars: &[char], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let previous = chars[index - 1];
    let current = chars[index];
    // A separator before it, or a lowercase-to-uppercase step -- which is the
    // same boundary written the other way, in `camelCase` rather than in
    // `kebab-case`.
    matches!(previous, '-' | '_' | '/' | '.' | ' ' | ':' | '\\')
        || (previous.is_lowercase() && current.is_uppercase())
}

/// Returns how well CANDIDATE matches PATTERN, or `None` when it does not. It
/// matches when PATTERN's characters come in it in order, not necessarily
/// together, each taken where it first comes. A character at a word start or
/// next to the one before scores more, PATTERN written whole in CANDIDATE
/// more again, and a long candidate a little less.
fn fuzzy_score(pattern: &str, candidate: &str) -> Option<i64> {
    let needle: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = candidate.chars().collect();
    let hay_lower: Vec<char> = hay
        .iter()
        .flat_map(|c| c.to_lowercase())
        .collect::<Vec<_>>();
    // `to_lowercase` can yield more than one character for some scripts, which
    // would put the two vectors out of step and make every index below wrong.
    // Falling back to a plain case-sensitive walk is a worse match for such a
    // candidate and still a correct one.
    let hay_lower: &[char] = if hay_lower.len() == hay.len() {
        &hay_lower
    } else {
        &hay
    };

    let mut score = 0i64;
    let mut at = 0usize;
    let mut previous_match: Option<usize> = None;
    for wanted in needle {
        let found = hay_lower[at..].iter().position(|c| *c == wanted)? + at;
        score += 1;
        if is_word_start(&hay, found) {
            score += 9;
        }
        if previous_match == Some(found.saturating_sub(1)) && found > 0 {
            score += 8;
        }
        previous_match = Some(found);
        at = found + 1;
    }
    // The pattern whole beats its letters scattered, and at the start beats
    // in the middle: otherwise `abc` would score `a-b-c-x` as high as `abc`,
    // three word starts weighing what a run of three does.
    let literal = {
        let needle_lower: String = pattern.to_lowercase();
        let hay_lower: String = candidate.to_lowercase();
        if hay_lower.starts_with(&needle_lower) {
            25
        } else if hay_lower.contains(&needle_lower) {
            15
        } else {
            0
        }
    };
    // Gentle, and integer division on purpose: it separates candidates that
    // differ a lot in length without letting length overrule a much better
    // match.
    Some(score + literal - (hay.len() as i64) / 4)
}

pub const FUZZY_FILTER_DOC: &str = "(fuzzy-filter PATTERN CANDIDATES): Return the candidates \
         PATTERN matches, best first. A candidate matches when PATTERN's characters come in it in \
         order, not necessarily together, case ignored; characters at the start of a word or next \
         to each other count for more, and a longer candidate for less. An empty PATTERN keeps \
         them all, in their order. A candidate is a string, or a (VALUE . DESCRIPTION) pair \
         matched by VALUE.\n\n\
         Example:\n\
         (fuzzy-filter \"cap\" '(\"completion-at-point\" \"copy\" \"cap\"))";

primitive!(fuzzy_filter, args, _env, ctx, {
    exact_arity(args, 2)?;
    let pattern = match &args[0] {
        ELispExp::String(text) | ELispExp::Symbol(text) => text.to_string(),
        other if other.is_nil() => String::new(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let candidates = args::list(&args[1]);
    // A unit of fuel for each candidate, before any is scored: the work grows
    // with the list, and one unit for the call would not.
    ctx.consume_fuel(u32::try_from(candidates.len()).unwrap_or(u32::MAX))?;
    let mut scored: Vec<(i64, usize, ELispExp<B>)> = Vec::new();
    for (position, item) in candidates.into_iter().enumerate() {
        let Some(value) = candidate_value(&item) else {
            continue;
        };
        if let Some(score) = fuzzy_score(&pattern, &value) {
            scored.push((score, position, item));
        }
    }
    // Highest score first; ties broken by the order the candidates arrived in,
    // so a source that sorted its own output keeps that order among equals
    // rather than having it shuffled by an unstable comparison.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    Ok(ELispExp::proper_list(
        scored.into_iter().map(|(_, _, item)| item).collect(),
    ))
});

/// Registers the primitives of fuzzy matching.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("fuzzy-filter", fuzzy_filter, FUZZY_FILTER_DOC);
}
