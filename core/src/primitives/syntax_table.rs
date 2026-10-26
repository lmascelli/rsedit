//! What a mode's text means: the syntax table the expression scanner reads,
//! which states how the mode differs from the default table, with its
//! brackets, `"` and `\`; and the questions the scanner answers with it.
//!
//! Design: `doc/architettura-editor.typ, <lessici>`.
use super::*;
use crate::modes::{StringStyle, sexp};
use crate::primitives::args;

/// Returns EXP as a class a mode may give a character. `open` and `close` are
/// not among them: a delimiter is nothing without its match, so pairs are
/// declared together, by `set-syntax-pairs`.
fn syntax_class_arg<B: BufferTrait>(
    exp: &ELispExp<B>,
) -> Result<SyntaxClass, EvalError<EditorState<B>>> {
    let (ELispExp::Symbol(name) | ELispExp::String(name)) = exp else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol naming a syntax class".into(),
            got: exp.clone(),
        });
    };
    Ok(match name.as_str() {
        "string" => SyntaxClass::StringQuote,
        "escape" => SyntaxClass::Escape,
        "prefix" => SyntaxClass::Prefix,
        "symbol" => SyntaxClass::Symbol,
        "punctuation" => SyntaxClass::Punctuation,
        other => {
            return Err(EvalError::RuntimeMessage(format!(
                "{other:?} is not a syntax class; expected string, escape, prefix, symbol or \
                 punctuation -- and see `set-syntax-pairs' for delimiters"
            )));
        }
    })
}

/// Applies EDIT to the table of MODE_NAME, made from the default when it has
/// none, answering t, or nil when there is no such mode. Through
/// `edit_syntax_table`, so that what was scanned under the old table is
/// scanned again under the new one.
fn with_syntax_table<B: BufferTrait, F>(
    ctx: &EditorState<B>,
    mode_name: &str,
    edit: F,
) -> ELispExp<B>
where
    F: FnOnce(&mut SyntaxTable),
{
    ELispExp::boolean(ctx.edit_syntax_table(mode_name, edit))
}

pub const SET_SYNTAX_PAIRS_DOC: &str = "(set-syntax-pairs MODE PAIRS): Make delimiters, in major \
         mode MODE, of the characters of the string PAIRS, read two at a time: an opener, then \
         the closer that matches it. They are added to the brackets every mode has. Return t, or \
         nil when there is no such mode.\n\n\
         Example:\n\
         (set-syntax-pairs 'conf-mode \"<>\")";

primitive!(set_syntax_pairs, args, _env, ctx, {
    exact_arity(args, 2)?;
    let mode = args::name(args.first())?;
    let pairs = args::name(args.get(1))?;
    let answer = with_syntax_table(ctx, &mode, |table| table.set_pairs(&pairs));
    if answer.is_nil() {
        ctx.log_diagnostic(&format!("Mode {mode} does not exist"));
    }
    Ok(answer)
});

pub const SET_SYNTAX_ENTRY_DOC: &str = "(set-syntax-entry MODE CHARS CLASS): Give each character \
         of the string CHARS the syntax CLASS in major mode MODE: `string', `escape', `prefix', \
         `symbol' or `punctuation'. Delimiters are made with `set-syntax-pairs'. Return t, or nil \
         when there is no such mode.\n\n\
         Example:\n\
         (set-syntax-entry 'conf-mode \"-.\" 'symbol)";

primitive!(set_syntax_entry, args, _env, ctx, {
    exact_arity(args, 3)?;
    let mode = args::name(args.first())?;
    let chars = args::name(args.get(1))?;
    let class = syntax_class_arg(&args[2])?;
    let answer = with_syntax_table(ctx, &mode, |table| {
        for c in chars.chars() {
            table.set_class(c, class);
        }
    });
    if answer.is_nil() {
        ctx.log_diagnostic(&format!("Mode {mode} does not exist"));
    }
    Ok(answer)
});

pub const SET_STRING_SYNTAX_DOC: &str = "(set-string-syntax MODE STYLES): Declare the strings of \
         major mode MODE whose delimiters are longer than one character, as a list of (OPENER \
         CLOSER &optional ESCAPED), in place of those declared before. Nothing is escaped \
         inside one, as in a raw string, unless ESCAPED is non-nil: then a backslash escapes \
         the character after it, as in Python's triple quotes and TOML's multiline basic \
         strings. The longest opener wins. \
         Return t, or nil when there is no such mode.\n\n\
         Example:\n\
         (set-string-syntax 'rust-mode '((\"r#\\\"\" \"\\\"#\") (\"r\\\"\" \"\\\"\")))";

primitive!(set_string_syntax, args, _env, ctx, {
    exact_arity(args, 2)?;
    let mode = args::name(args.first())?;
    let entries = args::list(&args[1]);
    let mut styles = Vec::with_capacity(entries.len());
    for entry in &entries {
        let parts = args::list(entry);
        let (Some(opener), Some(closer)) = (parts.first(), parts.get(1)) else {
            return Err(EvalError::RuntimeMessage(
                "a string style needs an opener and a closer".into(),
            ));
        };
        let opener = args::name(Some(opener))?;
        let closer = args::name(Some(closer))?;
        // An empty opener matches everywhere, which would make the whole
        // buffer a string the first time the scanner looked at it; an empty
        // closer would end it immediately and the style would do nothing.
        if opener.is_empty() || closer.is_empty() {
            return Err(EvalError::RuntimeMessage(
                "a string style's opener and closer cannot be empty".into(),
            ));
        }
        let escaped = parts.get(2).is_some_and(|part| part.is_truthy());
        styles.push(StringStyle {
            opener,
            closer,
            escaped,
        });
    }
    let answer = with_syntax_table(ctx, &mode, |table| table.set_strings(styles));
    if answer.is_nil() {
        ctx.log_diagnostic(&format!("Mode {mode} does not exist"));
    }
    Ok(answer)
});

pub const SET_CHAR_QUOTE_DOC: &str = "(set-char-quote MODE CHAR): Make CHAR, a one-character \
         string, the quote that encloses a single character in major mode MODE, as ' does in Rust \
         and C; nil takes it away. A quote that closes after one character, or after an escape \
         and one, makes a character literal, and any other is punctuation. Return t, or nil when \
         there is no such mode.\n\n\
         Example:\n\
         (set-char-quote 'rust-mode \"'\")";

primitive!(set_char_quote, args, _env, ctx, {
    exact_arity(args, 2)?;
    let mode = args::name(args.first())?;
    let quote = match &args[1] {
        exp if exp.is_nil() => None,
        ELispExp::String(text) => match text.chars().next() {
            Some(c) => Some(c),
            None => {
                return Err(EvalError::RuntimeMessage(
                    "set-char-quote wants a character, and was given an empty string".into(),
                ));
            }
        },
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let answer = with_syntax_table(ctx, &mode, |table| table.set_char_quote(quote));
    if answer.is_nil() {
        ctx.log_diagnostic(&format!("Mode {mode} does not exist"));
    }
    Ok(answer)
});

pub const SET_COMMENT_SYNTAX_DOC: &str = "(set-comment-syntax MODE STYLES): Declare how major \
         mode MODE writes comments, in place of what was declared before. STYLES has an entry for \
         each way: (OPENER) to the end of the line, (OPENER CLOSER) as far as CLOSER, and (OPENER \
         CLOSER t) the same, nesting. Return t, or nil when there is no such mode.\n\n\
         Example:\n\
         (set-comment-syntax 'rust-mode '((\"//\") (\"/*\" \"*/\" t)))";

primitive!(set_comment_syntax, args, _env, ctx, {
    exact_arity(args, 2)?;
    let mode = args::name(args.first())?;

    let entries = args::list(&args[1]);
    let mut styles = Vec::with_capacity(entries.len());
    for entry in &entries {
        let parts = args::list(entry);
        let Some(opener) = parts.first() else {
            return Err(EvalError::RuntimeMessage(
                "a comment style needs at least an opener".into(),
            ));
        };
        let opener = args::name(Some(opener))?;
        if opener.is_empty() {
            // An empty opener matches everywhere, which would make the whole
            // buffer a comment the first time the scanner looked at it.
            return Err(EvalError::RuntimeMessage(
                "a comment opener cannot be empty".into(),
            ));
        }
        styles.push(match parts.get(1) {
            None => CommentStyle::Line { opener },
            Some(closer) if closer.is_nil() => CommentStyle::Line { opener },
            Some(closer) => CommentStyle::Block {
                opener,
                closer: args::name(Some(closer))?,
                nestable: parts.get(2).is_some_and(|nest| !nest.is_nil()),
            },
        });
    }

    let answer = with_syntax_table(ctx, &mode, |table| table.set_comments(styles));
    if answer.is_nil() {
        ctx.log_diagnostic(&format!("Mode {mode} does not exist"));
    }
    Ok(answer)
});

pub const SYNTAX_CLASS_DOC: &str = "(syntax-class CHAR &optional MODE): Return what CHAR, a \
         one-character string, is in major mode MODE or in the current buffer's: `open', `close', \
         `string', `escape', `prefix', `symbol' or `punctuation'.\n\n\
         Example:\n\
         (syntax-class \"(\" 'rust-mode)";

primitive!(syntax_class, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let text = args::name(args.first())?;
    let Some(c) = text.chars().next() else {
        return Err(EvalError::RuntimeMessage(
            "syntax-class wants a character, and was given an empty string".into(),
        ));
    };
    let mode = match args.get(1) {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(exp) => Some(args::name(Some(exp))?),
    };
    let mode = match mode {
        Some(mode) => mode,
        None => ctx.with_current_buffer(|buf| buf.current_mode.clone()),
    };

    let class = ctx.modes(|modes| modes.syntax_table(&mode)).class_of(c);
    Ok(ELispExp::symbol(
        match class {
            SyntaxClass::Open(_) => "open",
            SyntaxClass::Close(_) => "close",
            SyntaxClass::StringQuote => "string",
            SyntaxClass::Escape => "escape",
            SyntaxClass::Prefix => "prefix",
            SyntaxClass::Symbol => "symbol",
            SyntaxClass::Punctuation => "punctuation",
        }
        .to_string(),
    ))
});

pub const MATCHING_DELIMITER_DOC: &str = "(matching-delimiter CHAR &optional MODE): Return the \
         delimiter that pairs with CHAR, a one-character string, in major mode MODE or in the \
         current buffer's: the closer of an opener, the opener of a closer. nil when CHAR is no \
         delimiter; a string quote is none, being its own pair.\n\n\
         Example:\n\
         (matching-delimiter \"(\")";

primitive!(matching_delimiter, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let text = args::name(args.first())?;
    let Some(c) = text.chars().next() else {
        return Err(EvalError::RuntimeMessage(
            "matching-delimiter wants a character, and was given an empty string".into(),
        ));
    };
    let mode = match args.get(1) {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(exp) => Some(args::name(Some(exp))?),
    };
    let mode = match mode {
        Some(mode) => mode,
        None => ctx.with_current_buffer(|buf| buf.current_mode.clone()),
    };

    let class = ctx.modes(|modes| modes.syntax_table(&mode)).class_of(c);
    Ok(match class {
        SyntaxClass::Open(closer) => ELispExp::string(closer.to_string()),
        SyntaxClass::Close(opener) => ELispExp::string(opener.to_string()),
        _ => ELispExp::nil(),
    })
});

pub const SYNTAX_PPSS_DOC: &str = "(syntax-ppss &optional POS): Return what POS, or point, is \
         inside, as a list of five: how many lists deep it is; where the innermost list begins, \
         its prefix included; where the string it is in begins; where the comment it is in \
         begins; and where the innermost list's opening delimiter is. Each is nil where there is \
         none, and the depth 0 at the top level.\n\n\
         Example:\n\
         (nth 3 (syntax-ppss))";

primitive!(syntax_ppss, args, _env, ctx, {
    let table = ctx.current_syntax_table();
    let found = ctx.with_current_buffer(|buf| {
        let pos = match args.first() {
            Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => {
                (*n as usize).min(buf.text.len())
            }
            _ => buf.text.cursor_pos_1d(),
        };
        sexp::context_at(&buf.text, &table, buf.scan_resume(pos, &table), pos)
    });
    let offset = |at: Option<usize>| {
        at.map(|at| ELispExp::number(at as f64))
            .unwrap_or_else(ELispExp::nil)
    };
    Ok(ELispExp::proper_list(vec![
        ELispExp::number(found.depth as f64),
        offset(found.innermost),
        offset(found.string_start),
        offset(found.comment_start),
        offset(found.innermost_delimiter),
    ]))
});

pub const BALANCE_POINT_DOC: &str = "(balance-point &optional POS): Return the first position, \
         from POS or point on, by which every list open there has closed: POS itself when none \
         is, nil when the buffer ends first. It stops there, without reading the rest of the \
         buffer.\n\n\
         Example:\n\
         (balance-point)";

primitive!(balance_point, args, _env, ctx, {
    let table = ctx.current_syntax_table();
    let found = ctx.with_current_buffer(|buf| {
        let pos = match args.first() {
            Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => {
                (*n as usize).min(buf.text.len())
            }
            _ => buf.text.cursor_pos_1d(),
        };
        sexp::balance_point(&buf.text, &table, buf.scan_resume(pos, &table), pos)
    });
    Ok(match found {
        Some(at) => ELispExp::number(at as f64),
        None => ELispExp::nil(),
    })
});

pub const BOUNDS_OF_ENCLOSING_LIST_DOC: &str = "(bounds-of-enclosing-list &optional POS): Return \
         (START END) of the innermost list that POS, or point, is inside, or nil at the top \
         level. START is at the list's prefix, when it has one, and END just past its closer; a \
         list not yet closed ends at the end of the buffer.\n\n\
         Example:\n\
         (bounds-of-enclosing-list)";

primitive!(bounds_of_enclosing_list, args, _env, ctx, {
    let table = ctx.current_syntax_table();
    let found = ctx.with_current_buffer(|buf| {
        let pos = match args.first() {
            Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => {
                (*n as usize).min(buf.text.len())
            }
            _ => buf.text.cursor_pos_1d(),
        };
        sexp::enclosing(&buf.text, &table, buf.scan_resume(pos, &table), pos)
    });
    Ok(match found {
        Some(found) => ELispExp::proper_list(vec![
            ELispExp::number(found.start as f64),
            ELispExp::number(found.end as f64),
        ]),
        None => ELispExp::nil(),
    })
});

/// Registers the primitives of syntax tables.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("set-syntax-pairs", set_syntax_pairs, SET_SYNTAX_PAIRS_DOC);
    into.function(
        "set-string-syntax",
        set_string_syntax,
        SET_STRING_SYNTAX_DOC,
    );
    into.function("set-char-quote", set_char_quote, SET_CHAR_QUOTE_DOC);
    into.function("set-syntax-entry", set_syntax_entry, SET_SYNTAX_ENTRY_DOC);
    into.function(
        "set-comment-syntax",
        set_comment_syntax,
        SET_COMMENT_SYNTAX_DOC,
    );
    into.function("syntax-class", syntax_class, SYNTAX_CLASS_DOC);
    into.function(
        "matching-delimiter",
        matching_delimiter,
        MATCHING_DELIMITER_DOC,
    );
    into.function("syntax-ppss", syntax_ppss, SYNTAX_PPSS_DOC);
    into.function("balance-point", balance_point, BALANCE_POINT_DOC);
    into.function(
        "bounds-of-enclosing-list",
        bounds_of_enclosing_list,
        BOUNDS_OF_ENCLOSING_LIST_DOC,
    );
}
