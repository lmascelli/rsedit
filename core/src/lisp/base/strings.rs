//! Strings, and converting to and from them.
//!
//! Characters rather than bytes throughout: `length` on a string counts
//! characters, and so does `substring`. A string in this Lisp is a `String`
//! behind an `Arc`, so passing one costs a pointer.
use super::*;

const MAKE_STRING_DOC: &str = "(make-string COUNT STRING): Return COUNT copies of STRING's first \
                 character, joined. A COUNT of zero or less gives the empty string.\n\n\
                 Chiefly for padding: laying anything out in columns means writing the spaces \
                 between them, and building those a character at a time in Lisp costs a cons \
                 per space.\n\n\
                 Example:\n\
                 (concat \"a\" (make-string 3 \" \") \"b\") => \"a   b\"";

fn primitive_make_string<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 2)?;
    let count = expect_number(&args[0])?;
    let fill = expect_string(&args[1])?;
    // The first character, as `self-insert` takes it -- this Lisp has no
    // character type, so a one-character string is how one is written.
    let Some(c) = fill.chars().next() else {
        return Ok(LispExp::string(String::new()));
    };
    let count = if count.is_finite() {
        count.max(0.0) as usize
    } else {
        0
    };
    Ok(LispExp::string(String::from(c).repeat(count)))
}

const CONCAT_DOC: &str = "(concat &rest STRINGS): Concatenate STRINGS into a single \
                 string.\n\n\
                 Example:\n\
                 (concat \"foo\" \"-\" \"bar\") => \"foo-bar\"";

fn primitive_concat<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    let mut result = String::new();
    for arg in args {
        result.push_str(&expect_string(arg)?);
    }
    Ok(LispExp::string(result))
}

const STRING_EQ_DOC: &str = "(string= STRING1 STRING2): Return t if STRING1 and STRING2 \
                 have the same contents, nil otherwise.\n\n\
                 Example:\n\
                 (string= \"foo\" \"foo\") => t\n\
                 (string= \"foo\" \"bar\") => nil";

fn primitive_string_eq<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 2)?;
    Ok(LispExp::boolean(
        expect_string(&args[0])? == expect_string(&args[1])?,
    ))
}

const STRING_LT_DOC: &str = "(string< STRING1 STRING2): Return t if STRING1 sorts before \
                 STRING2 lexicographically, nil otherwise.\n\n\
                 Example:\n\
                 (string< \"abc\" \"abd\") => t\n\
                 (string< \"abd\" \"abc\") => nil";

fn primitive_string_lt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 2)?;
    Ok(LispExp::boolean(
        expect_string(&args[0])? < expect_string(&args[1])?,
    ))
}

const SUBSTRING_DOC: &str = "(substring STRING &optional START END): Return the substring \
                 of STRING from START (inclusive, default 0) to END \
                 (exclusive, default the length of STRING). Negative indices \
                 count from the end of STRING. Signals a runtime error if \
                 START is after END.\n\n\
                 Example:\n\
                 (substring \"hello\" 1 3)  => \"el\"\n\
                 (substring \"hello\" -3)   => \"llo\"\n\
                 (substring \"hello\" -3 -1) => \"ll\"";

fn primitive_substring<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.is_empty() || args.len() > 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let s = expect_string(&args[0])?;
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;
    let normalize = |i: i64| -> i64 { if i < 0 { (len + i).max(0) } else { i.min(len) } };

    let start = if args.len() >= 2 {
        normalize(expect_number(&args[1])? as i64)
    } else {
        0
    };
    let end = if args.len() == 3 {
        normalize(expect_number(&args[2])? as i64)
    } else {
        len
    };
    if start > end {
        return Err(EvalError::RuntimeMessage(
            "Args out of range for substring".into(),
        ));
    }
    Ok(LispExp::string(
        chars[start as usize..end as usize].iter().collect(),
    ))
}

const UPCASE_DOC: &str = "(upcase STRING): Return a copy of STRING with all letters \
                 uppercased.\n\n\
                 Example:\n\
                 (upcase \"hello\") => \"HELLO\"";

string_op!(primitive_upcase, to_uppercase);

const DOWNCASE_DOC: &str = "(downcase STRING): Return a copy of STRING with all letters \
                 lowercased.\n\n\
                 Example:\n\
                 (downcase \"HELLO\") => \"hello\"";

string_op!(primitive_downcase, to_lowercase);

const NUMBER_TO_STRING_DOC: &str = "(number-to-string NUMBER): Return the decimal string \
                 representation of NUMBER, omitting the decimal point for \
                 integral values.\n\n\
                 Example:\n\
                 (number-to-string 5)   => \"5\"\n\
                 (number-to-string 5.5) => \"5.5\"";

fn primitive_number_to_string<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 1)?;
    Ok(LispExp::string(format_number(expect_number(&args[0])?)))
}

const STRING_TO_NUMBER_DOC: &str = "(string-to-number STRING): Parse STRING as a number and \
                 return it, ignoring leading/trailing whitespace. Returns 0 if \
                 STRING cannot be parsed.\n\n\
                 Example:\n\
                 (string-to-number \"42\")     => 42\n\
                 (string-to-number \"nope\")   => 0";

fn primitive_string_to_number<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 1)?;
    Ok(LispExp::number(
        expect_string(&args[0])?.trim().parse().unwrap_or(0.0),
    ))
}

const SYMBOL_NAME_DOC: &str = "(symbol-name SYMBOL): Return the name of SYMBOL as a \
                 string.\n\n\
                 Example:\n\
                 (symbol-name 'foo) => \"foo\"";

fn primitive_symbol_name<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 1)?;
    if let LispExp::Symbol(s) = &args[0] {
        Ok(LispExp::string(s.to_string()))
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "Symbol".into(),
            got: args[0].clone(),
        })
    }
}

const INTERN_DOC: &str = "(intern STRING): Return the symbol named STRING.\n\n\
                 Example:\n\
                 (intern \"foo\") => foo";

fn primitive_intern<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 1)?;
    Ok(LispExp::symbol(expect_string(&args[0])?))
}

const SPLIT_STRING_DOC: &str = "(split-string STRING &optional SEPARATORS): Split STRING into \
                 a list of substrings. With no SEPARATORS, splits on \
                 whitespace and discards empty pieces. With an empty \
                 SEPARATORS string, splits into individual characters. \
                 Otherwise splits on literal occurrences of SEPARATORS.\n\n\
                 Example:\n\
                 (split-string \"  a  b c \") => (\"a\" \"b\" \"c\")\n\
                 (split-string \"a,b,c\" \",\") => (\"a\" \"b\" \"c\")\n\
                 (split-string \"ab\" \"\")     => (\"a\" \"b\")";

fn primitive_split_string<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let s = expect_string(&args[0])?;
    let parts: Vec<LispExp<T>> = if args.len() == 2 {
        let sep = expect_string(&args[1])?;
        if sep.is_empty() {
            s.chars().map(|c| LispExp::string(c.to_string())).collect()
        } else {
            s.split(sep.as_str())
                .map(|p| LispExp::string(p.to_string()))
                .collect()
        }
    } else {
        s.split_whitespace()
            .map(|p| LispExp::string(p.to_string()))
            .collect()
    };
    Ok(LispExp::proper_list(parts))
}

const FORMAT_DOC: &str = "(format STRING &rest OBJECTS): Format OBJECTS according to \
                 the directives in STRING and return the result. Supports \
                 %s/%S (display), %d (integer), %f (float), and %% (literal \
                 percent). Signals an error if there are fewer OBJECTS than \
                 directives require.\n\n\
                 Example:\n\
                 (format \"%s is %d\" \"age\" 30) => \"age is 30\"\n\
                 (format \"100%%\")               => \"100%\"";

fn primitive_format<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    some_arguments(args)?;
    let fmt = expect_string(&args[0])?;
    let mut result = String::new();
    let mut arg_idx = 1;
    let mut chars = fmt.chars();

    let next_arg = |idx: &mut usize| -> Result<&LispExp<T>, EvalError<T>> {
        let val = args.get(*idx).ok_or(EvalError::WrongNumberOfArguments {
            expected: *idx + 1,
            got: args.len(),
        })?;
        *idx += 1;
        Ok(val)
    };

    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => result.push('%'),
                Some('s') | Some('S') => {
                    result.push_str(&lisp_display(next_arg(&mut arg_idx)?));
                }
                Some('d') => {
                    result.push_str(&format!(
                        "{}",
                        expect_number(next_arg(&mut arg_idx)?)? as i64
                    ));
                }
                Some('f') => {
                    result.push_str(&format!("{}", expect_number(next_arg(&mut arg_idx)?)?));
                }
                Some(other) => {
                    result.push('%');
                    result.push(other);
                }
                None => result.push('%'),
            }
        } else if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some(other) => {
                    return Err(EvalError::RuntimeMessage(format!(
                        "Wrong escape character \\{other}"
                    )));
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
            continue;
        }
    }
    Ok(LispExp::string(result))
}

/// Register this module's primitives: strings, and converting to and from them.
///
/// Called by [`super::setup_base_env`]. Here rather than there because a
/// primitive's name, its implementation and its docstring are one fact in three
/// pieces, and they were a thousand lines apart.
pub(super) fn install<T: LispContext>(into: &Registry<T>) {
    // ------------------------------- Strings ------------------------------
    into.function("make-string", primitive_make_string, MAKE_STRING_DOC);
    into.function("concat", primitive_concat, CONCAT_DOC);
    into.function("string=", primitive_string_eq, STRING_EQ_DOC);
    into.function("string<", primitive_string_lt, STRING_LT_DOC);
    into.function("substring", primitive_substring, SUBSTRING_DOC);
    into.function("upcase", primitive_upcase, UPCASE_DOC);
    into.function("downcase", primitive_downcase, DOWNCASE_DOC);
    into.function("format", primitive_format, FORMAT_DOC);
    into.function(
        "number-to-string",
        primitive_number_to_string,
        NUMBER_TO_STRING_DOC,
    );
    into.function(
        "string-to-number",
        primitive_string_to_number,
        STRING_TO_NUMBER_DOC,
    );
    into.function("symbol-name", primitive_symbol_name, SYMBOL_NAME_DOC);
    into.function("intern", primitive_intern, INTERN_DOC);
    into.function("split-string", primitive_split_string, SPLIT_STRING_DOC);
}
