use super::*;

pub const QUIT_DOC: &str = "(quit): Stop the editor's main loop.\n\n\
         Example:\n\
         (define-key nil \"C-x C-c\" 'quit)";

primitive!(quit, _args, _env, ctx, {
    ctx.quit();
    Ok(ELispExp::nil())
});

pub const EVAL_FILE_DOC: &str = "(eval-file FILE): Evaluate FILE as Lisp. FILE is first looked up as \
         an absolute or relative path; if that fails and FILE has no \
         directory separator or extension, each directory in `lisp-path` is \
         tried in order for FILE.lisp. Returns a one-element list holding the \
         result of evaluating the file's contents (wrapped in an implicit \
         `progn`), or nil (logging a diagnostic) if no matching file was \
         found.\n\n\
         Example:\n\
         (eval-file \"common-keymaps\") ; loads <lisp-path>/common-keymaps.lisp\n\
         (eval-file \"/home/me/.config/rsedit/extra.lisp\")";

primitive!(eval_file, args, env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else if let Some(ELispExp::String(file)) = args.first() {
        Ok(ctx.eval_file(file, env.clone())?)
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        })
    }
});

pub const DEFINE_KEY_DOC: &str = "(define-key MODE KEY COMMAND): Bind KEY (an Emacs-style key sequence \
         string, e.g. \"C-x\" or \"<ret>\") to COMMAND. MODE is either nil, \
         binding KEY globally, or a mode name symbol/string, binding KEY only \
         in that major mode. COMMAND may be a symbol naming a function (which \
         is wrapped so it is called with no arguments) or an arbitrary \
         expression to evaluate. Returns t on success, nil (logging a \
         diagnostic) if MODE names an unknown mode or KEY isn't a recognized \
         key sequence.\n\n\
         Example:\n\
         (define-key nil \"C-n\" 'next-line)          ; global binding\n\
         (define-key 'lisp-mode \"<ret>\" 'insert-newline) ; mode-local binding\n\
         (define-key nil \"C-x C-s\" '(save-buffer))  ; arbitrary expression";

primitive!(define_key, args, env, ctx, {
    if args.len() != 3 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        })
    } else {
        if let (mode, ELispExp::String(key_str), ast) = (&args[0], &args[1], &args[2]) {
            let mode_name: Option<String> = match mode {
                ELispExp::Symbol(mode_name) | ELispExp::String(mode_name) => {
                    if mode_name.as_str() == "nil" {
                        None
                    } else {
                        Some(mode_name.to_string())
                    }
                }
                _ => {
                    return Err(EvalError::WrongArgumentType {
                        expected: "Symbol, String, Symbol".into(),
                        got: ELispExp::string(format!(
                            "{:?}, {:?}, {:?}",
                            &args[0], &args[1], &args[2]
                        )),
                    });
                }
            };
            let actual_ast = command_ast(ast, &env);
            if let Some(keys) = parse_key_sequence(key_str) {
                if let Some(mode_name) = mode_name {
                    let mut mode_registry_lock = ctx
                        .mode_registry
                        .write()
                        .expect("Failed to acquire write lock on mode_registry");
                    if let Some(mode) = mode_registry_lock.get_mut(&mode_name) {
                        mode.keymaps.insert(keys, actual_ast);
                        Ok(ELispExp::t())
                    } else {
                        ctx.log_diagnostic(&format!(
                            "[ERROR]: define-key no mode named {mode_name}"
                        ));
                        Ok(ELispExp::nil())
                    }
                } else {
                    let mut keymaps = ctx
                        .keymaps
                        .write()
                        .expect("Failed to acquire write lock on keymaps");
                    keymaps.insert(keys, actual_ast);
                    Ok(ELispExp::t())
                }
            } else {
                ctx.log_diagnostic(&format!("Invalid key sequence: {}", key_str));
                Ok(ELispExp::nil())
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String, Symbol".into(),
                got: ELispExp::string(format!("{:?}, {:?}", &args[0], &args[1])),
            })
        }
    }
});

pub const LOG_DOC: &str = "(log MESSAGE): Append the string MESSAGE to the editor's diagnostic \
         log (and to the log file, if one is enabled).\n\n\
         Example:\n\
         (log \"buffer saved\")";

primitive!(log, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        if let ELispExp::String(message) = &args[0] {
            ctx.log_diagnostic(&message);
            Ok(ELispExp::nil())
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: args.first().cloned().unwrap_or_else(ELispExp::nil),
            })
        }
    }
});

pub const ALL_LOGS_DOC: &str = "(all-logs): Return every diagnostic message logged so far -- via \
         `log', `message', or the editor's own internal error reporting \
         -- oldest first, as a list of strings.\n\n\
         Example:\n\
         (length (all-logs))";

primitive!(all_logs, _args, _env, ctx, {
    Ok(ELispExp::proper_list(
        ctx.get_logs().into_iter().map(ELispExp::string).collect(),
    ))
});

pub const BACKTRACE_DOC: &str = "(backtrace): Return the call stack captured at the point of the \
         most recent uncaught error, as a list of function-name strings, \
         innermost (deepest) call first -- or an empty list if nothing has \
         errored since the backtrace was last reported (every top-level \
         error handler clears it once it's done reporting). Calls in tail \
         position are not shown as separate frames, since by the time they \
         run their caller's frame has already been popped -- see the \
         `push_call_frame` docs on the `LispContext` trait for why.\n\n\
         Example:\n\
         (backtrace) => (\"eval-string\" \"eval-expression-confirm\" \"funcall\" \"minibuffer-confirm\")";

primitive!(backtrace, _args, _env, ctx, {
    // `backtrace` is itself a call, so by the time this runs, the generic
    // per-call instrumentation has already pushed a frame for it -- drop
    // that one frame so the result reflects the stack as it stood before
    // this call, not this call's own (otherwise self-referential) frame.
    let mut frames = ctx.backtrace();
    if !frames.is_empty() {
        frames.remove(0);
    }
    Ok(ELispExp::proper_list(
        frames.into_iter().map(ELispExp::string).collect(),
    ))
});

pub const SET_ECHO_MESSAGE_DOC: &str = "(set-echo-message MESSAGE): Show MESSAGE in the editor's \
         echo area (the status line at the bottom of the frame). Prefer \
         `message', which also keeps MESSAGE in the log, unless you \
         specifically don't want that.\n\n\
         MESSAGE stays on screen for `echo-message-timeout' seconds and is \
         then no longer drawn; set that variable to nil to leave messages up \
         until something replaces them. Each new message starts its own \
         timeout.\n\n\
         Example:\n\
         (set-echo-message \"Saved\")\n\
         (setq echo-message-timeout nil) ; messages no longer time out";

primitive!(set_echo_message, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else if let ELispExp::String(message) = &args[0] {
        ctx.set_echo_message(message);
        Ok(ELispExp::nil())
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        })
    }
});

pub const REPEAT_DOC: &str = "(repeat): Run the last command again.\n\n\
         Emacs' `C-x z'. With `repeat' given a repeat key of its own, `C-x z z z' runs it three \
         times more -- which is what makes it worth having over pressing the original key again: \
         the original may be a three-key sequence, and `C-x u C-x u C-x u' to undo three times is \
         the complaint this answers.\n\n\
         Works for every command, including ones defined later, because it re-runs the form the \
         last command ran rather than consulting a list of which commands may be repeated. A \
         command that prompts will prompt again: what is remembered is the command, not the \
         answers that were given to it.\n\n\
         Repeating `repeat' is not a thing -- it never becomes the last command, so the second \
         press repeats what the first one did.\n\n\
         Returns whatever the repeated command returned, or nil (reporting it) when nothing has \
         run yet.\n\n\
         Example:\n\
         (define-key nil \"C-x z\" 'repeat)\n\
         (define-repeat-key 'repeat \"z\")";

primitive!(repeat, _args, env, ctx, {
    let Some(form) = ctx.last_command_form() else {
        ctx.set_echo_message("No command to repeat");
        return Ok(ELispExp::nil());
    };
    // Evaluated here rather than pushed back through the key handler: this is
    // already running inside a command, so it inherits that command's undo
    // group and hooks. Re-entering the dispatcher would open a second one
    // inside the first.
    crate::lisp::eval(&form, env.clone(), ctx)
});

pub const DEFINE_REPEAT_KEY_DOC: &str = "(define-repeat-key COMMAND KEY): Say that after COMMAND \
         runs, pressing KEY on its own runs it again -- and offers the same again after that, \
         until some other key is pressed.\n\n\
         KEY is one key, written as a binding is: \"o\", \"C-o\". The offer is shown in the frame \
         while it stands, and the key that ends it is *not* swallowed: it does whatever it \
         ordinarily does, so ignoring the offer costs nothing.\n\n\
         Declared rather than inferred: a rule like \"the last key of the sequence repeats\" would \
         make `C-x C-f' followed by `f' re-open `find-file'.\n\n\
         Example:\n\
         (define-repeat-key 'other-window \"o\")   ; C-x o o o cycles windows";

primitive!(define_repeat_key, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let command = match &args[0] {
        ELispExp::Symbol(name) | ELispExp::String(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol or String".into(),
                got: other.clone(),
            });
        }
    };
    let ELispExp::String(key) = &args[1] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[1].clone(),
        });
    };
    // One key, not a sequence: a repeat key that took two presses would not be
    // saving anybody anything, and the map would have to stay up between them.
    let parsed = super::parse_key_sequence(key).filter(|keys| keys.len() == 1);
    let Some(keys) = parsed else {
        return Err(EvalError::RuntimeMessage(format!(
            "{key:?} is not a single key, so it cannot repeat a command"
        )));
    };
    ctx.set_repeat_key(&command, keys[0].clone());
    Ok(ELispExp::t())
});

/// What a binding's COMMAND argument means, as an expression to evaluate.
///
/// A symbol naming a function is a *call* -- `(define-key nil "C-n"
/// 'next-line)` binds the command, not the symbol -- while anything else is
/// taken as an expression and evaluated as written, which is what lets a
/// binding be `'(insert "x")`.
///
/// One function because `define-key` and `set-transient-keymap` are two ways
/// of writing the same thing down, and a user who learns the rule from one has
/// every right to expect the other to follow it. They answered it separately
/// once, and separately means eventually differently.
///
/// # It changes nothing today
///
/// The key dispatcher accepts a bare symbol and a one-element form alike --
/// see the comment on `bound_command` in `handle_key_event`, where that is
/// spelt out -- so wrapping or not wrapping makes no difference to what
/// happens when the key is pressed, and no test can tell the two apart. What
/// it buys is that a keymap holds *one* shape rather than two, which is what
/// anything reading a keymap back rather than running it -- describing a
/// binding, finding where a command is bound -- would otherwise have to know
/// about.
fn command_ast<B: BufferTrait>(
    exp: &ELispExp<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
) -> ELispExp<B> {
    match exp {
        // Only when it names one: a symbol that is not a function is a
        // variable reference, and wrapping it would call something that does
        // not exist rather than reading what does.
        ELispExp::Symbol(name) if env.get_function(name).is_some() => {
            ELispExp::form(vec![exp.clone()])
        }
        other => other.clone(),
    }
}

pub const SET_TRANSIENT_KEYMAP_DOC: &str = "(set-transient-keymap BINDINGS &optional MESSAGE \
         PASS-THROUGH): Install a keymap that is consulted before every other, until one of its \
         own bindings takes it down with `clear-transient-keymap'.\n\n\
         With PASS-THROUGH nil -- the default -- the map **swallows every key it does not \
         bind**. Non-nil, an unbound key is handed on to the keymaps underneath and the map \
         stays up.\n\n\
         BINDINGS is a list of (KEY . COMMAND) pairs, where KEY is written as a binding is -- \
         \"n\", \"<ret>\", \"C-g\" -- and COMMAND is a symbol naming a function or an expression \
         to evaluate. MESSAGE, if given, is shown in the frame while the map stands, so that the \
         user can see what is being asked of them.\n\n\
         This is for a map that is a *question*: a list of completions to pick from, a \
         confirmation to answer. Swallowing the unbound keys is the point -- a half-finished \
         choice must not be walked away from by pressing something unrelated, because nothing \
         would then say it was still standing. Which is also why every such map must bind a way \
         out, `C-g' and Escape included: nothing else can take it down.\n\n\
         PASS-THROUGH is for the third kind: a map that is a *filter*. The completion strip \
         binds the keys that move and choose, and lets ordinary typing through to the buffer so \
         that it narrows the list -- which needs the key to arrive *and* the map to survive. A \
         swallowing map could not be typed at and a dismissing one would vanish at the first \
         letter, so neither of the other answers can be bent into it. Such a map still has to \
         bind a way out, because nothing takes it down on its own.\n\n\
         For the other kind of transient map -- an *offer*, like the `o' that keeps cycling \
         windows after `C-x o' -- see `define-repeat-key', which dismisses itself the moment \
         something else is pressed.\n\n\
         Example:\n\
         (set-transient-keymap (list (cons \"n\" 'next-candidate)\n\
                                     (cons \"p\" 'previous-candidate)\n\
                                     (cons \"<ret>\" 'choose-candidate)\n\
                                     (cons \"<esc>\" 'abandon-candidates)\n\
                                     (cons \"C-g\" 'abandon-candidates))\n\
                               \"[n/p to move, RET to choose]\")";

primitive!(set_transient_keymap, args, env, ctx, {
    if args.is_empty() || args.len() > 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let mut keymap = Keymap::new();
    let mut bound = 0;
    for binding in args[0].iter() {
        let (key, command) = match &binding {
            ELispExp::Cons(cell) => (cell.car.clone(), cell.cdr.clone()),
            other => {
                return Err(EvalError::WrongArgumentType {
                    expected: "a (KEY . COMMAND) pair".into(),
                    got: other.clone(),
                });
            }
        };
        let ELispExp::String(key) = &key else {
            return Err(EvalError::WrongArgumentType {
                expected: "String naming a key".into(),
                got: key.clone(),
            });
        };
        // A key that does not parse fails the whole call rather than being
        // dropped: a modal map with a missing binding is a map the user cannot
        // get out of, and finding that out by pressing Escape and having
        // nothing happen is the worst possible time.
        let Some(keys) = super::parse_key_sequence(key) else {
            return Err(EvalError::RuntimeMessage(format!(
                "{key:?} is not a key sequence"
            )));
        };
        // Through the same rule `define-key` uses, so that a binding written
        // one way means what it means written the other.
        keymap.insert(keys, command_ast(&command, &env));
        bound += 1;
    }
    if bound == 0 {
        return Err(EvalError::RuntimeMessage(
            "A transient keymap with no bindings could never be dismissed".into(),
        ));
    }
    let message = match args.get(1) {
        Some(ELispExp::String(message)) => message.to_string(),
        _ => String::new(),
    };
    let on_unbound = match args.get(2) {
        Some(flag) if flag.is_truthy() => OnUnbound::Pass,
        _ => OnUnbound::Refuse,
    };
    ctx.set_transient_keymap(TransientKeymap {
        keymap,
        on_unbound,
        message,
    });
    Ok(ELispExp::t())
});

pub const CLEAR_TRANSIENT_KEYMAP_DOC: &str = "(clear-transient-keymap): Take down the keymap \
         installed by `set-transient-keymap', so that keys reach the buffer's own bindings \
         again. Returns t.\n\n\
         Every command bound in such a map that ends the question -- choosing, cancelling -- \
         has to call this. The map does not dismiss itself when one of its own bindings runs, \
         which is what lets a map offer several keys that do not end anything.\n\n\
         Example:\n\
         (defun abandon-candidates () (completion--close) (clear-transient-keymap))";

primitive!(clear_transient_keymap, _args, _env, ctx, {
    ctx.clear_transient_keymap();
    Ok(ELispExp::t())
});

// ---------------------------------------------------------------------------
// Asking the interpreter what it knows
// ---------------------------------------------------------------------------
//
// A completion source for this editor's own Lisp has a problem no other source
// has: the list of names worth offering is not a property of the language, it
// is a property of *this session*. Half of them are primitives compiled into
// the editor, the other half are whatever the user's modules defined, and both
// change as the editor grows. Writing that list into a `.lisp` file means
// writing down a snapshot that is wrong by the next commit.
//
// So `capf-symbols` does not write it down. It asks, every time it is called,
// and a name is offered exactly when calling it would work.

pub const ALL_FUNCTIONS_DOC: &str = "(all-functions): Return the names of every function visible \
         from the current environment, as a sorted list of strings -- primitives and Lisp \
         definitions alike, since nothing calling them can tell the difference.\n\n\
         A name bound in an inner scope appears once, not twice: this is the set of names that \
         would resolve, not the set of bindings that exist.\n\n\
         Macros are *not* included; see `all-macros'. Keeping them apart is what lets a caller \
         treat a macro as syntax and a function as a call.\n\n\
         Example:\n\
         (all-functions) => (\"1+\" \"abs\" \"append\" ...)";

primitive!(all_functions, _args, env, _ctx, {
    Ok(ELispExp::proper_list(
        env.function_names()
            .into_iter()
            .map(ELispExp::string)
            .collect(),
    ))
});

pub const ALL_MACROS_DOC: &str = "(all-macros): Return the names of every macro visible from the \
         current environment, as a sorted list of strings.\n\n\
         Special forms are not macros and are not listed: `if' and `let' are built into the \
         evaluator and have no binding to enumerate.\n\n\
         Example:\n\
         (all-macros) => (\"defcommand\")";

primitive!(all_macros, _args, env, _ctx, {
    Ok(ELispExp::proper_list(
        env.macro_names()
            .into_iter()
            .map(ELispExp::string)
            .collect(),
    ))
});

pub const ALL_VARIABLES_DOC: &str = "(all-variables): Return the names of every variable visible \
         from the current environment, as a sorted list of strings.\n\n\
         Called inside a `let', its bindings are in the list too -- they are variables, and they \
         would resolve. What comes back describes the environment it was asked in.\n\n\
         Example:\n\
         (all-variables) => (\"case-fold-search\" \"frame-width\" ...)";

primitive!(all_variables, _args, env, _ctx, {
    Ok(ELispExp::proper_list(
        env.variable_names()
            .into_iter()
            .map(ELispExp::string)
            .collect(),
    ))
});

pub const REGEXP_OPT_DOC: &str = "(regexp-opt WORDS): Return a regular expression matching any one \
         of WORDS, with every character that means something to the regexp engine escaped. The \
         group is non-capturing, so a rule wrapping it can still use group 1 for its own.\n\n\
         In Rust rather than in a module because which characters are special is a property of \
         the engine, not of the language being matched. A module escaping the ones it could \
         think of is writing down a guess -- and this editor's own names are full of them: \
         unquoted, `1+' is a pattern matching `11', `111' and every other run of ones, and \
         `*items*' does not compile at all.\n\n\
         What it is for is letting a mode keep **one** list of its keywords and get both its \
         colouring and its completion out of it, rather than two copies that drift.\n\n\
         Example:\n\
         (regexp-opt '(\\\"fn\\\" \\\"let\\\")) => \\\"(?:fn|let)\\\"";

primitive!(regexp_opt, args, _env, _ctx, {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let words: Vec<ELispExp<B>> = match &args[0] {
        ELispExp::Form(items) => items.to_vec(),
        other if other.is_nil() => Vec::new(),
        other => other.iter().collect(),
    };
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
    // An alternation of nothing matches the empty string at every position,
    // which as a syntax rule would face the whole buffer one character at a
    // time. A pattern that matches nothing at all is the honest answer.
    //
    // Spelt as a character class that excludes every character, because the
    // obvious `(?!)` is a look-ahead and this engine has none -- the same
    // constraint that shapes `crate::modes::syntax`.
    if parts.is_empty() {
        return Ok(ELispExp::string("[^\\s\\S]".into()));
    }
    Ok(ELispExp::string(format!("(?:{})", parts.join("|"))))
});

pub const STRING_MATCH_DOC: &str = "(string-match REGEXP STRING): Match REGEXP against STRING and \
         return what it captured, or nil if it does not match.\n\n\
         The answer is a list: the whole match first, then one element per capture group, in \
         order. A group that did not take part is nil, so the positions of the others do not \
         shift -- which is what lets a caller say \"group 3 is the column\" and be right whether \
         or not group 2 was there.\n\n\
         Stateless, unlike Emacs' `string-match', which records the match in global data that a \
         later `match-string' reads. Global match data is a variable that any function you call \
         in between can overwrite, and the bug it causes -- the right groups, from the wrong \
         match -- is invisible at the point it is read.\n\n\
         Returns nil (logging a diagnostic) if REGEXP does not compile. The engine is the one \
         the syntax rules use: no lookahead, no backreferences, which is how it stays linear.\n\n\
         Example:\n\
         (string-match \\\"([a-z]+):([0-9]+)\\\" \\\"src/main.rs:42\\\")\n\
         => (\\\"main.rs:42\\\" \\\"main.rs\\\" \\\"42\\\")";

primitive!(string_match, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let pattern = match &args[0] {
        ELispExp::String(text) => text.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let subject = match &args[1] {
        ELispExp::String(text) => text.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let compiled = match regex::Regex::new(&pattern) {
        Ok(compiled) => compiled,
        Err(why) => {
            ctx.log_diagnostic(&format!("{pattern:?} is not a regular expression: {why}"));
            return Ok(ELispExp::nil());
        }
    };
    let Some(captures) = compiled.captures(&subject) else {
        return Ok(ELispExp::nil());
    };
    // Every group, including the ones that did not take part -- as nil, so
    // that a caller counting groups is never off by one because an optional
    // one was absent.
    let groups = captures
        .iter()
        .map(|group| match group {
            Some(matched) => ELispExp::string(matched.as_str().to_string()),
            None => ELispExp::nil(),
        })
        .collect();
    Ok(ELispExp::proper_list(groups))
});
