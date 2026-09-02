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
            let actual_ast = if let ELispExp::Symbol(symbol_name) = ast {
                if let Some(_) = env.get_function(symbol_name) {
                    ELispExp::form(vec![ast.clone()])
                } else {
                    ast.clone()
                }
            } else {
                ast.clone()
            };
            if let Some(key_event) = parse_key_sequence(key_str) {
                if let Some(mode_name) = mode_name {
                    let mut mode_registry_lock = ctx
                        .mode_registry
                        .write()
                        .expect("Failed to acquire write lock on mode_registry");
                    if let Some(mode) = mode_registry_lock.get_mut(&mode_name) {
                        mode.keymaps.insert(key_event, actual_ast);
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
                    keymaps.insert(key_event, actual_ast);
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
         Example:\n\
         (set-echo-message \"Saved\")";

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
