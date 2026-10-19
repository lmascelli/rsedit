use super::*;
use risp::{Lambda, eval};

pub const CURRENT_BUFFER_DOC: &str = "(current-buffer): Return the name of the current buffer.";

primitive!(current_buffer, _args, _env, ctx, {
    Ok(ELispExp::string(ctx.get_current_buffer_name()))
});

pub const BUFFER_CREATE_DOC: &str = "(buffer-create NAME &optional MODE): Make an empty buffer \
         called NAME in MODE, fundamental-mode by default, without showing it, and return NAME. A \
         buffer of that name that exists already is left as it is, mode included.\n\n\
         Example:\n\
         (buffer-create \"*report*\")";

primitive!(buffer_create, args, env, ctx, {
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
            ctx.new_buffer(&name, None, mode, &env);
        }
        Ok(ELispExp::string(name))
    }
});

pub const SET_BUFFER_READ_ONLY_DOC: &str = "(set-buffer-read-only FLAG): Make the current buffer \
         refuse every change to its text when FLAG is non-nil, or accept changes again when it is \
         nil, and return FLAG. Point still moves.\n\n\
         Example:\n\
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

pub const BUFFER_PUT_DOC: &str = "(buffer-put KEY VALUE &optional BUFFER): Keep VALUE with \
         BUFFER, the current one by default, under KEY, and return VALUE. The value goes when the \
         buffer goes, and a VALUE of nil removes it. A BUFFER that does not exist gives nil.\n\n\
         Example:\n\
         (buffer-put 'occur-pattern \"TODO\")";

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

pub const BUFFER_GET_DOC: &str = "(buffer-get KEY &optional BUFFER): Return the value kept with \
         BUFFER, the current one by default, under KEY, or nil.\n\n\
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

pub const BUFFER_READ_ONLY_P_DOC: &str = "(buffer-read-only-p): Return t when the current buffer \
         refuses changes to its text, nil otherwise.";

primitive!(buffer_read_only_p, _args, _env, ctx, {
    let read_only = ctx.with_current_buffer(|buf| buf.read_only);
    Ok(if read_only {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

pub const CLOSE_BUFFER_DOC: &str = "(close-buffer &optional BUFFER-OR-NAME): Kill the buffer \
         named BUFFER-OR-NAME, the current one by default: `kill-buffer' under another name.";

pub const KILL_BUFFER_DOC: &str = "(kill-buffer &optional BUFFER-OR-NAME): Kill the buffer named \
         BUFFER-OR-NAME, the current one by default or when the name is empty, and return t when \
         it is gone. A buffer with unsaved changes to a file asks first, with yes typed in full, \
         and returns nil: it is killed when the answer comes. The windows that showed it show \
         another buffer.\n\n\
         Example:\n\
         (kill-buffer \"*report*\")";

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
         BUFFER-OR-NAME): Kill the buffer, asking nothing and saving nothing, and return t if \
         there was one.";

primitive!(kill_buffer_without_saving, args, env, ctx, {
    let target = close_target(args, ctx)?;
    Ok(if ctx.close_buffer(&target, &env) {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

pub const BUFFER_STRING_DOC: &str = "(buffer-string): Return the whole text of the current \
         buffer.";

primitive!(buffer_string, _args, _env, ctx, {
    Ok(ELispExp::string(
        ctx.with_current_buffer(|buf| buf.text.to_string()),
    ))
});

pub const BUFFER_SUBSTRING_DOC: &str = "(buffer-substring START END): Return the current buffer's \
         text between START and END, which may come in either order and are kept within the \
         buffer.\n\n\
         Example:\n\
         (buffer-substring (point-min) (point))";

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

pub const CLEAR_BUFFER_DOC: &str = "(clear-buffer): Delete the whole text of the current buffer.";

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

pub const SWITCH_TO_BUFFER_DOC: &str = "(switch-to-buffer BUFFER-NAME): Show the buffer named \
         BUFFER-NAME in the focused window, make it current, and return the name; nil when there \
         is no such buffer, since this does not make one.";

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

pub const WITH_CURRENT_BUFFER_DOC: &str = "(with-current-buffer NAME FUNCTION): Call FUNCTION \
         with no arguments while the buffer NAME is current, without showing it, and make the one \
         before current again, even when FUNCTION fails. Return what FUNCTION returns. A NAME \
         that is no buffer is an error.\n\n\
         Example:\n\
         (with-current-buffer \"*scratch*\" (lambda () (buffer-string)))";

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

pub const MAJOR_MODE_DOC: &str = "(major-mode &optional BUFFER): Return the major mode of BUFFER, \
         the current one by default, as a symbol, or nil when it does not exist.\n\n\
         Example:\n\
         (get (major-mode) 'tab-width)";

primitive!(major_mode, args, _env, ctx, {
    let name = buffer_argument(args, ctx)?;
    let Some(mode) = ctx.with_buffer(&name, |buf| buf.current_mode.clone()) else {
        return Ok(ELispExp::nil());
    };
    Ok(ELispExp::symbol(mode))
});

pub const SET_MAJOR_MODE_DOC: &str = "(set-major-mode MODE): Put the current buffer in MODE, \
         which must be a mode the editor knows, and return MODE. A different mode runs the hooks \
         the `modes' page describes; the same one changes nothing.\n\n\
         Example:\n\
         (set-major-mode 'rust-mode)";

primitive!(set_major_mode, args, env, ctx, {
    exact_arity(args, 1)?;
    let mode = crate::primitives::args::name(args.first())?;
    if !ctx.modes(|modes| modes.get(&mode).is_some()) {
        return Err(EvalError::RuntimeMessage(format!(
            "No such major mode: {mode}"
        )));
    }
    let name = ctx.get_current_buffer_name();
    ctx.change_major_mode(&name, &mode, &env);
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

pub const BUFFER_MODIFIED_P_DOC: &str = "(buffer-modified-p &optional BUFFER): Return t when \
         BUFFER, the current one by default, has changes that have not been saved, nil when it \
         has none or does not exist.";

primitive!(buffer_modified_p, args, _env, ctx, {
    let name = buffer_argument(args, ctx)?;
    let modified = ctx.with_buffer(&name, |buf| buf.is_modified) == Some(true);
    Ok(if modified {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

pub const BUFFER_FILE_NAME_DOC: &str = "(buffer-file-name &optional BUFFER): Return the file \
         BUFFER, the current one by default, is visiting, or nil when it has none or does not \
         exist.";

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
    into.function("set-major-mode", set_major_mode, SET_MAJOR_MODE_DOC);
}
