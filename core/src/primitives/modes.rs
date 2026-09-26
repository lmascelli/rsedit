use super::*;
use crate::modes::{StringStyle, sexp};

pub const MAKE_MODE_DOC: &str = "(make-mode NAME): Register a new, empty major mode named NAME (a \
         symbol) with no keymaps, hooks, or syntax rules of its own. Returns \
         t. rsedit's own simplified alternative to Emacs's \
         `define-derived-mode`.\n\n\
         Example:\n\
         (make-mode 'lisp-mode)\n\
         (add-syntax-rule 'lisp-mode \"defun\" 'keyword)";

primitive!(make_mode, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        if let Some(ELispExp::Symbol(mode_name)) = args.get(0) {
            ctx.mode_registry
                .write()
                .expect("Failed to acquire write lock on mode_registry")
                .insert(mode_name.to_string(), MajorMode::new(mode_name));
            Ok(ELispExp::symbol("t".into()))
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: args.first().cloned().unwrap_or_else(ELispExp::nil),
            })
        }
    }
});

pub const ADD_HOOK_DOC: &str = "(add-hook MODE HOOK FUNCTION): Append the function named FUNCTION to \
         the list of functions run for HOOK (a string, e.g. \
         \"post-command-hook\") in major mode MODE. Returns t, or nil \
         (logging a diagnostic) if MODE names an unknown mode.\n\n\
         MODE may be nil, meaning every mode -- the same way nil means the global keymap in \
         `define-key'. Use that for a hook whose business has nothing to do with what kind of \
         buffer this is: the completion strip redraws after any command that changed what was \
         typed, and registering that in each mode separately would work until somebody defined \
         a mode afterwards.\n\n\
         A mode's own hooks run before the ones registered for every mode, so the specific \
         statement about this buffer acts before anything general reacts to the result.\n\n\
         Unlike real Emacs Lisp's `add-hook`, a hook here belongs to a mode (or to all of them) \
         rather than being a variable that can be let-bound.\n\n\
         Example:\n\
         (defun my-mode-hook () (log \"entered my-mode\"))\n\
         (add-hook 'my-mode \"post-command-hook\" 'my-mode-hook)\n\
         (add-hook nil \"post-command-hook\" 'something-everywhere)";

primitive!(add_hook, args, _env, ctx, {
    if args.len() != 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }

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
        let mut registry = ctx
            .mode_registry
            .write()
            .expect("Failed to acquire write lock on mode_registry");

        if let Some(mode) = registry.get_mut(mode_name.as_str()) {
            let hook_list = mode
                .hooks
                .entry(hook_name.to_string())
                .or_insert_with(Vec::new);
            hook_list.push(ELispExp::symbol(func_name.to_string()));

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

pub const ADD_SYNTAX_RULE_DOC: &str = "(add-syntax-rule MODE REGEX FACE &optional GROUP): Add a \
         syntax-highlighting rule to major mode MODE: text matching the regular expression REGEX \
         is drawn with FACE.\n\n\
         FACE is any symbol. Naming one that does not exist defines it, so a grammar can colour \
         whatever its language has; `set-face' then says what it looks like and `list-faces' \
         lists it.\n\n\
         GROUP, if given, is the capture group that takes the face rather than the whole match. \
         This is how a rule expresses context the regexp engine cannot look ahead for: to colour \
         a function *call*, match the name and the parenthesis together and face only the name.\n\n\
         A rule applies within one line. Something that can span lines is a region -- see \
         `add-syntax-region'.\n\n\
         Returns t, or nil (logging a diagnostic) if MODE is unknown or REGEX fails to \
         compile.\n\n\
         Example:\n\
         (add-syntax-rule 'rust-mode \"\\\\b(fn|let|mut)\\\\b\" 'keyword)\n\
         (add-syntax-rule 'rust-mode \"\\\\b([a-z_]\\\\w*)\\\\s*\\\\(\" 'function 1)";

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
    Ok(with_mode(ctx, mode_name.as_str(), |mode| {
        mode.grammar.rules.push(SyntaxRule {
            pattern,
            face,
            group,
        });
    }))
});

pub const ADD_SYNTAX_REGION_DOC: &str = "(add-syntax-region MODE BEGIN END FACE &optional ESCAPE \
         NESTABLE): Add a construct that runs from BEGIN to END -- possibly across lines -- and is \
         drawn with FACE. Block comments and strings are regions; a line comment is a rule.\n\n\
         ESCAPE, if given, is a regular expression for text that looks like END but is not, so \
         that a quote written with a backslash does not close a string. It is needed because the \
         regexp engine has no lookbehind, and so END cannot say \"not preceded by a backslash\" \
         itself.\n\n\
         NESTABLE, if non-nil, means BEGIN matching inside the region opens another one, so that \
         a nested block comment closes where it should rather than at the first end \
         delimiter.\n\n\
         Everything between the two is drawn with FACE, delimiters included, and no rule applies \
         inside: a string is uniformly a string.\n\n\
         Returns t, or nil (logging a diagnostic) if MODE is unknown or a regexp fails to \
         compile.\n\n\
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
    Ok(with_mode(ctx, mode_name.as_str(), |mode| {
        mode.grammar.regions.push(SyntaxRegion {
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

/// Apply EDIT to the named mode, answering t or nil the way the rest of the
/// mode primitives do.
fn with_mode<B: BufferTrait, F>(ctx: &EditorState<B>, mode_name: &str, edit: F) -> ELispExp<B>
where
    F: FnOnce(&mut crate::modes::MajorMode<B>),
{
    let mut registry = ctx
        .mode_registry
        .write()
        .expect("Failed to acquire write lock on mode_registry");
    match registry.get_mut(mode_name) {
        Some(mode) => {
            edit(mode);
            ELispExp::t()
        }
        None => ELispExp::nil(),
    }
}

pub const ADD_AUTO_MODE_DOC: &str = "(add-auto-mode PATTERN MODE): Open a file whose name matches \
         the regular expression PATTERN in major mode MODE.\n\n\
         PATTERN is matched against the whole path, so it can key on a directory as well as an \
         extension. Patterns are tried in the order they are declared and the first match wins, \
         which is what lets a specific rule be written before a general one.\n\n\
         This is how a language module comes to be used at all: nothing else maps a file to a \
         mode, so a grammar without one can only be reached by hand.\n\n\
         Returns t, or nil (logging a diagnostic) if PATTERN fails to compile.\n\n\
         Example:\n\
         (add-auto-mode \"\\\\.rs$\" 'rust-mode)\n\
         (add-auto-mode \"\\\\.lisp$\" 'lisp-mode)";

primitive!(add_auto_mode, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
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

fn string_arg<B: BufferTrait>(exp: &ELispExp<B>) -> Result<String, EvalError<EditorState<B>>> {
    match exp {
        ELispExp::String(text) | ELispExp::Symbol(text) => Ok(text.to_string()),
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.clone(),
        }),
    }
}

/// Change MODE's table, making one from the default if it has none yet.
fn with_syntax_table<B: BufferTrait, F>(
    ctx: &EditorState<B>,
    mode_name: &str,
    edit: F,
) -> ELispExp<B>
where
    F: FnOnce(&mut SyntaxTable),
{
    with_mode(ctx, mode_name, |mode| {
        edit(mode.syntax_table.get_or_insert_with(SyntaxTable::default));
    })
}

pub const SET_SYNTAX_PAIRS_DOC: &str = "(set-syntax-pairs MODE PAIRS): Declare the delimiters that \
         nest in major mode MODE. PAIRS is a string read two characters at a time: each opener \
         followed by the closer that matches it. Returns t, or nil (logging a diagnostic) if \
         MODE is unknown.\n\n\
         Openers and closers are declared together, and cannot be set one at a time by \
         `set-syntax-entry': a delimiter whose partner is unknown tells the scanner nothing.\n\n\
         A trailing odd character is ignored rather than refused -- it is a typo in a mode file, \
         and taking the whole table down over it would take the mode with it.\n\n\
         Example:\n\
         (set-syntax-pairs 'rust-mode \"()[]{}\")";

primitive!(set_syntax_pairs, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let mode = string_arg(&args[0])?;
    let pairs = string_arg(&args[1])?;
    let answer = with_syntax_table(ctx, &mode, |table| table.set_pairs(&pairs));
    if answer.is_nil() {
        ctx.log_diagnostic(&format!("Mode {mode} does not exist"));
    }
    Ok(answer)
});

pub const SET_STRING_SYNTAX_DOC: &str = "(set-string-syntax MODE STYLES): Declare the strings in \
         major mode MODE whose delimiters are more than one character. STYLES is a list of \
         (OPENER CLOSER). Returns t, or nil (logging a diagnostic) if MODE is unknown.\n\n\
         The `string' syntax class says \\\"this character opens a string and the same one ends \
         it\\\", which is true of the double quote and of nothing else. A Rust raw string opens \
         with three characters and closes with two; Python's triple quote opens with three and \
         closes with three, and its first character is already a string quote in its own \
         right.\n\n\
         The longest opener wins, so a raw-string opener beats the plain quote it begins with. \
         Nothing is escaped inside one, which is usually the point of having them.\n\n\
         This is about what the text *means* -- what `forward-sexp', the indenter and \
         `syntax-ppss' read. What it should look like is the grammar's business; a mode \
         declaring a raw string here will usually want `add-syntax-region' for it too.\n\n\
         Example:\n\
         (set-string-syntax 'rust-mode '((\\\"r#\\\\\\\"\\\" \\\"\\\\\\\"#\\\") (\\\"r\\\\\\\"\\\" \\\"\\\\\\\"\\\")))";

primitive!(set_string_syntax, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let mode = string_arg(&args[0])?;
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
        let opener = string_arg(opener)?;
        let closer = string_arg(closer)?;
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

pub const SET_CHAR_QUOTE_DOC: &str = "(set-char-quote MODE CHAR): Declare the character that \
         encloses a single literal character in major mode MODE -- `\'` in Rust, C, C++ and \
         Java. CHAR is a one-character string; nil removes it. Returns t, or nil (logging a \
         diagnostic) if MODE is unknown.\n\n\
         Not a syntax class, because `\'` is not one thing. In `\'a\'` it quotes a character, in \
         Rust\'s `\'static` it begins a lifetime, and in prose it is an apostrophe. A class says \
         what a character *is*; this depends on what comes after it, so the scanner decides by \
         looking: a quote that closes within one character -- two if the first is an escape -- \
         is a literal and is read as one atom, and anything else is ordinary punctuation.\n\n\
         Neither existing class can do the job. `string\' would make the first lifetime open a \
         string that swallows the rest of the file. `escape\' is honoured inside strings too, so \
         it would eat the closing quote of \\\"it\'s\\\".\n\n\
         What it fixes is everything that reads the table: `forward-sexp\', the indenter, \
         `syntax-ppss\' and the pairing built on them all stop being confused by `let c = \
         \'\\\"\';\'.\n\n\
         Example:\n\
         (set-char-quote \'rust-mode \"\'\")";

primitive!(set_char_quote, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let mode = string_arg(&args[0])?;
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

pub const SET_SYNTAX_ENTRY_DOC: &str = "(set-syntax-entry MODE CHARS CLASS): Give every character \
         in the string CHARS the syntax CLASS in major mode MODE. Returns t, or nil (logging a \
         diagnostic) if MODE is unknown.\n\n\
         CLASS is one of:\n\
         `string'       opens and closes a string; the same character ends it\n\
         `escape'       the next character is literal, whatever it is\n\
         `prefix'       attaches to the expression that follows it\n\
         `symbol'       part of a name\n\
         `punctuation'  separates, and belongs to nothing\n\n\
         Delimiters are not here; see `set-syntax-pairs'.\n\n\
         The same character means different things in different languages, which is the whole \
         reason a table is per mode: `'' is a prefix in Lisp and punctuation in Rust, where \
         making it a string quote would leave every lifetime looking like an unterminated \
         string.\n\n\
         Example:\n\
         (set-syntax-entry 'risp-mode \"'`,#\" 'prefix)\n\
         (set-syntax-entry 'rust-mode \"'\" 'punctuation)";

primitive!(set_syntax_entry, args, _env, ctx, {
    if args.len() != 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }
    let mode = string_arg(&args[0])?;
    let chars = string_arg(&args[1])?;
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

pub const SET_COMMENT_SYNTAX_DOC: &str = "(set-comment-syntax MODE STYLES): Declare how major mode \
         MODE writes comments. Replaces whatever was declared before. Returns t, or nil (logging \
         a diagnostic) if MODE is unknown.\n\n\
         STYLES is a list, one entry per way the language writes a comment:\n\
         (OPENER)                runs from OPENER to the end of the line\n\
         (OPENER CLOSER)         runs from OPENER to CLOSER, across lines if need be\n\
         (OPENER CLOSER t)       the same, and nests: /* /* */ */ is one comment\n\n\
         A list rather than one line-comment and one block-comment, because a language may have \
         two of either and the limit would be the editor's rather than the language's.\n\n\
         Example:\n\
         (set-comment-syntax 'risp-mode '((\";\")))\n\
         (set-comment-syntax 'rust-mode '((\"//\") (\"/*\" \"*/\" t)))";

primitive!(set_comment_syntax, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let mode = string_arg(&args[0])?;

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
        let opener = string_arg(opener)?;
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
                closer: string_arg(closer)?,
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

pub const MATCHING_DELIMITER_DOC: &str = "(matching-delimiter CHAR &optional MODE): The character \
         that closes CHAR if CHAR opens, or the one that opens it if CHAR closes, according to \
         major mode MODE -- the current buffer's mode when MODE is omitted. Returns nil if CHAR \
         is not a delimiter at all.\n\n\
         This is the same table `syntax-class' reads, asked for the partner rather than the \
         kind, and it is what auto-pairing is built on. A mode that declared its brackets with \
         `set-syntax-pairs' has therefore already said everything electric pairing needs to \
         know -- there is no second list of pairs to keep in step with the first, and a mode \
         whose brackets are unusual gets pairing that matches them for free.\n\n\
         A string quote is not a delimiter in this sense and answers nil: the same character \
         both opens and closes it, so `syntax-class' already says all there is to say. A \
         caller that pairs quotes tests for `string' and uses CHAR itself.\n\n\
         Example:\n\
         (matching-delimiter \"(\")            => \")\"\n\
         (matching-delimiter \"]\")            => \"[\"\n\
         (matching-delimiter \"x\")            => nil";

primitive!(matching_delimiter, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let text = string_arg(&args[0])?;
    let Some(c) = text.chars().next() else {
        return Err(EvalError::RuntimeMessage(
            "matching-delimiter wants a character, and was given an empty string".into(),
        ));
    };
    let mode = match args.get(1) {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(exp) => Some(string_arg(exp)?),
    };
    let mode = match mode {
        Some(mode) => mode,
        None => ctx.with_current_buffer(|buf| buf.current_mode.clone()),
    };

    let class = {
        let registry = ctx
            .mode_registry
            .read()
            .expect("Failed to acquire read lock on mode_registry");
        let table = registry
            .get(&mode)
            .and_then(|mode| mode.syntax_table.clone());
        table.unwrap_or_default().class_of(c)
    };
    Ok(match class {
        SyntaxClass::Open(closer) => ELispExp::string(closer.to_string()),
        SyntaxClass::Close(opener) => ELispExp::string(opener.to_string()),
        _ => ELispExp::nil(),
    })
});

pub const SYNTAX_CLASS_DOC: &str = "(syntax-class CHAR &optional MODE): What CHAR -- a \
         one-character string -- means in major mode MODE, or in the current buffer's mode when \
         MODE is omitted. One of `open', `close', `string', `escape', `prefix', `symbol' or \
         `punctuation'.\n\n\
         A mode that declared no table of its own answers from the default one, which pairs the \
         brackets and knows `\\\"' and `\\\\'. That is why sexp motion works in a buffer whose \
         mode never mentioned syntax.\n\n\
         Example:\n\
         (syntax-class \"(\" 'rust-mode) => open";

primitive!(syntax_class, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let text = string_arg(&args[0])?;
    let Some(c) = text.chars().next() else {
        return Err(EvalError::RuntimeMessage(
            "syntax-class wants a character, and was given an empty string".into(),
        ));
    };
    let mode = match args.get(1) {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(exp) => Some(string_arg(exp)?),
    };
    let mode = match mode {
        Some(mode) => mode,
        None => ctx.with_current_buffer(|buf| buf.current_mode.clone()),
    };

    let class = {
        let registry = ctx
            .mode_registry
            .read()
            .expect("Failed to acquire read lock on mode_registry");
        let table = registry
            .get(&mode)
            .and_then(|mode| mode.syntax_table.clone());
        table.unwrap_or_default().class_of(c)
    };
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

pub const SYNTAX_PPSS_DOC: &str = "(syntax-ppss &optional POS): What point -- or POS, if given -- \
         is inside, as a list of four:\n\n\
         0  how many lists deep it is; 0 at the top level\n\
         1  where the innermost open list begins, or nil at the top level\n\
         2  where the string it is inside begins, or nil if it is not in one\n\
         3  where the comment it is inside begins, or nil if it is not in one\n\
         4  where the innermost list's delimiter is, as opposed to where its expression \
         begins -- they differ by any prefix, and indentation lines up against the delimiter\n\n\
         Positional, so `(nth 3 (syntax-ppss))' asks whether point is in a comment. Emacs \
         returns eleven elements; these four are the ones this editor knows.\n\n\
         A string does not change the depth: point inside a string in a list is still one list \
         deep.\n\n\
         Example:\n\
         (if (nth 2 (syntax-ppss)) (message \\\"in a string\\\"))";

primitive!(syntax_ppss, args, _env, ctx, {
    let table = ctx.current_syntax_table();
    let found = ctx.with_current_buffer(|buf| {
        let pos = match args.first() {
            Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => {
                (*n as usize).min(buf.text.len())
            }
            _ => buf.text.cursor_pos_1d(),
        };
        sexp::context_at(&buf.text, &table, buf.scan_resume(pos), pos)
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

pub const BALANCE_POINT_DOC: &str = "(balance-point &optional POS): The first position at or \
         after POS -- or point -- by which every list open there has been closed. nil when the \
         buffer runs out with something still open.\n\n\
         POS itself when nothing is open there, since nothing then has to close.\n\n\
         Not the same question as whether the buffer balances. A buffer balances when the depth \
         is zero at the *end*; this asks whether it reaches zero at all, and stops as soon as it \
         does. The two come apart whenever something further down is still being typed: in a \
         file whose last function is half-written, every brace above it is closed and the buffer \
         as a whole is not.\n\n\
         Cheaper for the same reason. It stops at the first point of balance instead of running \
         to the end of the file, which from inside a function is that function's own closing \
         brace.\n\n\
         Example:\n\
         (if (balance-point) (message \\\"closed already\\\"))";

primitive!(balance_point, args, _env, ctx, {
    let table = ctx.current_syntax_table();
    let found = ctx.with_current_buffer(|buf| {
        let pos = match args.first() {
            Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => {
                (*n as usize).min(buf.text.len())
            }
            _ => buf.text.cursor_pos_1d(),
        };
        sexp::balance_point(&buf.text, &table, buf.scan_resume(pos), pos)
    });
    Ok(match found {
        Some(at) => ELispExp::number(at as f64),
        None => ELispExp::nil(),
    })
});

pub const BOUNDS_OF_ENCLOSING_LIST_DOC: &str = "(bounds-of-enclosing-list &optional POS): The \
         positions (START END) of the innermost list point -- or POS -- is inside, or nil at the \
         top level.\n\n\
         START is the opening delimiter, or the prefix before it; END is one past the closing \
         one, so the two bound the whole expression. A list left unclosed ends at the end of the \
         buffer, which is what a file being typed into looks like.\n\n\
         Example:\n\
         (bounds-of-enclosing-list) => (12 48)";
primitive!(bounds_of_enclosing_list, args, _env, ctx, {
    let table = ctx.current_syntax_table();
    let found = ctx.with_current_buffer(|buf| {
        let pos = match args.first() {
            Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => {
                (*n as usize).min(buf.text.len())
            }
            _ => buf.text.cursor_pos_1d(),
        };
        sexp::enclosing(&buf.text, &table, buf.scan_resume(pos), pos)
    });
    Ok(match found {
        Some(found) => ELispExp::proper_list(vec![
            ELispExp::number(found.start as f64),
            ELispExp::number(found.end as f64),
        ]),
        None => ELispExp::nil(),
    })
});
