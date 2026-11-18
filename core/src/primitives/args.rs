//! Reading one argument out of a primitive's argument list: whether it is
//! there, whether it is the right kind, and what the error says when it is
//! not. Nothing here knows what an argument means.
//!
//! Design: `doc/architettura-editor.typ, <lettori-argomenti>`.
use super::*;
use crate::buffer::BufferRole;

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

/// Returns the elements of EXP, a list in either of the shapes one reaches a
/// primitive in: a cons chain, which is what a list quoted in source is, or a
/// `Form`, which is what a list a macro quotes from its own arguments still
/// is. Anything else, nil among it, has no elements.
pub(crate) fn list<B: BufferTrait>(exp: &ELispExp<B>) -> Vec<ELispExp<B>> {
    match exp {
        ELispExp::Form(items) => items.to_vec(),
        other => other.iter().collect(),
    }
}

/// Returns the first of ARGS as a signed count of steps: 1 when it is absent,
/// rounded down, and an error when it is not a finite number. A negative
/// count is the same move, or the same deletion, the other way.
///
/// Not bounded: a count is a number of steps, and every step stops at the
/// edge of the buffer, so a huge one means "as far as it goes". A caller that
/// adds it to a position does so saturating.
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

/// The most characters a width or a column given as a number may ask for: a
/// million, wider than any line, and four megabytes as a string. Past it the
/// number is refused rather than used: a string that size is more than the
/// system may give, and an allocation it refuses ends the process rather than
/// failing; and `format!` panics on a width past 65 535.
pub(crate) const WIDTH_LIMIT: usize = 1_000_000;

/// Returns `(args-out-of-range ARGS...)`, the error Emacs signals for an
/// argument outside what a function takes: an index past the end of a
/// string, a width past [`WIDTH_LIMIT`].
pub(crate) fn out_of_range<B: BufferTrait>(args: &[ELispExp<B>]) -> EvalError<EditorState<B>> {
    EvalError::Signal {
        symbol: ELispExp::symbol("args-out-of-range".into()),
        data: ELispExp::proper_list(args.to_vec()),
    }
}

/// Returns N as a width or a column: rounded down, 0 when negative or not a
/// number, and `(args-out-of-range N)` past [`WIDTH_LIMIT`], infinity
/// included.
pub(crate) fn width<B: BufferTrait>(n: f64) -> Result<usize, EvalError<EditorState<B>>> {
    // NaN goes to 0 here, as a negative does: `max` takes the number.
    let width = n.max(0.0).floor();
    if width > WIDTH_LIMIT as f64 {
        return Err(out_of_range(&[ELispExp::number(n)]));
    }
    Ok(width as usize)
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

/// Returns whether EXP is a keyword: a symbol whose name starts with `:`,
/// and is more than the colon.
pub(crate) fn is_keyword<B: BufferTrait>(exp: &ELispExp<B>) -> bool {
    matches!(exp, ELispExp::Symbol(name) if name.len() > 1 && name.starts_with(':'))
}

/// The keyword arguments a primitive was given after its positional ones:
/// each at most once, and each one the primitive knows.
pub(crate) struct Keywords<'a, B: BufferTrait> {
    given: Vec<(&'static str, &'a ELispExp<B>)>,
}

impl<'a, B: BufferTrait> Keywords<'a, B> {
    /// Returns the value given for KEYWORD, written with its colon, or
    /// `None` when it was not given.
    pub(crate) fn get(&self, keyword: &str) -> Option<&'a ELispExp<B>> {
        self.given
            .iter()
            .find(|(name, _)| *name == keyword)
            .map(|(_, value)| *value)
    }
}

/// A primitive's arguments, split: the positional ones and the keywords.
pub(crate) type Split<'a, B> = (&'a [ELispExp<B>], Keywords<'a, B>);

/// Splits ARGS, the arguments FUNCTION was called with, into the positional
/// ones, from REQUIRED to MAX_POSITIONAL of them, and the keyword pairs that
/// follow, every keyword one of KNOWN.
///
/// The keywords start at the first keyword past the required arguments, so a
/// call with none is read exactly as before there were any. Everything is
/// checked before anything is returned, so that a caller can refuse a call
/// before it changes anything: a keyword not in KNOWN, one given twice, one
/// with no value after it, and anything but a keyword where one belongs.
pub(crate) fn with_keywords<'a, B: BufferTrait>(
    function: &str,
    args: &'a [ELispExp<B>],
    required: usize,
    max_positional: usize,
    known: &[&'static str],
) -> Result<Split<'a, B>, EvalError<EditorState<B>>> {
    let split = args
        .iter()
        .enumerate()
        .skip(required)
        .find(|(_, arg)| is_keyword(arg))
        .map_or(args.len(), |(at, _)| at);
    let (positional, rest) = args.split_at(split);
    if positional.len() < required || positional.len() > max_positional {
        return Err(EvalError::WrongNumberOfArguments {
            expected: if positional.len() < required {
                required
            } else {
                max_positional
            },
            got: positional.len(),
        });
    }
    let mut given: Vec<(&'static str, &'a ELispExp<B>)> = Vec::new();
    for pair in rest.chunks(2) {
        let (true, ELispExp::Symbol(written)) = (is_keyword(&pair[0]), &pair[0]) else {
            return Err(EvalError::RuntimeMessage(format!(
                "{function}: {:?} where a keyword was expected",
                pair[0]
            )));
        };
        let Some(keyword) = known.iter().find(|known| ***known == ***written) else {
            return Err(EvalError::RuntimeMessage(format!(
                "{function}: unknown keyword {written}; it takes {}",
                known.join(" ")
            )));
        };
        if given.iter().any(|(name, _)| name == keyword) {
            return Err(EvalError::RuntimeMessage(format!(
                "{function}: the keyword {keyword} is given twice"
            )));
        }
        let Some(value) = pair.get(1) else {
            return Err(EvalError::RuntimeMessage(format!(
                "{function}: the keyword {keyword} has no value after it"
            )));
        };
        given.push((keyword, value));
    }
    Ok((positional, Keywords { given }))
}

/// Returns the argument a caller gave either as the positional argument at
/// INDEX or as KEYWORD, or `None` when it gave neither; an error naming
/// FUNCTION when it gave both, which would leave it to guess which was meant.
pub(crate) fn positional_or_keyword<'a, B: BufferTrait>(
    function: &str,
    positional: &'a [ELispExp<B>],
    index: usize,
    keywords: &Keywords<'a, B>,
    keyword: &str,
) -> Result<Option<&'a ELispExp<B>>, EvalError<EditorState<B>>> {
    match (positional.get(index), keywords.get(keyword)) {
        (Some(_), Some(_)) => Err(EvalError::RuntimeMessage(format!(
            "{function}: given both as argument {} and as {keyword}",
            index + 1
        ))),
        (from_position, from_keyword) => Ok(from_position.or(from_keyword)),
    }
}

/// Returns ARG as the role it names, one CTX ships or its configuration
/// defined, or `None` when it was not given or is nil; an error naming
/// FUNCTION for anything else, a name no role has among it.
pub(crate) fn optional_role<B: BufferTrait>(
    function: &str,
    arg: Option<&ELispExp<B>>,
    ctx: &EditorState<B>,
) -> Result<Option<BufferRole>, EvalError<EditorState<B>>> {
    let Some(name) = optional_name(arg, "Symbol naming a buffer role")? else {
        return Ok(None);
    };
    ctx.modes(|modes| modes.role_named(&name))
        .map(Some)
        .ok_or_else(|| {
            let known = ctx.modes(|modes| modes.roles());
            EvalError::RuntimeMessage(format!(
                "{function}: no buffer role is called {name}; the roles are {}",
                known
                    .iter()
                    .map(|role| role.name())
                    .collect::<Vec<_>>()
                    .join(" ")
            ))
        })
}

/// Returns ARG as the role it names, as [`optional_role`] does, but required:
/// nil or nothing is an error too.
pub(crate) fn role<B: BufferTrait>(
    function: &str,
    arg: Option<&ELispExp<B>>,
    ctx: &EditorState<B>,
) -> Result<BufferRole, EvalError<EditorState<B>>> {
    optional_role(function, arg, ctx)?.ok_or_else(|| EvalError::WrongArgumentType {
        expected: "Symbol naming a buffer role".into(),
        got: arg.cloned().unwrap_or_else(ELispExp::nil),
    })
}
