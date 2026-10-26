//! Reading one argument out of a primitive's argument list: whether it is
//! there, whether it is the right kind, and what the error says when it is
//! not. Nothing here knows what an argument means.
//!
//! Design: `doc/architettura-editor.typ, <lettori-argomenti>`.
use super::*;

/// Returns ARG as a string, which it must already be.
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

/// Returns ARG as a name, written either way.
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

/// Returns ARG as a name, or the empty string when there is nothing usable
/// there.
///
/// For the primitives whose missing argument is not an error but a default --
/// a replacement of nothing is a deletion, and that is a thing to ask for.
pub(crate) fn name_or_empty<B: BufferTrait>(arg: Option<&ELispExp<B>>) -> String {
    name(arg).unwrap_or_default()
}

/// Returns ARG as a name, or FALLBACK when it is absent or empty.
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

/// Returns ARG as a name, or `None` when it was not given at all.
///
/// Absent and nil are both "not given": a Lisp caller passing an optional
/// argument through from its own optional argument hands on a nil rather than
/// shortening the list, and the two have to mean the same thing or every such
/// caller has to special-case it.
///
/// EXPECTED is the one part of this that differs between callers and the one
/// part a Lisp author ever reads -- "Symbol naming a category" and "String
/// naming a buffer" are the same check and not the same complaint -- so it
/// is a parameter, and the four-armed `match` behind it is written once.
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

/// Returns the first of ARGS as a signed count of steps: 1 when it is absent,
/// rounded down, and an error when it is not a finite number. A negative
/// count is the same move the other way.
pub(crate) fn movement_count<B: BufferTrait>(
    args: &[ELispExp<B>],
) -> Result<isize, EvalError<EditorState<B>>> {
    match args.first() {
        None => Ok(1),
        Some(ELispExp::Number(n)) if n.is_finite() => Ok(n.floor() as isize),
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: "finite Number".into(),
            got: other.clone(),
        }),
    }
}

// TODO(CMD-6): a negative count becomes 0, so the deletions that read it do
// nothing where Emacs deletes backwards.
// TODO(CMD-3): nor is a count bounded.
/// Returns the first of ARGS as a number of repetitions: 1 when it is absent,
/// rounded down and at least 0, and an error when it is not a number.
pub(crate) fn repeat_count<B: BufferTrait>(
    args: &[ELispExp<B>],
) -> Result<usize, EvalError<EditorState<B>>> {
    match args.first() {
        None => Ok(1),
        Some(ELispExp::Number(n)) => Ok(n.floor().max(0.0) as usize),
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: other.clone(),
        }),
    }
}

/// Returns VALUE as `(quote VALUE)`, so that a form built in Rust hands it
/// back whole.
///
/// The callbacks the asking primitives are handed are already values -- a
/// lambda, a symbol naming a function -- and they have to survive being
/// written into a form that will later be evaluated. `quote` is exactly that:
/// it hands back its argument untouched.
pub(crate) fn quoted<B: BufferTrait>(value: ELispExp<B>) -> ELispExp<B> {
    ELispExp::form(vec![ELispExp::symbol("quote".into()), value])
}
