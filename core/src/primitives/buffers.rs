use super::*;

pub const CURRENT_BUFFER_DOC: &str = "(current-buffer): Return the name of the current buffer, as a \
         string. Unlike real Emacs Lisp's `current-buffer`, which returns a \
         buffer object, this returns the buffer's name.\n\n\
         Example:\n\
         (current-buffer) => \"*scratch*\"";

primitive!(current_buffer, _args, _env, ctx, {
    Ok(ELispExp::string(ctx.get_current_buffer_name()))
});

pub const BUFFER_CREATE_DOC: &str = "(buffer-create NAME): Create a new, empty buffer named NAME (a \
         string or symbol) in fundamental-mode, if one doesn't already \
         exist -- does nothing if it does. Does not switch to it or \
         change what any window is displaying; see `switch-to-buffer'. \
         Returns NAME as a string.\n\n\
         Example:\n\
         (buffer-create \"*Messages*\")";

primitive!(buffer_create, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        let name = match &args[0] {
            ELispExp::String(name) => name.to_string(),
            ELispExp::Symbol(name) => name.to_string(),
            _ => {
                return Err(EvalError::WrongArgumentType {
                    expected: "String or Symbol".into(),
                    got: args[0].clone(),
                });
            }
        };
        if ctx.get_buffer(&name).is_none() {
            ctx.new_buffer(&name, None, None);
        }
        Ok(ELispExp::string(name))
    }
});

pub const CLOSE_BUFFER_DOC: &str = "(close-buffer &optional BUFFER-OR-NAME): Close the buffer named \
         BUFFER-OR-NAME (a string or symbol), or the current buffer if no \
         argument is given. Detaches it from whatever window is showing \
         it -- a tiled window falls back to *scratch*, a floating window \
         is removed and focus returns to whatever was focused before it \
         opened -- runs that buffer's major mode's after-close-hook, and \
         removes it from the buffer list. If BUFFER-OR-NAME was the last \
         remaining buffer, a fresh empty *scratch* is created so the \
         editor is never left without one. Returns t on success, nil if \
         no such buffer exists. Not a standard Elisp primitive -- the \
         closest real Elisp equivalent is `kill-buffer`.\n\n\
         Example:\n\
         (close-buffer) ; closes the current buffer\n\
         (close-buffer \"*Minibuffer*\")";

primitive!(close_buffer, args, env, ctx, {
    let target = match args.first() {
        None => ctx.get_current_buffer_name(),
        Some(ELispExp::String(name)) => name.to_string(),
        Some(ELispExp::Symbol(name)) => name.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    Ok(if ctx.close_buffer(&target, &env) {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

pub const BUFFER_STRING_DOC: &str = "(buffer-string): Return the entire contents of the current buffer as \
         a string.\n\n\
         Example:\n\
         (buffer-string) => \"line one\\nline two\\n\"";

primitive!(buffer_string, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let content = buf
        .read()
        .expect("Failed to acquire read lock on current buffer")
        .text
        .to_string();
    Ok(ELispExp::string(content))
});

pub const CLEAR_BUFFER_DOC: &str = "(clear-buffer): Delete the entire contents of the current buffer. \
         Not a standard Elisp primitive -- comparable to Emacs's \
         `erase-buffer`.\n\n\
         Example:\n\
         (clear-buffer)\n\
         (buffer-string) => \"\"";

primitive!(clear_buffer, _args, _env, ctx, {
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        // Through the recording layer rather than straight to `clear`, so that
        // emptying a buffer is undoable like any other deletion. The layer
        // still uses `clear` underneath for a whole-buffer range, so this
        // costs one pass rather than one deletion per character.
        let len = buf.text.len();
        super::edits::delete_range(buf, 0, len);
    });

    Ok(ELispExp::nil())
});

pub const SWITCH_TO_BUFFER_DOC: &str = "(switch-to-buffer BUFFER-NAME): Make the buffer named BUFFER-NAME \
         (a string or symbol) the one shown in the focused window, and \
         return BUFFER-NAME. Returns nil (logging a diagnostic) if no buffer \
         with that name exists -- unlike real Emacs Lisp's \
         `switch-to-buffer`, this does not create one.\n\n\
         Example:\n\
         (switch-to-buffer \"*scratch*\") => \"*scratch*\"";

primitive!(switch_to_buffer, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        let buffer_name = match &args[0] {
            ELispExp::String(name) => Some(name.to_string()),
            ELispExp::Symbol(name) => Some(name.to_string()),
            _ => None,
        };
        if let Some(buffer_name) = buffer_name {
            if ctx.switch_to_buffer(&buffer_name) {
                Ok(args[0].clone())
            } else {
                Ok(ELispExp::nil())
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: args.first().cloned().unwrap_or_else(ELispExp::nil),
            })
        }
    }
});

pub const WITH_CURRENT_BUFFER_DOC: &str = "(with-current-buffer NAME FUNCTION): Call FUNCTION with \
         no arguments while NAME is the current buffer, then make whatever was current before \
         current again -- whether FUNCTION returned or signalled. Returns what FUNCTION returned.\n\n\
         The buffer is made current without being *shown*: no window changes, so this is for code \
         that wants to act on a buffer rather than take the user to it.\n\n\
         Signals if there is no buffer called NAME.\n\n\
         Unlike Emacs Lisp's macro of the same name, this takes a function rather than a body, \
         because a primitive receives its arguments already evaluated. Wrap the body in a lambda:\n\n\
         Example:\n\
         (with-current-buffer \"notes.txt\" (lambda () (buffer-string)))";

primitive!(with_current_buffer, args, env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let name = match &args[0] {
        ELispExp::String(name) | ELispExp::Symbol(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    };
    let Some(previous) = ctx.set_current_buffer(&name) else {
        return Err(EvalError::RuntimeMessage(format!("No such buffer: {name}")));
    };

    // The call is not allowed to leave the editor pointing somewhere the caller
    // did not ask for, so the result is caught rather than propagated with `?`
    // and the buffer is put back either way. An error that escaped here would
    // strand every later command on whatever buffer this one happened to be
    // visiting.
    let result = crate::lisp::call_callable(&args[1], &[], env.clone(), ctx);
    ctx.set_current_buffer(&previous);
    result
});
