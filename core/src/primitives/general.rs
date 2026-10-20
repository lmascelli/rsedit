use super::*;

pub const EVAL_FILE_DOC: &str = "(eval-file FILE): Evaluate the Lisp in FILE, and return a \
         one-element list holding the value of its last form, or nil when there is nothing to \
         evaluate.\n\n\
         FILE is a path, relative to the directory the editor was started in. When no such file \
         exists and FILE has no directory, extension or dot in it, FILE.lisp is looked for in \
         each directory of `lisp-path' in turn. A file that is not found is logged; a file that \
         does not parse is skipped.\n\n\
         Example:\n\
         (eval-file \"dired\")                          ; dired.lisp, from lisp-path\n\
         (eval-file (expand-file-name \"~/lisp/mine.lisp\"))";

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

pub const SET_LOCAL_KEYMAP_DOC: &str = "(set-local-keymap MODE): Give the current buffer the \
         keymap of the mode MODE, looked in after a transient keymap and before the buffer's own \
         mode, which stays as it is; nil takes it away. Return MODE; an unknown mode is refused. \
         (buffer-get 'local-keymap) reads it back.";

primitive!(set_local_keymap, args, _env, ctx, {
    exact_arity(args, 1)?;
    if !args[0].is_nil() {
        let name = args::name(args.first())?;
        if !ctx.modes(|modes| modes.get(&name).is_some()) {
            return Err(EvalError::RuntimeMessage(format!(
                "Unknown keymap mode: {name}"
            )));
        }
    }
    ctx.with_current_buffer_mut(|buf| {
        if args[0].is_nil() {
            buf.forget_data("lisp:local-keymap");
        } else {
            buf.put_data("lisp:local-keymap", args[0].clone());
        }
    });
    // A lifecycle hook may install a map in a hidden buffer while the user
    // has a partial key sequence in the focused one.
    if ctx.focused_window_buffer().as_deref() == Some(ctx.get_current_buffer_name().as_str()) {
        ctx.abandon_pending_input();
    }
    Ok(args[0].clone())
});

pub const DEFINE_KEY_DOC: &str = "(define-key MODE KEY COMMAND): Bind KEY, a key sequence such as \
         \"C-x C-f\" or \"<ret>\", to COMMAND in the global keymap when MODE is nil, or in the \
         keymap of the mode MODE. COMMAND is the name of a function, run with no arguments, or an \
         expression, evaluated as written. Return t, or nil when KEY or MODE is unknown. The KEY \
         \"<char>\" binds the characters the keymap does not: COMMAND is then a function, given \
         the character as a string, or nil to take it away.\n\n\
         Example:\n\
         (define-key nil \"C-c l\" 'goto-line)\n\
         (define-key 'rust-mode \"C-c d\" '(insert \"dbg!()\"))";

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

            if key_str.as_str() == "<char>" {
                if !ast.is_nil() && !matches!(ast, ELispExp::Symbol(_) | ELispExp::Lambda(_)) {
                    return Err(EvalError::WrongArgumentType {
                        expected: "Function name, lambda or nil for <char>".into(),
                        got: ast.clone(),
                    });
                }
                let bound = ctx.modes_mut(|modes| match &mode_name {
                    Some(name) => {
                        modes.edit(name, |mode| mode.keymaps.set_character_handler(ast.clone()))
                    }
                    None => {
                        modes.global_keymap_mut().set_character_handler(ast.clone());
                        true
                    }
                });
                return Ok(ELispExp::boolean(bound));
            }

            let actual_ast = command_ast(ast, &env);
            if let Some(keys) = parse_key_sequence(key_str) {
                if let Some(mode_name) = mode_name {
                    // Reported outside the closure: the diagnostic writes the
                    // echo area, which is a compartment of its own.
                    let bound = ctx.modes_mut(|modes| {
                        modes.edit(&mode_name, |mode| mode.keymaps.insert(keys, actual_ast))
                    });
                    if bound {
                        Ok(ELispExp::t())
                    } else {
                        ctx.log_diagnostic(&format!(
                            "[ERROR]: define-key no mode named {mode_name}"
                        ));
                        Ok(ELispExp::nil())
                    }
                } else {
                    ctx.modes_mut(|modes| modes.global_keymap_mut().insert(keys, actual_ast));
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

pub const LOG_DOC: &str = "(log MESSAGE): Add the string MESSAGE to the log, without showing \
         it. It reaches the log file, when there is one, within a moment.\n\n\
         Example:\n\
         (log \"my-mode: index rebuilt\")";

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

pub const ALL_LOGS_DOC: &str = "(all-logs): Return the log, oldest line first, as a list of \
         strings: what `log' and `message' added and what the editor reported itself. The \
         oldest lines go once there are many.\n\n\
         Example:\n\
         (length (all-logs))";

primitive!(all_logs, _args, _env, ctx, {
    Ok(ELispExp::proper_list(
        ctx.get_logs().into_iter().map(ELispExp::string).collect(),
    ))
});

pub const BACKTRACE_DOC: &str = "(backtrace): Return the call stack of the most recent failure, \
         innermost call first, as a list of function names; nil once that failure has been \
         reported. A call made last in its function is not in the list: by the time it ran, the \
         function that made it had returned.\n\n\
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

pub const REPEAT_DOC: &str = "(repeat): Run the last command again, and return what it returned. \
         A command that asks for arguments asks again. With nothing run yet, say so and return \
         nil. C-x z, and z after it.\n\n\
         Example:\n\
         (define-key nil \"C-c r\" 'repeat)";

primitive!(repeat, _args, env, ctx, {
    let Some(form) = ctx.last_command_form() else {
        ctx.set_echo_message("No command to repeat");
        return Ok(ELispExp::nil());
    };
    // Evaluated here rather than pushed back through the key handler: this is
    // already running inside a command, so it inherits that command's undo
    // group and hooks. Re-entering the dispatcher would open a second one
    // inside the first.
    risp::eval(&form, env.clone(), ctx)
});

pub const DEFINE_REPEAT_KEY_DOC: &str = "(define-repeat-key COMMAND KEY): Offer KEY, a single \
         key, after COMMAND runs: pressing it runs COMMAND again, and offers it again. The offer \
         shows in the echo area, and any other key ends it and does what it always does. Return \
         t.\n\n\
         Example:\n\
         (define-repeat-key 'kill-word \"d\")   ; M-d d d kills three words";

primitive!(define_repeat_key, args, _env, ctx, {
    exact_arity(args, 2)?;
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
         PASS-THROUGH): Put up a keymap looked in before every other, until \
         `clear-transient-keymap' takes it down, and return t. BINDINGS is a list of (KEY . \
         COMMAND), written as for `define-key'; MESSAGE shows in the echo area while it stands. A \
         key it does not bind does nothing, or, with PASS-THROUGH, goes on to the other keymaps \
         while the keymap stays. Nothing but its own bindings takes it down.\n\n\
         Example:\n\
         (set-transient-keymap (list (cons \"C-g\" 'clear-transient-keymap)) \"C-g to go on\")";

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
         `set-transient-keymap' put up, so that keys reach the buffer's keymaps again, and return \
         t. The binding of that keymap that ends what it was put up for calls this: the keymap \
         does not go by itself.";

primitive!(clear_transient_keymap, _args, _env, ctx, {
    ctx.clear_transient_keymap();
    Ok(ELispExp::t())
});

pub const REGEXP_OPT_DOC: &str = "(regexp-opt WORDS): Return a regular expression that matches \
         any of WORDS, a list of strings or symbols, every character special to a pattern \
         escaped, in a group that is not numbered. An empty list gives a pattern that matches \
         nothing.\n\n\
         Example:\n\
         (regexp-opt '(\"fn\" \"let\"))   ; \"(?:fn|let)\"";

primitive!(regexp_opt, args, _env, _ctx, {
    exact_arity(args, 1)?;
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

/// Register this module's primitives: the odds and ends that belong to no one feature.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("eval-file", eval_file, EVAL_FILE_DOC);
    into.function("define-key", define_key, DEFINE_KEY_DOC);
    into.function("set-local-keymap", set_local_keymap, SET_LOCAL_KEYMAP_DOC);
    into.function(
        "define-repeat-key",
        define_repeat_key,
        DEFINE_REPEAT_KEY_DOC,
    );
    into.command("repeat", repeat, &[], REPEAT_DOC);
    into.function("log", log, LOG_DOC);
    into.function("regexp-opt", regexp_opt, REGEXP_OPT_DOC);
    into.function("all-logs", all_logs, ALL_LOGS_DOC);
    into.function("backtrace", backtrace, BACKTRACE_DOC);
    into.function("set-echo-message", set_echo_message, SET_ECHO_MESSAGE_DOC);
    // A keymap that is a question. Not commands: `M-x set-transient-keymap`
    // would put the editor into a state whose way out is in an argument the
    // user was never asked for.
    into.function(
        "set-transient-keymap",
        set_transient_keymap,
        SET_TRANSIENT_KEYMAP_DOC,
    );
    into.function(
        "clear-transient-keymap",
        clear_transient_keymap,
        CLEAR_TRANSIENT_KEYMAP_DOC,
    );
}
