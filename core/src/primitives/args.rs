//! Reading one argument out of a primitive's argument list.
//!
//! Every primitive receives a `&[ELispExp]` and has to decide what it was
//! handed. The deciding is mechanical -- is it there, is it the right kind,
//! what does the error say if it is not -- and it was written out by hand in
//! each of them, which is how four files came to hold four spellings of "the
//! first argument, as a string, or `WrongArgumentType`".
//!
//! What lives here is the *reading*, and only that. Nothing here knows what
//! any argument means, so there is no `pattern_arg` or `prompt_arg`: a pattern
//! and a prompt are both the first argument as a string, and the two names
//! were the only difference between two identical functions in two files.
//!
//! # Why a missing argument and a wrong one are the same error
//!
//! Both of these take an `Option`, which is what `args.get(n)` gives, and both
//! report an absent argument as a nil of the wrong type. That is deliberate,
//! and it is what the hand-written copies already did: a primitive called with
//! nothing where a string belongs has been called wrongly in the same way as
//! one called with a number there, and `(expected String, got nil)` says so in
//! the one sentence. A primitive whose *arity* is the point -- which must have
//! exactly two arguments, no more -- still says so itself, with
//! `WrongNumberOfArguments`, because that is a different complaint.
use super::*;

/// ARG as a string, which it must already be.
///
/// Strict about the kind: a symbol is not accepted, because the callers are
/// the ones taking text a person typed or a pattern to search for, and
/// `(occur 'TODO)` is far more likely to be a mistake than a shorthand.
pub(crate) fn text<B: BufferTrait>(
    arg: Option<&ELispExp<B>>,
) -> Result<String, EvalError<EditorState<B>>> {
    match arg {
        Some(ELispExp::String(text)) => Ok(text.to_string()),
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

/// ARG as a name, written either way.
///
/// A symbol *or* a string, because a name is a name: `(commandp 'find-file)`
/// and `(commandp "find-file")` ask the same question, and Lisp code that
/// built the name by `concat` has a string whether it wanted one or not.
pub(crate) fn name<B: BufferTrait>(
    arg: Option<&ELispExp<B>>,
) -> Result<String, EvalError<EditorState<B>>> {
    match arg {
        Some(ELispExp::String(name)) | Some(ELispExp::Symbol(name)) => Ok(name.to_string()),
        other => Err(EvalError::WrongArgumentType {
            expected: "Symbol or String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

/// ARG as a name, or the empty string when there is nothing usable there.
///
/// For the primitives whose missing argument is not an error but a default --
/// a replacement of nothing is a deletion, and that is a thing to ask for.
pub(crate) fn name_or_empty<B: BufferTrait>(arg: Option<&ELispExp<B>>) -> String {
    name(arg).unwrap_or_default()
}

/// ARG as a name, or FALLBACK when it is absent or empty.
///
/// The empty string counts as absent here, which the plain [`name`] does not:
/// a mode named `""` would be looked up, found missing, and leave the buffer
/// in no mode at all, whereas every caller of this one has a sensible default
/// sitting right there.
pub(crate) fn name_or<B: BufferTrait>(arg: Option<&ELispExp<B>>, fallback: &str) -> String {
    match name(arg) {
        Ok(found) if !found.is_empty() => found,
        _ => fallback.to_string(),
    }
}

/// ARG as a name, `None` when it was not given at all.
///
/// Absent and nil are both "not given": a Lisp caller passing an optional
/// argument through from its own optional argument hands on a nil rather than
/// shortening the list, and the two have to mean the same thing or every such
/// caller has to special-case it.
///
/// EXPECTED is the one part of this that differs between callers and the one
/// part a Lisp author ever reads -- "Symbol naming a category" and "String
/// naming a buffer" are the same check and not the same complaint -- so it
/// stays a parameter while the four-armed `match` behind it does not stay
/// copied into eight primitives.
pub(crate) fn optional_name<B: BufferTrait>(
    arg: Option<&ELispExp<B>>,
    expected: &str,
) -> Result<Option<String>, EvalError<EditorState<B>>> {
    match arg {
        None => Ok(None),
        Some(exp) if exp.is_nil() => Ok(None),
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) => Ok(Some(name.to_string())),
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: expected.into(),
            got: other.clone(),
        }),
    }
}

/// VALUE as `(quote VALUE)`, so that a form built in Rust hands it back whole.
///
/// The callbacks the asking primitives are handed are already values -- a
/// lambda, a symbol naming a function -- and they have to survive being
/// written into a form that will later be evaluated. `quote` is exactly that:
/// it hands back its argument untouched.
pub(crate) fn quoted<B: BufferTrait>(value: ELispExp<B>) -> ELispExp<B> {
    ELispExp::form(vec![ELispExp::symbol("quote".into()), value])
}
