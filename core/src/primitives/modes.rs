use super::*;

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
         (logging a diagnostic) if MODE names an unknown mode. Unlike real \
         Emacs Lisp's `add-hook`, hooks here are scoped to a single major \
         mode rather than being global variables.\n\n\
         Example:\n\
         (defun my-mode-hook () (log \"entered my-mode\"))\n\
         (add-hook 'my-mode \"post-command-hook\" 'my-mode-hook)";

primitive!(add_hook, args, _env, ctx, {
    if args.len() != 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
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
