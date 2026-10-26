//! `string-match` and the match data, as Emacs has them: the match answers
//! where it starts, and `match-beginning`, `match-end` and `match-string` read
//! its groups afterwards, in characters. The data is per thread. And
//! `regexp-opt`, which makes one pattern of a list of words.
//!
//! Design: `doc/architettura-editor.typ, <dati-corrispondenza>`.
use super::*;
use std::cell::RefCell;

/// Where each group of the last match began and ended, in characters.
/// Group 0 is the whole match; a group that did not take part is `None`.
type Groups = Vec<Option<(usize, usize)>>;

thread_local! {
    /// The groups of the last match on this thread: a Lisp job's step, on the
    /// executor's thread, cannot change what a command reads.
    static MATCH_DATA: RefCell<Groups> = const { RefCell::new(Vec::new()) };
}

/// Returns the error for an argument that is not EXPECTED: GOT, or nil when
/// nothing came.
fn wrong_type<B: BufferTrait>(
    expected: &str,
    got: Option<&ELispExp<B>>,
) -> EvalError<EditorState<B>> {
    EvalError::WrongArgumentType {
        expected: expected.into(),
        got: got.cloned().unwrap_or_else(ELispExp::nil),
    }
}

/// Returns ARG as a whole number, or the error when it is not one.
fn integer<B: BufferTrait>(arg: Option<&ELispExp<B>>) -> Result<i64, EvalError<EditorState<B>>> {
    match arg {
        Some(ELispExp::Number(n)) if n.fract() == 0.0 => Ok(*n as i64),
        other => Err(wrong_type("Integer", other)),
    }
}

/// Returns `(args-out-of-range ARGS...)`, the error Emacs signals for an index
/// that is not in the string.
fn out_of_range<B: BufferTrait>(args: &[ELispExp<B>]) -> EvalError<EditorState<B>> {
    EvalError::Signal {
        symbol: ELispExp::symbol("args-out-of-range".into()),
        data: ELispExp::proper_list(args.to_vec()),
    }
}

/// Returns the byte offset of character index CHAR in TEXT, or `None` past
/// the end.
fn byte_of(text: &str, char: usize) -> Option<usize> {
    text.char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .nth(char)
}

/// Runs REGEXP against STRING from character START, the three taken from
/// ARGS, and returns every group's span in characters, or `None` for no match.
///
/// An expression that does not compile signals `invalid-regexp`, as in Emacs:
/// answering nil would make a broken pattern look like a string that did not
/// match.
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

    // Compiled once per pattern, not once per call: the modules call this per
    // file or per line of output, and compiling costs a hundred times what
    // matching does.
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

    // Bytes to characters, counted from the front for each position: linear
    // in the string per group, and the strings matched are lines and names.
    let to_char = |byte: usize| subject[..byte].chars().count();
    Ok(Some(
        captures
            .iter()
            .map(|group| group.map(|m| (to_char(m.start()), to_char(m.end()))))
            .collect(),
    ))
}

pub const STRING_MATCH_DOC: &str = "(string-match REGEXP STRING &optional START): Return where in \
         STRING the first match of REGEXP starts, or nil, and record where its groups matched for \
         `match-beginning', `match-end' and `match-string'. The search begins at character START, \
         a negative one counting from the end, but ^ still means the start of STRING. Case is \
         matched exactly. A failed match leaves the match data alone, and a REGEXP that is not \
         valid signals `invalid-regexp'.\n\n\
         Example:\n\
         (string-match \"([a-z.]+):([0-9]+)\" \"see main.rs:42\") => 4";

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
         `string-match', but leave the match data alone.\n\n\
         Example:\n\
         (string-match-p \"[.]rs$\" \"main.rs\") => 4";

primitive!(string_match_p, args, _env, _ctx, {
    Ok(match search(args)? {
        Some(groups) => ELispExp::number(groups[0].map(|(start, _)| start).unwrap_or(0) as f64),
        None => ELispExp::nil(),
    })
});

/// Returns the span of the group the first of ARGS numbers, if it took part
/// in the last match; an error for a negative number.
fn group<B: BufferTrait>(
    args: &[ELispExp<B>],
) -> Result<Option<(usize, usize)>, EvalError<EditorState<B>>> {
    let n = integer(args.first())?;
    if n < 0 {
        return Err(out_of_range(&args[..1]));
    }
    Ok(MATCH_DATA.with(|data| data.borrow().get(n as usize).copied().flatten()))
}

pub const MATCH_BEGINNING_DOC: &str = "(match-beginning N): Return where group N of the last \
         match started, or nil if the group took no part. Group 0 is the whole match.";

primitive!(match_beginning, args, _env, _ctx, {
    exact_arity(args, 1)?;
    Ok(match group(args)? {
        Some((start, _)) => ELispExp::number(start as f64),
        None => ELispExp::nil(),
    })
});

pub const MATCH_END_DOC: &str = "(match-end N): Return where group N of the last match ended, \
         just past its last character, or nil if the group took no part.";

primitive!(match_end, args, _env, _ctx, {
    exact_arity(args, 1)?;
    Ok(match group(args)? {
        Some((_, end)) => ELispExp::number(end as f64),
        None => ELispExp::nil(),
    })
});

pub const MATCH_STRING_DOC: &str = "(match-string N STRING): Return the text group N of the last \
         match covered in STRING, which has to be the string matched, or nil if the group took no \
         part.\n\n\
         Example:\n\
         (string-match \"([a-z.]+):([0-9]+)\" \"main.rs:42\")\n\
         (match-string 2 \"main.rs:42\") => \"42\"";

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

pub const MATCH_DATA_DOC: &str = "(match-data): Return the last match as a list of positions: the \
         start and end of group 0, then of group 1, and so on, nil for both ends of a group that \
         took no part. Groups at the end that took no part are left off.\n\n\
         Example:\n\
         (progn (string-match \"(a)|(b)\" \"b\") (match-data)) => (0 1 nil nil 0 1)";

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

pub const SET_MATCH_DATA_DOC: &str = "(set-match-data LIST): Make LIST, in the shape `match-data' \
         returns, the match data; nil clears it.";

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

pub const REGEXP_OPT_DOC: &str = "(regexp-opt WORDS): Return a regular expression that matches \
         any of WORDS, a list of strings or symbols, every character special to a pattern \
         escaped, in a group that is not numbered. An empty list gives a pattern that matches \
         nothing.\n\n\
         Example:\n\
         (regexp-opt '(\"fn\" \"let\"))   ; \"(?:fn|let)\"";

primitive!(regexp_opt, args, _env, _ctx, {
    exact_arity(args, 1)?;
    let words = args::list(&args[0]);
    let mut parts = Vec::with_capacity(words.len());
    for word in &words {
        let (ELispExp::String(text) | ELispExp::Symbol(text)) = word else {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: word.clone(),
            });
        };
        parts.push(regex::escape(text));
    }
    // No words, a pattern that matches nothing: an empty alternation matches
    // everywhere, and a rule made of it would colour every position. A class
    // of no character, since the engine has no look-ahead to write `(?!)`.
    if parts.is_empty() {
        return Ok(ELispExp::string("[^\\s\\S]".into()));
    }
    Ok(ELispExp::string(format!("(?:{})", parts.join("|"))))
});

/// Registers `string-match`, the primitives of the match data, and
/// `regexp-opt`.
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
    into.function("regexp-opt", regexp_opt, REGEXP_OPT_DOC);
}
