use super::*;
use risp::{Lambda, eval};

pub const CURRENT_BUFFER_DOC: &str = "(current-buffer): Return the name of the current buffer, as a \
         string. Unlike real Emacs Lisp's `current-buffer`, which returns a \
         buffer object, this returns the buffer's name.\n\n\
         Example:\n\
         (current-buffer) => \"*scratch*\"";

primitive!(current_buffer, _args, _env, ctx, {
    Ok(ELispExp::string(ctx.get_current_buffer_name()))
});

pub const BUFFER_CREATE_DOC: &str = "(buffer-create NAME &optional MODE): Create a new, empty buffer \
         named NAME (a string or symbol), if one doesn't already exist -- does nothing \
         if it does. Does not switch to it or change what any window is displaying; see \
         `switch-to-buffer'. Returns NAME as a string.\n\n\
         MODE, if given, is the major mode the buffer opens in, and therefore which \
         keymap answers keys typed into it; it defaults to `fundamental'. A buffer that \
         is a *view* of something -- a directory listing, a backtrace -- needs this: its \
         mode is the whole of what makes its keys mean what they mean, and there is no \
         file name for `add-auto-mode' to key on.\n\n\
         An existing buffer keeps the mode it has. Re-creating a buffer is how a view is \
         refreshed, and a refresh that quietly reset the mode would take the keymap away \
         from the very buffer being refreshed.\n\n\
         Example:\n\
         (buffer-create \"*Messages*\")\n\
         (buffer-create \"*dired*\" 'dired-mode)";

primitive!(buffer_create, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
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
        let mode = args::optional_name(args.get(1), "String or Symbol naming a mode")?;
        if !ctx.has_buffer(&name) {
            ctx.new_buffer(&name, None, mode);
        }
        Ok(ELispExp::string(name))
    }
});

pub const SET_BUFFER_READ_ONLY_DOC: &str = "(set-buffer-read-only FLAG): Make the current buffer \
         refuse to have its text changed if FLAG is non-nil, or accept changes again if it \
         is nil. Returns FLAG.\n\n\
         The refusal is enforced where undo is, at the two functions every text change goes \
         through, so it holds against every command and every line of Lisp -- not only \
         against the keys a mode remembered to unbind. Point still moves freely: reading is \
         the reason to have a read-only buffer at all.\n\n\
         A buffer that presents state living somewhere else -- a directory listing, a list \
         of faces -- writes its contents with the flag off and turns it back on, so the \
         only way the text changes is by being rebuilt from the thing it describes.\n\n\
         Example:\n\
         (set-buffer-read-only nil)\n\
         (clear-buffer)\n\
         (insert listing)\n\
         (set-buffer-read-only t)";

primitive!(set_buffer_read_only, args, _env, ctx, {
    exact_arity(args, 1)?;
    let flag = args[0].is_truthy();
    ctx.with_current_buffer_mut(|buf| buf.read_only = flag);
    Ok(args[0].clone())
});

// ---------------------------------------------------------------------------
// Things hung on a buffer
// ---------------------------------------------------------------------------
//
// The Lisp front door to [`crate::buffer::Buffer::data`]. A module that needs
// to remember something about one buffer has had two choices until now: a
// global keyed by the buffer's name, which is wrong the moment the buffer is
// renamed and still there after it is killed, or the buffer's own text, which
// only works when the value is something you would have shown anyway.
//
// What goes in is an ordinary Lisp value, held in the same table Rust
// attaches typed values to, and freed with the buffer like everything else in
// it.

/// The name a Lisp value is filed under inside the buffer's table.
///
/// Prefixed so that a Lisp key and a Rust one cannot collide: a module
/// putting `"results"` and a Rust feature attaching its own `"results"` are
/// two different things, and the one that looked second would find a value of
/// a type it did not expect.
fn lisp_key(key: &str) -> String {
    format!("lisp:{key}")
}

fn data_key<B: BufferTrait>(args: &[ELispExp<B>]) -> Result<String, EvalError<EditorState<B>>> {
    match args.first() {
        Some(ELispExp::String(key)) | Some(ELispExp::Symbol(key)) => Ok(lisp_key(key)),
        other => Err(EvalError::WrongArgumentType {
            expected: "String or Symbol".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

/// Which buffer a data command was asked about: the named one, or this one.
fn data_buffer<B: BufferTrait>(args: &[ELispExp<B>], index: usize, ctx: &EditorState<B>) -> String {
    match args.get(index) {
        Some(ELispExp::String(name)) if !name.is_empty() => name.to_string(),
        Some(ELispExp::Symbol(name)) => name.to_string(),
        _ => ctx.get_current_buffer_name(),
    }
}

pub const BUFFER_PUT_DOC: &str = "(buffer-put KEY VALUE &optional BUFFER): Remember VALUE against \
         BUFFER -- the current one when it is omitted -- under KEY. Returns VALUE.\n\n\
         What `put' is for a symbol, this is for a buffer, and the difference is what it is \
         for: a symbol property is global and outlives everything, while this **goes when the \
         buffer goes**. That is the whole point. A module that kept `compilation--output-of-x' \
         in a global would still be holding it long after the buffer was killed, and would be \
         holding it under the wrong name the moment somebody renamed one.\n\n\
         A VALUE of nil removes the entry rather than storing nil, so putting nil and never \
         putting are the same state -- which is what `buffer-get' answering nil for both \
         already implies.\n\n\
         Returns nil, and logs, when there is no buffer called BUFFER.\n\n\
         Example:\n\
         (buffer-put 'occur-pattern \"TODO\")\n\
         (buffer-get 'occur-pattern) => \"TODO\"";

primitive!(buffer_put, args, _env, ctx, {
    if args.len() < 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let key = data_key(args)?;
    let value = args[1].clone();
    let name = data_buffer(args, 2, ctx);
    let stored = ctx.with_buffer_mut(&name, |buf| {
        if value.is_nil() {
            buf.forget_data(&key);
        } else {
            buf.put_data(&key, value.clone());
        }
    });
    if stored.is_none() {
        ctx.log_diagnostic(&format!("buffer-put: no buffer called {name}"));
        return Ok(ELispExp::nil());
    }
    Ok(args[1].clone())
});

pub const BUFFER_GET_DOC: &str = "(buffer-get KEY &optional BUFFER): What was remembered against \
         BUFFER -- the current one when it is omitted -- under KEY, or nil.\n\n\
         nil for a key nothing was put under, for a buffer that does not exist, and for a \
         buffer that was killed and made again: what is attached to a buffer dies with it.\n\n\
         Example:\n\
         (buffer-get 'occur-pattern)";

primitive!(buffer_get, args, _env, ctx, {
    let key = data_key(args)?;
    let name = data_buffer(args, 1, ctx);
    Ok(ctx
        .with_buffer(&name, |buf| {
            buf.data::<ELispExp<B>>(&key)
                .cloned()
                .unwrap_or_else(ELispExp::nil)
        })
        .unwrap_or_else(ELispExp::nil))
});

pub const BUFFER_READ_ONLY_P_DOC: &str = "(buffer-read-only-p): Return t if the current buffer \
         refuses text changes, nil otherwise. See `set-buffer-read-only'.\n\n\
         Example:\n\
         (buffer-read-only-p) => nil";

primitive!(buffer_read_only_p, _args, _env, ctx, {
    let read_only = ctx.with_current_buffer(|buf| buf.read_only);
    Ok(if read_only {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

pub const CLOSE_BUFFER_DOC: &str = "(close-buffer &optional BUFFER-OR-NAME): Close the buffer named \
         BUFFER-OR-NAME (a string or symbol), or the current buffer if no \
         argument is given.\n\n\
         A buffer visiting a file with unsaved changes is *not* closed: it asks first, and \
         closes when the question is answered -- so this returns nil in that case, because the \
         buffer is still there. A buffer with nowhere to save to is closed without a question, \
         which is what keeps `*scratch*' and every listing from asking about work that does not \
         exist. `kill-buffer-without-saving' skips the question altogether. Detaches it from whatever window is showing \
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

pub const KILL_BUFFER_DOC: &str = "(kill-buffer &optional BUFFER-OR-NAME): Emacs' name for \
         `close-buffer', and the one reachable from `M-x' and bound to `C-x k'. Closes the buffer \
         named BUFFER-OR-NAME, or the current buffer when the name is omitted or empty.\n\n\
         Answering the prompt with nothing kills the buffer you are in, which is what the key is \
         reached for nine times in ten.\n\n\
         A buffer visiting a file with unsaved changes is asked about before it goes -- typed \
         in full, because there is nothing to undo afterwards -- and killed when the answer \
         arrives. Until then it is still there, which is why this returns nil rather than t \
         when it asks. Buffers with no file behind them are killed without a question.\n\n\
         The same function as `close-buffer', under both names: one is what Emacs calls it and \
         the other is what this editor called it first, and two implementations would eventually \
         be two behaviours.\n\n\
         Example:\n\
         (kill-buffer)\n\
         (kill-buffer \"*Messages*\")";

/// Which buffer a close command was asked about.
///
/// An empty answer is not the buffer called "": the prompt is offered with a
/// default of "this one", and Return without typing is how that default is
/// taken.
fn close_target<B: BufferTrait>(
    args: &[ELispExp<B>],
    ctx: &EditorState<B>,
) -> Result<String, EvalError<EditorState<B>>> {
    Ok(match args.first() {
        None => ctx.get_current_buffer_name(),
        Some(ELispExp::String(name)) if name.is_empty() => ctx.get_current_buffer_name(),
        Some(exp) if exp.is_nil() => ctx.get_current_buffer_name(),
        Some(ELispExp::String(name)) => name.to_string(),
        Some(ELispExp::Symbol(name)) => name.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: other.clone(),
            });
        }
    })
}

primitive!(close_buffer, args, env, ctx, {
    let target = close_target(args, ctx)?;
    if crate::primitives::io::has_unsaved_work(ctx, &target) {
        // The question is asked and this command is over: the answer arrives
        // later, in another command, and closes the buffer then. See
        // `primitives::ask` for why nothing here can wait for it.
        //
        // A lambda with the name already in it, rather than a note of which
        // buffer was being asked about kept somewhere: two questions open at
        // once would be two lambdas, where a slot would be one of them
        // overwriting the other.
        let kill = ELispExp::lambda(Lambda {
            params: Vec::new(),
            optionals: Vec::new(),
            rest: None,
            body: vec![ELispExp::form(vec![
                ELispExp::symbol("kill-buffer-without-saving".into()),
                ELispExp::string(target.clone()),
            ])],
            env: env.clone(),
            doc: None,
        });
        eval(
            &ELispExp::form(vec![
                ELispExp::symbol("yes-or-no".into()),
                ELispExp::string(format!("{target} is unsaved. Kill it anyway?")),
                ELispExp::form(vec![ELispExp::symbol("quote".into()), kill]),
            ]),
            env,
            ctx,
        )?;
        // Not killed -- not yet, and perhaps not at all. A caller told `t`
        // here would go on to act as though the buffer were gone.
        return Ok(ELispExp::nil());
    }
    Ok(if ctx.close_buffer(&target, &env) {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

pub const KILL_BUFFER_WITHOUT_SAVING_DOC: &str = "(kill-buffer-without-saving &optional \
         BUFFER-OR-NAME): Close the buffer, asking nothing and saving nothing. Returns t if \
         there was one to close.\n\n\
         What `kill-buffer' calls once its question has been answered, and the way out for \
         anything that has already done the asking itself -- the same arrangement as `quit' and \
         `quit-without-saving'.\n\n\
         Example:\n\
         (kill-buffer-without-saving \"notes.txt\")";

primitive!(kill_buffer_without_saving, args, env, ctx, {
    let target = close_target(args, ctx)?;
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
    Ok(ELispExp::string(
        ctx.with_current_buffer(|buf| buf.text.to_string()),
    ))
});

pub const BUFFER_SUBSTRING_DOC: &str = "(buffer-substring START END): Return the text of the \
         current buffer between positions START and END, as a string. The positions are clamped \
         to the buffer and may be given in either order, so a caller holding a region does not \
         have to sort it first.\n\n\
         Unlike `buffer-string', this copies only what was asked for -- which is what makes it \
         usable on a key press, where reading a whole buffer to look at eight characters is the \
         difference between a completion and a pause.\n\n\
         Example:\n\
         (buffer-substring (point-min) (point)) => \"everything before point\"";

primitive!(buffer_substring, args, _env, ctx, {
    exact_arity(args, 2)?;
    let bound = |exp: &ELispExp<B>| match exp {
        ELispExp::Number(n) if n.is_finite() && *n >= 0.0 => Ok(*n as usize),
        other => Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: other.clone(),
        }),
    };
    let (from, to) = (bound(&args[0])?, bound(&args[1])?);

    Ok(ELispExp::string(ctx.with_current_buffer(|buf| {
        buf.text.slice(from.min(to), from.max(to))
    })))
});

pub const CLEAR_BUFFER_DOC: &str = "(clear-buffer): Delete the entire contents of the current buffer. \
         Not a standard Elisp primitive -- comparable to Emacs's \
         `erase-buffer`.\n\n\
         Example:\n\
         (clear-buffer)\n\
         (buffer-string) => \"\"";

primitive!(clear_buffer, _args, _env, ctx, {
    let happened = ctx.with_current_buffer_mut(|buf| {
        // Through the recording layer rather than straight to `clear`, so that
        // emptying a buffer is undoable like any other deletion. The layer
        // still uses `clear` underneath for a whole-buffer range, so this
        // costs one pass rather than one deletion per character.
        let len = buf.text.len();
        super::edits::delete_range(buf, 0, len)
    });

    Ok(super::edits::edited(ctx, happened))
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
    exact_arity(args, 2)?;
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
    let result = risp::call_callable(&args[1], &[], env.clone(), ctx);
    ctx.set_current_buffer(&previous);
    result
});

pub const MAJOR_MODE_DOC: &str = "(major-mode &optional BUFFER): The major mode BUFFER is in -- \
         the current buffer if it is omitted -- as a symbol. nil if BUFFER names no live \
         buffer.\n\n\
         A symbol rather than a string because that is how every other form taking a mode is \
         written -- `(make-mode 'rust-mode)', `(add-hook 'rust-mode ...)' -- and because it makes \
         `eq' work on the result, which is what anything keying data by mode needs.\n\n\
         Example:\n\
         (get (major-mode) 'keywords)";

primitive!(major_mode, args, _env, ctx, {
    let name = buffer_argument(args, ctx)?;
    let Some(mode) = ctx.with_buffer(&name, |buf| buf.current_mode.clone()) else {
        return Ok(ELispExp::nil());
    };
    Ok(ELispExp::symbol(mode))
});

/// The name of the buffer NAME refers to, or of the current one when it is
/// omitted.
///
/// Every per-buffer accessor takes its argument the same way -- `major-mode`,
/// `buffer-modified-p`, `buffer-file-name` -- so that a listing can ask all of
/// them about the same name without any of them being the odd one out.
///
/// A *name* rather than a handle. Whether a buffer by that name exists is then
/// answered by the accessor that goes looking, one lock later, instead of
/// here: there is no point resolving it twice, and a handle resolved here
/// could go stale before its caller used it.
fn buffer_argument<B: BufferTrait>(
    args: &[ELispExp<B>],
    ctx: &EditorState<B>,
) -> Result<String, EvalError<EditorState<B>>> {
    Ok(args::optional_name(args.first(), "String naming a buffer")?
        .unwrap_or_else(|| ctx.get_current_buffer_name()))
}

pub const BUFFER_MODIFIED_P_DOC: &str = "(buffer-modified-p &optional BUFFER): t if BUFFER has \
         changes that have not been saved -- the current buffer if it is omitted. nil if it has \
         none, and nil if BUFFER names no live buffer.\n\n\
         A buffer with no file behind it can still be modified: `*scratch*' counts as changed \
         the moment something is typed into it, which is what makes killing it worth a question.\n\n\
         Example:\n\
         (if (buffer-modified-p \"notes.txt\") (message \"unsaved\"))";

primitive!(buffer_modified_p, args, _env, ctx, {
    let name = buffer_argument(args, ctx)?;
    let modified = ctx.with_buffer(&name, |buf| buf.is_modified) == Some(true);
    Ok(if modified {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

pub const BUFFER_FILE_NAME_DOC: &str = "(buffer-file-name &optional BUFFER): The file BUFFER is \
         visiting, as a string -- the current buffer if it is omitted. nil when the buffer has no \
         file behind it, and nil if BUFFER names no live buffer.\n\n\
         nil for `*scratch*', `*Messages*', a directory listing, a completion strip: the buffers \
         that present something rather than holding a file. That is the question this answers, \
         and why a listing can use it to tell the two kinds apart.\n\n\
         Example:\n\
         (buffer-file-name) => \"/home/user/notes.txt\"";

primitive!(buffer_file_name, args, _env, ctx, {
    let name = buffer_argument(args, ctx)?;
    let path = ctx
        .with_buffer(&name, |buf| buf.file_path.clone())
        .flatten();
    Ok(match path {
        Some(path) => ELispExp::string(path),
        None => ELispExp::nil(),
    })
});

/// Register this module's primitives: making, naming, killing and switching buffers.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Remembering something about one buffer, for as long as it exists.
    into.function("buffer-put", buffer_put, BUFFER_PUT_DOC);
    into.function("buffer-get", buffer_get, BUFFER_GET_DOC);
    into.function("buffer-substring", buffer_substring, BUFFER_SUBSTRING_DOC);
    into.function("switch-to-buffer", switch_to_buffer, SWITCH_TO_BUFFER_DOC);
    into.function("current-buffer", current_buffer, CURRENT_BUFFER_DOC);
    into.function("buffer-create", buffer_create, BUFFER_CREATE_DOC);
    into.function("close-buffer", close_buffer, CLOSE_BUFFER_DOC);
    // The same function under Emacs' name, and *this* is the one registered as
    // a command: `M-x close-buffer` would be a second way to reach one thing,
    // named what no Emacs user would look for.
    into.command(
        "kill-buffer",
        close_buffer,
        &["bKill buffer: "],
        KILL_BUFFER_DOC,
    );
    into.command(
        "kill-buffer-without-saving",
        kill_buffer_without_saving,
        &["bKill buffer without saving: "],
        KILL_BUFFER_WITHOUT_SAVING_DOC,
    );
    into.function("buffer-string", buffer_string, BUFFER_STRING_DOC);
    into.function("clear-buffer", clear_buffer, CLEAR_BUFFER_DOC);
    into.function(
        "with-current-buffer",
        with_current_buffer,
        WITH_CURRENT_BUFFER_DOC,
    );
    into.function(
        "set-buffer-read-only",
        set_buffer_read_only,
        SET_BUFFER_READ_ONLY_DOC,
    );
    into.function(
        "buffer-read-only-p",
        buffer_read_only_p,
        BUFFER_READ_ONLY_P_DOC,
    );
    into.function(
        "buffer-modified-p",
        buffer_modified_p,
        BUFFER_MODIFIED_P_DOC,
    );
    into.function("buffer-file-name", buffer_file_name, BUFFER_FILE_NAME_DOC);
    into.function("major-mode", major_mode, MAJOR_MODE_DOC);
}
