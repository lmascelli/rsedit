//! Matching a regular expression against a string, the way Emacs does it.
//!
//! `string-match` answers *where* the match starts, and leaves the details --
//! where each group began and ended -- in the **match data**, which
//! `match-beginning`, `match-end` and `match-string` read afterwards. That is
//! the shape every piece of Emacs Lisp ever written expects, and code ported
//! from Emacs should work here without being restructured.
//!
//! # The cost of the shape, and how it is contained
//!
//! Match data is state that survives the call, so a function called between a
//! match and the read of it can overwrite it. Emacs' answer is
//! `save-match-data`, defined in `commands.lisp`, and the same convention
//! applies here: a function that matches on someone else's behalf wraps itself
//! in it. `string-match-p` sets nothing, for callers that only want a yes/no.
//!
//! The data is **per thread**. A Lisp worker on another thread matching its
//! own strings must not change what the command on the main thread is about to
//! read, and the two cannot coordinate. Fibers on one thread share it, as
//! Emacs timers share it with whatever they interrupt -- `save-match-data` is
//! the answer there too.
//!
//! # Positions are characters
//!
//! The regex engine reports byte offsets. Lisp sees character indices, as
//! `substring` and `length` do, so a match in a string containing `è` reports
//! the same numbers it would in Emacs.
//!
//! # What is not Emacs
//!
//! The *syntax* of the regular expressions: groups are `(...)`, not `\(...\)`.
//! It is the engine the syntax rules and the search commands use, which stays
//! linear by having no lookahead and no backreferences.
use super::*;
use std::cell::RefCell;

/// Where each group of the last match began and ended, in characters.
/// Group 0 is the whole match; a group that did not take part is `None`.
type Groups = Vec<Option<(usize, usize)>>;

thread_local! {
    static MATCH_DATA: RefCell<Groups> = const { RefCell::new(Vec::new()) };
}

fn wrong_type<B: BufferTrait>(
    expected: &str,
    got: Option<&ELispExp<B>>,
) -> EvalError<EditorState<B>> {
    EvalError::WrongArgumentType {
        expected: expected.into(),
        got: got.cloned().unwrap_or_else(ELispExp::nil),
    }
}

/// ARG as a whole number, which it must be.
fn integer<B: BufferTrait>(arg: Option<&ELispExp<B>>) -> Result<i64, EvalError<EditorState<B>>> {
    match arg {
        Some(ELispExp::Number(n)) if n.fract() == 0.0 => Ok(*n as i64),
        other => Err(wrong_type("Integer", other)),
    }
}

/// `(args-out-of-range ...)`, which is what Emacs signals for an index that
/// is not in the string.
fn out_of_range<B: BufferTrait>(args: &[ELispExp<B>]) -> EvalError<EditorState<B>> {
    EvalError::Signal {
        symbol: ELispExp::symbol("args-out-of-range".into()),
        data: ELispExp::proper_list(args.to_vec()),
    }
}

/// The byte offset of character index CHAR in TEXT, or `None` past the end.
fn byte_of(text: &str, char: usize) -> Option<usize> {
    text.char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .nth(char)
}

/// Run REGEXP against STRING from character START, answering every group's
/// span in characters -- or `None` for no match.
///
/// An expression that does not compile is an error, `invalid-regexp`, as in
/// Emacs: a pattern that cannot match anything is a bug in the pattern, and
/// answering nil would make it look like a string that did not match.
fn search<B: BufferTrait>(
    args: &[ELispExp<B>],
) -> Result<Option<Groups>, EvalError<EditorState<B>>> {
    if !(2..=3).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let pattern = args::text(args.first())?;
    let subject = args::text(args.get(1))?;

    let length = subject.chars().count() as i64;
    let start = match args.get(2) {
        None => 0,
        Some(arg) if arg.is_nil() => 0,
        // Negative counts back from the end, as in Emacs.
        Some(arg) => {
            let given = integer(Some(arg))?;
            let start = if given < 0 { length + given } else { given };
            if !(0..=length).contains(&start) {
                return Err(out_of_range(&[ELispExp::string(subject), arg.clone()]));
            }
            start as usize
        }
    };

    // Compiled once per pattern rather than once per call. The modules call
    // this per element -- per file, per line of output -- and compiling a
    // pattern costs a hundred times what matching one does.
    let compiled = crate::text::search::compiled(&pattern).map_err(|why| EvalError::Signal {
        symbol: ELispExp::symbol("invalid-regexp".into()),
        data: ELispExp::proper_list(vec![ELispExp::string(why.to_string())]),
    })?;

    // From START, but with `^` still meaning the start of the *string*: the
    // engine is handed the whole of it and told where to begin, which is the
    // behaviour Emacs has.
    let from = byte_of(&subject, start).unwrap_or(subject.len());
    let Some(captures) = compiled.captures_at(&subject, from) else {
        return Ok(None);
    };

    // Bytes to characters. Counted from the front for each position, which is
    // linear in the string per group -- the strings matched are lines and
    // names, and a cleverer count would be code to get wrong for no gain.
    let to_char = |byte: usize| subject[..byte].chars().count();
    Ok(Some(
        captures
            .iter()
            .map(|group| group.map(|m| (to_char(m.start()), to_char(m.end()))))
            .collect(),
    ))
}

pub const STRING_MATCH_DOC: &str = "(string-match REGEXP STRING &optional START): The index in STRING \
         where the first match for REGEXP starts, or nil if there is none.\n\n\
         The search begins at character START when it is given -- negative counts from the end -- \
         but `^' still means the start of STRING.\n\n\
         A match is recorded in the match data, which `match-beginning', `match-end' and \
         `match-string' read: group 0 is the whole match, and 1 onwards the parenthesised groups. \
         A failed match leaves the match data as it was. Use `string-match-p' to ask without \
         changing it, and `save-match-data' around anything that matches on somebody else's \
         behalf.\n\n\
         Signals `invalid-regexp' if REGEXP does not compile. The syntax is the one the search \
         commands use -- groups are (...), not \\(...\\) -- with no lookahead and no \
         backreferences.\n\n\
         Example:\n\
         (when (string-match \\\"([a-z.]+):([0-9]+)\\\" \\\"see main.rs:42\\\")\n\
         \x20\x20(match-string 2 \\\"see main.rs:42\\\")) => \\\"42\\\"";

primitive!(string_match, args, _env, _ctx, {
    match search(args)? {
        Some(groups) => {
            let start = groups[0].map(|(start, _)| start).unwrap_or(0);
            MATCH_DATA.with(|data| *data.borrow_mut() = groups);
            Ok(ELispExp::number(start as f64))
        }
        None => Ok(ELispExp::nil()),
    }
});

pub const STRING_MATCH_P_DOC: &str = "(string-match-p REGEXP STRING &optional START): Like \
         `string-match', but leaves the match data alone.\n\n\
         For the question \"does it match?\" -- a filter, a predicate -- where nothing will read \
         the groups and nothing should lose the ones it was about to read.\n\n\
         Example:\n\
         (string-match-p \\\"[.]rs$\\\" \\\"main.rs\\\") => 4";

primitive!(string_match_p, args, _env, _ctx, {
    Ok(match search(args)? {
        Some(groups) => ELispExp::number(groups[0].map(|(start, _)| start).unwrap_or(0) as f64),
        None => ELispExp::nil(),
    })
});

/// The span of group N of the last match, if it took part.
fn group<B: BufferTrait>(
    args: &[ELispExp<B>],
) -> Result<Option<(usize, usize)>, EvalError<EditorState<B>>> {
    let n = integer(args.first())?;
    if n < 0 {
        return Err(out_of_range(&args[..1]));
    }
    Ok(MATCH_DATA.with(|data| data.borrow().get(n as usize).copied().flatten()))
}

pub const MATCH_BEGINNING_DOC: &str = "(match-beginning N): Where group N of the last match \
         started, or nil if that group did not take part. Group 0 is the whole match.\n\n\
         Example:\n\
         (string-match \\\"b+\\\" \\\"abbc\\\") (match-beginning 0) => 1";

primitive!(match_beginning, args, _env, _ctx, {
    exact_arity(args, 1)?;
    Ok(match group(args)? {
        Some((start, _)) => ELispExp::number(start as f64),
        None => ELispExp::nil(),
    })
});

pub const MATCH_END_DOC: &str = "(match-end N): Where group N of the last match ended -- the \
         index just past it -- or nil if that group did not take part.\n\n\
         Example:\n\
         (string-match \\\"b+\\\" \\\"abbc\\\") (match-end 0) => 3";

primitive!(match_end, args, _env, _ctx, {
    exact_arity(args, 1)?;
    Ok(match group(args)? {
        Some((_, end)) => ELispExp::number(end as f64),
        None => ELispExp::nil(),
    })
});

pub const MATCH_STRING_DOC: &str = "(match-string N STRING): The text group N of the last match \
         covered in STRING, or nil if that group did not take part.\n\n\
         STRING has to be the one that was matched: the match data records positions, not text. \
         Emacs reads the current buffer when STRING is left out; this editor has no buffer regexp \
         search to have matched there, so leaving it out is an error rather than a quiet read of \
         the wrong text.\n\n\
         Example:\n\
         (let ((line \\\"main.rs:42\\\"))\n\
         \x20\x20(when (string-match \\\"([a-z.]+):([0-9]+)\\\" line)\n\
         \x20\x20\x20\x20(match-string 1 line))) => \\\"main.rs\\\"";

primitive!(match_string, args, _env, _ctx, {
    exact_arity(args, 2)?;
    let subject = args::text(args.get(1))?;
    let Some((start, end)) = group(args)? else {
        return Ok(ELispExp::nil());
    };
    // Checked rather than trusted: a string that is not the one matched may
    // be shorter than the positions recorded against the other.
    let (Some(from), Some(to)) = (byte_of(&subject, start), byte_of(&subject, end)) else {
        return Err(out_of_range(args));
    };
    Ok(ELispExp::string(subject[from..to].to_string()))
});

pub const MATCH_DATA_DOC: &str = "(match-data): The last match as a list of positions: the start \
         and end of group 0, then of group 1, and so on, with nil for both ends of a group that \
         did not take part. Trailing groups that did not take part are left off.\n\n\
         What `save-match-data' saves, and `set-match-data' puts back.\n\n\
         Example:\n\
         (string-match \\\"(a)|(b)\\\" \\\"b\\\") (match-data) => (0 1 nil nil 0 1)";

primitive!(match_data, args, _env, _ctx, {
    exact_arity(args, 0)?;
    let groups = MATCH_DATA.with(|data| data.borrow().clone());
    let used = groups
        .iter()
        .rposition(Option::is_some)
        .map_or(0, |last| last + 1);
    let positions = groups[..used]
        .iter()
        .flat_map(|group| match group {
            Some((start, end)) => [
                ELispExp::number(*start as f64),
                ELispExp::number(*end as f64),
            ],
            None => [ELispExp::nil(), ELispExp::nil()],
        })
        .collect();
    Ok(ELispExp::proper_list(positions))
});

pub const SET_MATCH_DATA_DOC: &str = "(set-match-data LIST): Make LIST the match data, in the \
         shape `match-data' returns. nil clears it.\n\n\
         Example:\n\
         (let ((saved (match-data)))\n\
         \x20\x20(string-match \\\"x\\\" \\\"xyz\\\")\n\
         \x20\x20(set-match-data saved))";

primitive!(set_match_data, args, _env, _ctx, {
    exact_arity(args, 1)?;
    let list = &args[0];
    if !list.is_nil() && !matches!(list, ELispExp::Cons(_)) {
        return Err(wrong_type("List", Some(list)));
    }
    let positions: Vec<ELispExp<B>> = list.iter().collect();
    let position = |item: &ELispExp<B>| match item {
        ELispExp::Number(n) if *n >= 0.0 && n.fract() == 0.0 => Ok(Some(*n as usize)),
        other if other.is_nil() => Ok(None),
        other => Err(wrong_type("Integer or nil", Some(other))),
    };
    let mut groups = Vec::with_capacity(positions.len() / 2);
    for pair in positions.chunks(2) {
        let start = position(&pair[0])?;
        let end = match pair.get(1) {
            Some(end) => position(end)?,
            None => None,
        };
        groups.push(start.zip(end));
    }
    MATCH_DATA.with(|data| *data.borrow_mut() = groups);
    Ok(ELispExp::nil())
});

/// Register the matching primitives.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("string-match", string_match, STRING_MATCH_DOC);
    into.function("string-match-p", string_match_p, STRING_MATCH_P_DOC);
    into.function("match-beginning", match_beginning, MATCH_BEGINNING_DOC);
    into.function("match-end", match_end, MATCH_END_DOC);
    into.function("match-string", match_string, MATCH_STRING_DOC);
    into.function("match-data", match_data, MATCH_DATA_DOC);
    into.function("set-match-data", set_match_data, SET_MATCH_DATA_DOC);
}
