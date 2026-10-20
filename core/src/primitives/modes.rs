use super::*;
use crate::modes::{StringStyle, sexp};
use crate::primitives::args;

pub const REMOVE_HOOK_DOC: &str = "(remove-hook MODE HOOK FUNCTION): Take FUNCTION off HOOK in \
         major mode MODE, or off the hook for every mode when MODE is nil: the arguments \
         `add-hook' took. Return t, or nil when there is no such mode.";

primitive!(remove_hook, args, _env, ctx, {
    exact_arity(args, 3)?;
    let mode = crate::primitives::args::optional_name(args.first(), "Mode or nil")?;
    let (ELispExp::String(hook), ELispExp::Symbol(function)) = (&args[1], &args[2]) else {
        return Err(EvalError::WrongArgumentType {
            expected: "String, Symbol".into(),
            got: args[1].clone(),
        });
    };
    Ok(
        if ctx.modes_mut(|modes| {
            modes.remove_hook(
                mode.as_deref(),
                hook,
                &ELispExp::symbol(function.to_string()),
            )
        }) {
            ELispExp::t()
        } else {
            ELispExp::nil()
        },
    )
});

pub const MAKE_MODE_DOC: &str = "(make-mode NAME): Make a major mode called NAME, a symbol, with \
         no keys, hooks, colouring or syntax of its own, and return t. A mode that exists already \
         is made again, empty.\n\n\
         Example:\n\
         (make-mode 'conf-mode)";

primitive!(make_mode, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        if let Some(ELispExp::Symbol(mode_name)) = args.get(0) {
            // Through `set_mode` rather than straight into the registry: a mode
            // made again has a new, empty grammar, and whatever was coloured
            // under the old one has to hear about it.
            ctx.set_mode(mode_name, MajorMode::new(mode_name));
            Ok(ELispExp::symbol("t".into()))
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: args.first().cloned().unwrap_or_else(ELispExp::nil),
            })
        }
    }
});

pub const ADD_HOOK_DOC: &str = "(add-hook MODE HOOK FUNCTION): Call FUNCTION, the name of a \
         function of no arguments, at HOOK -- a string, such as \"find-file-hook\" -- in the \
         buffers of major mode MODE, or in every buffer when MODE is nil. Return t, or nil when \
         there is no such mode. A function already there is not added again.\n\n\
         Example:\n\
         (add-hook 'rust-mode \"find-file-hook\" 'my-rust-setup)";

primitive!(add_hook, args, _env, ctx, {
    exact_arity(args, 3)?;

    // `nil' for the mode means every mode, the way it does in `define-key'.
    // Checked before the pattern below, because nil *is* a symbol here and
    // would otherwise be looked up as a mode named "nil" and reported missing.
    if args[0].is_nil() {
        let (ELispExp::String(hook_name), ELispExp::Symbol(func_name)) = (&args[1], &args[2])
        else {
            return Err(EvalError::WrongArgumentType {
                expected: "String, Symbol".into(),
                got: args[1].clone(),
            });
        };
        ctx.add_global_hook(hook_name, ELispExp::symbol(func_name.to_string()));
        return Ok(ELispExp::t());
    }

    if let (ELispExp::Symbol(mode_name), ELispExp::String(hook_name), ELispExp::Symbol(func_name)) =
        (&args[0], &args[1], &args[2])
    {
        // Reported outside the closure: the diagnostic writes the echo area,
        // which is a compartment of its own.
        let added = ctx.modes_mut(|modes| {
            modes.edit(mode_name.as_str(), |mode| {
                let hooks = mode.hooks.entry(hook_name.to_string()).or_default();
                let function = ELispExp::symbol(func_name.to_string());
                if !hooks.contains(&function) {
                    hooks.push(function);
                }
            })
        });
        if added {
            Ok(ELispExp::symbol("t".into()))
        } else {
            ctx.log_diagnostic(&format!("Mode {} does not exist", mode_name));
            Ok(ELispExp::nil())
        }
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "Symbol, String, Symbol".into(),
            got: ELispExp::string("other".into()),
        })
    }
});

pub const ADD_SYNTAX_RULE_DOC: &str = "(add-syntax-rule MODE REGEX FACE &optional GROUP): Colour \
         in FACE, in major mode MODE, what the regular expression REGEX matches within a line; \
         with GROUP, only that capture group of the match. Return t, or nil when there is no such \
         mode or REGEX is wrong, which the log says.\n\n\
         Example:\n\
         (add-syntax-rule 'rust-mode \"\\\\b(fn|let|mut)\\\\b\" 'keyword)";

primitive!(add_syntax_rule, args, _env, ctx, {
    if !(3..=4).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }
    let (ELispExp::Symbol(mode_name), ELispExp::String(regex_str)) = (&args[0], &args[1]) else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol, String, Symbol".into(),
            got: ELispExp::string(format!("{:?}", args)),
        });
    };
    let face = face_arg(&args[2])?;
    let group = match args.get(3) {
        None => 0,
        Some(exp) if exp.is_nil() => 0,
        Some(ELispExp::Number(n)) => n.max(0.0) as usize,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.clone(),
            });
        }
    };

    let Some(pattern) = compile(ctx, regex_str) else {
        return Ok(ELispExp::nil());
    };
    Ok(with_grammar(ctx, mode_name.as_str(), |grammar| {
        grammar.rules.push(SyntaxRule {
            pattern,
            face,
            group,
        });
    }))
});

pub const ADD_SYNTAX_REGION_DOC: &str = "(add-syntax-region MODE BEGIN END FACE &optional ESCAPE \
         NESTABLE): Colour in FACE, in major mode MODE, everything from a match of BEGIN to the \
         next match of END, across lines if need be, and nothing else inside. ESCAPE matches text \
         inside that looks like END and is not; NESTABLE non-nil means BEGIN inside opens \
         another. Return t, or nil when there is no such mode or a regular expression is wrong.\n\n\
         Example:\n\
         (add-syntax-region 'rust-mode \"/\\\\*\" \"\\\\*/\" 'comment nil t)";

primitive!(add_syntax_region, args, _env, ctx, {
    if !(4..=6).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 4,
            got: args.len(),
        });
    }
    let (ELispExp::Symbol(mode_name), ELispExp::String(begin_str), ELispExp::String(end_str)) =
        (&args[0], &args[1], &args[2])
    else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol, String, String, Symbol".into(),
            got: ELispExp::string(format!("{:?}", args)),
        });
    };
    let face = face_arg(&args[3])?;
    let escape = match args.get(4) {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(ELispExp::String(escape)) => match compile(ctx, escape) {
            Some(pattern) => Some(pattern),
            None => return Ok(ELispExp::nil()),
        },
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let nestable = args.get(5).is_some_and(|exp| exp.is_truthy());

    let (Some(begin), Some(end)) = (compile(ctx, begin_str), compile(ctx, end_str)) else {
        return Ok(ELispExp::nil());
    };
    Ok(with_grammar(ctx, mode_name.as_str(), |grammar| {
        grammar.regions.push(SyntaxRegion {
            begin,
            end,
            escape,
            face,
            nestable,
        });
    }))
});

/// Read a face argument, defining the face if nothing has named it yet.
fn face_arg<B: BufferTrait>(exp: &ELispExp<B>) -> Result<Face, EvalError<EditorState<B>>> {
    match exp {
        ELispExp::Symbol(name) | ELispExp::String(name) => Ok(Face::intern(name.as_str())),
        other => Err(EvalError::WrongArgumentType {
            expected: "Symbol or String naming a face".into(),
            got: other.clone(),
        }),
    }
}

/// Compile a pattern, reporting a bad one rather than signalling.
///
/// A grammar is configuration: one broken pattern in a language module should
/// cost that pattern, not the whole file it appears in.
fn compile<B: BufferTrait>(ctx: &EditorState<B>, source: &str) -> Option<regex::Regex> {
    match regex::Regex::new(source) {
        Ok(pattern) => Some(pattern),
        Err(why) => {
            ctx.log_diagnostic(&format!("Invalid Regex {source}: {why}"));
            None
        }
    }
}

/// Apply EDIT to the named mode's grammar, answering t or nil the way the rest
/// of the mode primitives do. Through `edit_grammar`, so that what was coloured
/// under the old grammar is coloured again under the new one.
fn with_grammar<B: BufferTrait, F>(ctx: &EditorState<B>, mode_name: &str, edit: F) -> ELispExp<B>
where
    F: FnOnce(&mut crate::modes::Grammar),
{
    match ctx.edit_grammar(mode_name, edit) {
        true => ELispExp::t(),
        false => ELispExp::nil(),
    }
}

pub const ADD_AUTO_MODE_DOC: &str = "(add-auto-mode PATTERN MODE): Open in major mode MODE a file \
         whose whole path the regular expression PATTERN matches. Of the patterns that match a \
         file, the one added first is used. Return t, or nil when PATTERN is wrong, which the log \
         says.\n\n\
         Example:\n\
         (add-auto-mode \"\\\\.ino$\" 'cpp-mode)";

primitive!(add_auto_mode, args, _env, ctx, {
    exact_arity(args, 2)?;
    let ELispExp::String(pattern) = &args[0] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[0].clone(),
        });
    };
    let mode = match &args[1] {
        ELispExp::Symbol(name) | ELispExp::String(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol or String naming a mode".into(),
                got: other.clone(),
            });
        }
    };
    let Some(pattern) = compile(ctx, pattern) else {
        return Ok(ELispExp::nil());
    };
    ctx.add_auto_mode(pattern, &mode);
    Ok(ELispExp::t())
});

// ---------------------------------------------------------------------------
// The syntax table
// ---------------------------------------------------------------------------
//
// What a mode tells the *scanner*, as opposed to what it tells the highlighter.
// The two are separate declarations of overlapping facts, and deliberately so:
// a grammar says what text should look like, a table says what it means. A
// mode may colour a doc comment differently from an ordinary one and they are
// still both comments, and the scanner has to work on a buffer whose grammar
// has never run -- the colouring is computed on another thread, and `C-M-f`
// cannot wait for it.
//
// A mode starts from `SyntaxTable::default`, which pairs the brackets and
// knows `"` and `\`, so a table only ever states its differences.

/// The classes a mode may name. `open` and `close` are deliberately absent:
/// a delimiter is useless without knowing what matches it, so pairs are
/// declared together by `set-syntax-pairs`.
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

/// Change MODE's table, making one from the default if it has none yet --
/// through `edit_syntax_table`, so that what was scanned under the old table
/// is scanned again under the new one.
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

pub const SET_STRING_SYNTAX_DOC: &str = "(set-string-syntax MODE STYLES): Declare the strings of \
         major mode MODE whose delimiters are longer than one character, as a list of (OPENER \
         CLOSER), in place of those declared before. Nothing is escaped inside one, and the \
         longest opener wins. Return t, or nil when there is no such mode.\n\n\
         Example:\n\
         (set-string-syntax 'rust-mode '((\"r#\\\"\" \"\\\"#\") (\"r\\\"\" \"\\\"\")))";

primitive!(set_string_syntax, args, _env, ctx, {
    exact_arity(args, 2)?;
    let mode = args::name(args.first())?;
    let entries: Vec<ELispExp<B>> = match &args[1] {
        ELispExp::Form(items) => items.to_vec(),
        other if other.is_nil() => Vec::new(),
        other => other.iter().collect(),
    };
    let mut styles = Vec::with_capacity(entries.len());
    for entry in &entries {
        let parts: Vec<ELispExp<B>> = match entry {
            ELispExp::Form(items) => items.to_vec(),
            other => other.iter().collect(),
        };
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
        styles.push(StringStyle { opener, closer });
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

pub const SET_COMMENT_SYNTAX_DOC: &str = "(set-comment-syntax MODE STYLES): Declare how major \
         mode MODE writes comments, in place of what was declared before. STYLES has an entry for \
         each way: (OPENER) to the end of the line, (OPENER CLOSER) as far as CLOSER, and (OPENER \
         CLOSER t) the same, nesting. Return t, or nil when there is no such mode.\n\n\
         Example:\n\
         (set-comment-syntax 'rust-mode '((\"//\") (\"/*\" \"*/\" t)))";

primitive!(set_comment_syntax, args, _env, ctx, {
    exact_arity(args, 2)?;
    let mode = args::name(args.first())?;

    let entries: Vec<ELispExp<B>> = match &args[1] {
        ELispExp::Form(items) => items.to_vec(),
        other if other.is_nil() => Vec::new(),
        other => other.iter().collect(),
    };
    let mut styles = Vec::with_capacity(entries.len());
    for entry in &entries {
        let parts: Vec<ELispExp<B>> = match entry {
            ELispExp::Form(items) => items.to_vec(),
            other => other.iter().collect(),
        };
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

/// Register this module's primitives: major and minor modes, keymaps, hooks and syntax tables.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("make-mode", make_mode, MAKE_MODE_DOC);
    into.function("add-hook", add_hook, ADD_HOOK_DOC);
    into.function("remove-hook", remove_hook, REMOVE_HOOK_DOC);
    into.function("add-syntax-rule", add_syntax_rule, ADD_SYNTAX_RULE_DOC);
    into.function(
        "add-syntax-region",
        add_syntax_region,
        ADD_SYNTAX_REGION_DOC,
    );
    into.function("add-auto-mode", add_auto_mode, ADD_AUTO_MODE_DOC);
    into.function("syntax-ppss", syntax_ppss, SYNTAX_PPSS_DOC);
    into.function("balance-point", balance_point, BALANCE_POINT_DOC);
    into.function(
        "bounds-of-enclosing-list",
        bounds_of_enclosing_list,
        BOUNDS_OF_ENCLOSING_LIST_DOC,
    );
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
}
