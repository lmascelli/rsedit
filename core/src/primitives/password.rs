//! Reading a password from Lisp: `read-passwd`, which opens a prompt whose
//! answer is never in its buffer and hands it to a callback; the commands
//! that type and edit it there; `process-send-password`, which writes the
//! answer to a process without making a string of it; and the cache, Emacs'
//! `password-read' and its company. What is kept, and how the buffer shows
//! it, is [`crate::feature::password`].
//!
//! Design: `doc/architettura-editor.typ, <password>`.
use super::*;
use crate::editor::processes::Form;
use crate::feature::minibuffer::PROMPT_BUFFER;
use crate::feature::password::{
    self, Answer, CONFIRM_PROMPT, NOT_REPEATED, PROMPT_DATA, PasswordPrompt, Request, Secret,
};
use risp::call_callable;
use std::sync::Arc;
use std::time::Instant;

type Failure<B> = EvalError<EditorState<B>>;

// ---------------------------------------------------------------------------
// Opening the prompt
// ---------------------------------------------------------------------------

/// Opens the prompt REQUEST asks for, through `minibuffer-read`, and so
/// through the reader `*minibuffer-read-function*` names, in
/// `read-passwd-mode`: the question REQUEST's, or the confirmation's when
/// FIRST, the first answer, is given.
///
/// A reader that opens no prompt, or one in another mode, where what is typed
/// would go into the buffer, is an error, and its prompt is closed.
fn open<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    request: Arc<Request<B>>,
    first: Option<Secret>,
) -> Result<(), Failure<B>> {
    let title = if first.is_some() {
        CONFIRM_PROMPT.to_string()
    } else {
        request.prompt.clone()
    };
    super::minibuffer::minibuffer_read(
        &[
            ELispExp::string(title),
            // Return is the mode's own, which reads the answer from where it
            // is kept: the prompt's ON-CONFIRM would be handed what the
            // buffer shows.
            ELispExp::nil(),
            ELispExp::nil(),
            request.on_cancel.clone(),
            ELispExp::string(password::PASSWORD_MODE.to_string()),
        ],
        env.clone(),
        ctx,
    )?;
    // Read before the buffer is taken: a setting is read under no lock.
    let hide = password::hide_char(env);
    let opened = ctx.with_buffer_mut(PROMPT_BUFFER, |buf| {
        if buf.current_mode != password::PASSWORD_MODE {
            return false;
        }
        password::attach(buf, PasswordPrompt::new(request, first), hide);
        true
    });
    match opened {
        Some(true) => Ok(()),
        Some(false) => {
            ctx.close_buffer(PROMPT_BUFFER, env);
            Err(EvalError::RuntimeMessage(
                "read-passwd: the prompt *minibuffer-read-function* opened is not in \
                 read-passwd-mode, and would show the password"
                    .into(),
            ))
        }
        None => Err(EvalError::RuntimeMessage(
            "read-passwd: *minibuffer-read-function* opened no prompt".into(),
        )),
    }
}

/// Returns the optional default of a password prompt: a string, or `None`
/// for nil or nothing.
fn default_answer<B: BufferTrait>(
    value: Option<&ELispExp<B>>,
) -> Result<Option<String>, Failure<B>> {
    match value {
        None => Ok(None),
        Some(value) if value.is_nil() => Ok(None),
        Some(ELispExp::String(text)) => Ok(Some(text.to_string())),
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: "String or nil".into(),
            got: other.clone(),
        }),
    }
}

const READ_PASSWD_DOC: &str = "(read-passwd PROMPT ON-CONFIRM &optional CONFIRM DEFAULT &key \
         :confirm :default :on-cancel): Ask for a password in the minibuffer, titled PROMPT, \
         without showing it, and return at once. ON-CONFIRM, when not nil, is called with the \
         password, a string, when Return is pressed, and :on-cancel, when not nil, with no \
         arguments when Escape or C-g is. With CONFIRM, the password is asked for twice, and \
         again from the start when the two differ. DEFAULT is what an empty answer stands for. \
         CONFIRM and DEFAULT each go in their place or after their keyword, not both. Each \
         character typed is drawn as `read-hide-char'; Tab shows the password and hides it \
         again. What is typed is never in the buffer, its undo, the kill ring, the history or \
         a keyboard macro: see the `minibuffer' manual page. Refused in a background job.\n\n\
         Example:\n\
         (read-passwd \"Password: \" (lambda (password) (message \"%d characters\" (length password))))";

/// The keywords `read-passwd` takes.
const READ_PASSWD_KEYWORDS: &[&str] = &[":confirm", ":default", ":on-cancel"];

primitive!(read_passwd, args, env, ctx, {
    const NAME: &str = "read-passwd";
    // Read whole before anything opens, so that a call wrong anywhere opens
    // nothing.
    let (positional, keywords) = args::with_keywords(NAME, args, 2, 4, READ_PASSWD_KEYWORDS)?;
    let prompt = args::text(positional.first())?;
    let confirm = args::positional_or_keyword(NAME, positional, 2, &keywords, ":confirm")?
        .is_some_and(|value| value.is_truthy());
    let default = default_answer(args::positional_or_keyword(
        NAME, positional, 3, &keywords, ":default",
    )?)?;
    let request = Request {
        prompt,
        confirm,
        default,
        on_cancel: keywords
            .get(":on-cancel")
            .cloned()
            .unwrap_or_else(ELispExp::nil),
        answer: Answer::Call {
            function: positional[1].clone(),
            cache_key: None,
        },
    };
    open(ctx, &env, Arc::new(request), None)?;
    Ok(ELispExp::t())
});

// ---------------------------------------------------------------------------
// Typing at it
// ---------------------------------------------------------------------------

/// Takes the open password prompt out of the current buffer, which must be
/// one, with where the buffer's point is.
fn take_prompt<B: BufferTrait>(ctx: &EditorState<B>) -> Option<(Box<PasswordPrompt<B>>, usize)> {
    ctx.with_current_buffer_mut(|buf| {
        if buf.name != PROMPT_BUFFER {
            return None;
        }
        let prompt = buf
            .data
            .remove(PROMPT_DATA)?
            .downcast::<PasswordPrompt<B>>()
            .ok()?;
        Some((prompt, buf.text.cursor_pos_1d()))
    })
}

/// Returns the error of a command of the password prompt run elsewhere.
fn not_in_prompt<B: BufferTrait>(name: &str) -> Failure<B> {
    EvalError::RuntimeMessage(format!("{name}: not in a password prompt"))
}

/// Runs CHANGE on what has been typed at the password prompt that is the
/// current buffer, and where its cursor is, and shows the outcome: CHANGE
/// returns where the cursor goes. An error naming NAME when the current
/// buffer is no password prompt.
fn edit<B: BufferTrait>(
    name: &str,
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    change: impl FnOnce(&mut PasswordPrompt<B>, usize) -> usize,
) -> Result<ELispExp<B>, Failure<B>> {
    let hide = password::hide_char(env);
    let edited = ctx.with_current_buffer_mut(|buf| {
        if buf.name != PROMPT_BUFFER {
            return false;
        }
        let Some(mut prompt) = buf
            .data
            .remove(PROMPT_DATA)
            .and_then(|data| data.downcast::<PasswordPrompt<B>>().ok())
        else {
            return false;
        };
        let cursor = prompt.cursor(buf.text.cursor_pos_1d(), hide);
        let cursor = change(&mut prompt, cursor);
        password::redraw(buf, &prompt, cursor, hide);
        buf.data.insert(PROMPT_DATA.to_string(), prompt);
        true
    });
    if edited {
        Ok(ELispExp::nil())
    } else {
        Err(not_in_prompt(name))
    }
}

/// Returns TEXT as it goes into a password: without its line breaks, which
/// a password typed on one line cannot have, and which a paste copied with
/// a line brings along.
fn one_line(text: &str) -> String {
    text.chars()
        .filter(|ch| *ch != '\n' && *ch != '\r')
        .collect()
}

/// Puts TEXT into the password prompt that is the current buffer, at its
/// cursor, and returns whether there was one: what a paste does there,
/// rather than putting TEXT in the buffer. Not a command: a paste reaches it
/// without becoming one, so that its text is in no form the editor keeps.
pub(crate) fn insert_typed<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    text: &str,
) -> bool {
    let mut text = one_line(text);
    let done = edit("insert-pasted-text", ctx, env, |prompt, cursor| {
        prompt.typed.insert(cursor, &text)
    })
    .is_ok();
    password::wipe_string(&mut text);
    done
}

const READ_PASSWD_SELF_INSERT_DOC: &str = "(read-passwd-self-insert STRING): Put STRING into the \
         password being typed, at the cursor, without putting it in the buffer, which shows a \
         `read-hide-char' for each character instead. What a character runs in a password \
         prompt; not called directly.";

primitive!(read_passwd_self_insert, args, env, ctx, {
    let text = args::text(args.first())?;
    let mut text = one_line(&text);
    let result = edit("read-passwd-self-insert", ctx, &env, |prompt, cursor| {
        prompt.typed.insert(cursor, &text)
    });
    password::wipe_string(&mut text);
    result
});

const READ_PASSWD_DELETE_BACKWARD_CHAR_DOC: &str = "(read-passwd-delete-backward-char): Delete \
         the character of the password before the cursor. Backspace in a password prompt.";

primitive!(read_passwd_delete_backward_char, _args, env, ctx, {
    edit(
        "read-passwd-delete-backward-char",
        ctx,
        &env,
        |prompt, cursor| {
            let from = cursor.saturating_sub(1);
            prompt.typed.delete(from, cursor);
            from
        },
    )
});

const READ_PASSWD_DELETE_CHAR_DOC: &str = "(read-passwd-delete-char): Delete the character of \
         the password after the cursor. C-d and Delete in a password prompt.";

primitive!(read_passwd_delete_char, _args, env, ctx, {
    edit("read-passwd-delete-char", ctx, &env, |prompt, cursor| {
        prompt.typed.delete(cursor, cursor + 1);
        cursor
    })
});

const READ_PASSWD_KILL_LINE_DOC: &str = "(read-passwd-kill-line): Delete the password from the \
         cursor to its end. Nothing goes on the kill ring. C-k in a password prompt.";

primitive!(read_passwd_kill_line, _args, env, ctx, {
    edit("read-passwd-kill-line", ctx, &env, |prompt, cursor| {
        let end = prompt.typed.len();
        prompt.typed.delete(cursor, end);
        cursor
    })
});

const READ_PASSWD_ERASE_DOC: &str = "(read-passwd-erase): Delete the whole password typed so \
         far. C-u in a password prompt, as in Emacs and at a terminal.";

primitive!(read_passwd_erase, _args, env, ctx, {
    edit("read-passwd-erase", ctx, &env, |prompt, _| {
        prompt.typed.clear();
        0
    })
});

const READ_PASSWD_YANK_DOC: &str = "(read-passwd-yank): Put the latest kill into the password, \
         at the cursor, without its line breaks. C-y in a password prompt; a paste does the \
         same with what is pasted.";

primitive!(read_passwd_yank, _args, env, ctx, {
    let Some(kill) = ctx.current_kill() else {
        return edit("read-passwd-yank", ctx, &env, |_, cursor| cursor);
    };
    let mut kill = one_line(&kill);
    let result = edit("read-passwd-yank", ctx, &env, |prompt, cursor| {
        prompt.typed.insert(cursor, &kill)
    });
    password::wipe_string(&mut kill);
    result
});

const READ_PASSWD_TOGGLE_VISIBILITY_DOC: &str = "(read-passwd-toggle-visibility): Show the \
         password being typed, or hide it again. While it is shown it is in the prompt's \
         buffer, which still refuses to have it killed, copied or undone. Tab in a password \
         prompt.";

primitive!(read_passwd_toggle_visibility, _args, env, ctx, {
    let mut shown = false;
    let result = edit(
        "read-passwd-toggle-visibility",
        ctx,
        &env,
        |prompt, cursor| {
            prompt.shown = !prompt.shown;
            shown = prompt.shown;
            cursor
        },
    );
    if result.is_ok() {
        ctx.set_echo_message(if shown {
            "The password is shown: Tab hides it"
        } else {
            "The password is hidden"
        });
    }
    result
});

// ---------------------------------------------------------------------------
// Answering it
// ---------------------------------------------------------------------------

const READ_PASSWD_CONFIRM_DOC: &str = "(read-passwd-confirm): Close the password prompt and do \
         with the password what the prompt was opened for; with CONFIRM, the first time ask for \
         it again, and when the two differ start over. An empty answer is the prompt's DEFAULT, \
         when it has one. Return in a password prompt; `minibuffer-confirm' there does the \
         same.";

primitive!(read_passwd_confirm, _args, env, ctx, {
    let Some((prompt, _)) = take_prompt(ctx) else {
        return Err(not_in_prompt("read-passwd-confirm"));
    };
    let PasswordPrompt {
        request,
        mut typed,
        first,
        ..
    } = *prompt;
    if typed.is_empty()
        && let Some(default) = &request.default
    {
        typed = Secret::from_text(default);
    }
    // Closed before anything is done with the answer: what is called next
    // may open a prompt of its own.
    ctx.close_buffer(PROMPT_BUFFER, &env);
    if request.confirm {
        match first {
            None => {
                open(ctx, &env, request, Some(typed))?;
                return Ok(ELispExp::nil());
            }
            Some(first) if first != typed => {
                ctx.set_echo_message(NOT_REPEATED);
                open(ctx, &env, request, None)?;
                return Ok(ELispExp::nil());
            }
            Some(_) => {}
        }
    }
    deliver(ctx, &env, &request, typed)?;
    Ok(ELispExp::nil())
});

/// Does with TYPED, the answer, what REQUEST asked for.
fn deliver<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    request: &Request<B>,
    typed: Secret,
) -> Result<(), Failure<B>> {
    match &request.answer {
        Answer::Call {
            function,
            cache_key,
        } => {
            if let Some(key) = cache_key {
                add_to_cache(ctx, env, key, typed.duplicate());
            }
            if function.is_truthy() {
                call_callable(
                    function,
                    &[ELispExp::string(typed.reveal())],
                    env.clone(),
                    ctx,
                )?;
            }
        }
        Answer::Send {
            process,
            newline,
            on_sent,
        } => {
            let sent = ctx.dialogue(process).is_some_and(|dialogue| {
                let mut body = typed.bytes_with(newline);
                let bytes = match dialogue.exchange.form() {
                    Form::Messages => {
                        let mut framed =
                            format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
                        framed.extend_from_slice(&body);
                        password::wipe_bytes(&mut body);
                        framed
                    }
                    Form::Chunks | Form::Lines => body,
                };
                dialogue.exchange.send(bytes)
            });
            if !sent {
                ctx.set_echo_message(&format!(
                    "The process {process} is not reading: the password did not go"
                ));
            }
            if on_sent.is_truthy() {
                call_callable(on_sent, &[ELispExp::boolean(sent)], env.clone(), ctx)?;
            }
        }
    }
    Ok(())
}

const PROCESS_SEND_PASSWORD_DOC: &str = "(process-send-password NAME PROMPT &key :newline \
         :confirm :on-sent :on-cancel): Ask for a password in the minibuffer, as `read-passwd' \
         does, and write it to the input of the process NAME, followed by :newline, \"\\n\" by \
         default. The password is never made a string of Lisp's. Return t once the prompt is \
         open, or nil, asking nothing, when there is no such process. :on-sent, when not nil, \
         is called with t once the password is written, or nil when the process was not \
         reading; :on-cancel, with no arguments, when the prompt is closed without an \
         answer.\n\n\
         Example:\n\
         (process-send-password 'sudo \"[sudo] password: \")";

/// The keywords `process-send-password` takes.
const PROCESS_SEND_PASSWORD_KEYWORDS: &[&str] = &[":newline", ":confirm", ":on-sent", ":on-cancel"];

primitive!(process_send_password, args, env, ctx, {
    const NAME: &str = "process-send-password";
    let (positional, keywords) =
        args::with_keywords(NAME, args, 2, 2, PROCESS_SEND_PASSWORD_KEYWORDS)?;
    let process = args::name(positional.first())?;
    let prompt = args::text(positional.get(1))?;
    let newline = match keywords.get(":newline") {
        None => "\n".to_string(),
        Some(value) if value.is_nil() => String::new(),
        Some(ELispExp::String(text)) => text.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String or nil".into(),
                got: other.clone(),
            });
        }
    };
    if ctx.dialogue(&process).is_none() {
        return Ok(ELispExp::nil());
    }
    let request = Request {
        prompt,
        confirm: keywords
            .get(":confirm")
            .is_some_and(|value| value.is_truthy()),
        default: None,
        on_cancel: keywords
            .get(":on-cancel")
            .cloned()
            .unwrap_or_else(ELispExp::nil),
        answer: Answer::Send {
            process,
            newline,
            on_sent: keywords
                .get(":on-sent")
                .cloned()
                .unwrap_or_else(ELispExp::nil),
        },
    };
    open(ctx, &env, Arc::new(request), None)?;
    Ok(ELispExp::t())
});

// ---------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------

/// Keeps SECRET in the cache under KEY, for as long as
/// `password-cache-expiry` says, and returns whether it was kept: not while
/// `password-cache` is nil.
fn add_to_cache<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    key: &str,
    secret: Secret,
) -> bool {
    if !password::cache_enabled(env) {
        return false;
    }
    let lifetime = password::cache_lifetime(env);
    ctx.passwords_mut(|cache| cache.add(key, secret, lifetime, Instant::now()));
    true
}

/// Returns the password the cache keeps under KEY, unless the cache is off.
fn from_cache<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    key: &str,
) -> Option<String> {
    if !password::cache_enabled(env) {
        return None;
    }
    ctx.passwords_mut(|cache| cache.get(key, Instant::now()))
}

/// Returns a cache key: a string or a symbol, or `None` for nil or nothing.
fn cache_key<B: BufferTrait>(value: Option<&ELispExp<B>>) -> Result<Option<String>, Failure<B>> {
    match value {
        None => Ok(None),
        Some(value) if value.is_nil() => Ok(None),
        other => args::name(other).map(Some),
    }
}

/// The keywords the cache's two readers take.
const PASSWORD_READ_KEYWORDS: &[&str] = &[":on-cancel"];

/// Calls ON-CONFIRM with the password the cache keeps under the key ARGS
/// give, or else asks for it, as `password-read` does; and with ADD, keeps
/// what is typed in the cache under the key, as `password-read-and-add`
/// does.
fn read_cached<B: BufferTrait>(
    name: &str,
    args: &[ELispExp<B>],
    env: &Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
    add: bool,
) -> Result<ELispExp<B>, Failure<B>> {
    let (positional, keywords) = args::with_keywords(name, args, 2, 3, PASSWORD_READ_KEYWORDS)?;
    let prompt = args::text(positional.first())?;
    let on_confirm = positional[1].clone();
    let key = cache_key(positional.get(2))?;
    if let Some(key) = &key
        && let Some(cached) = from_cache(ctx, env, key)
    {
        if on_confirm.is_truthy() {
            call_callable(&on_confirm, &[ELispExp::string(cached)], env.clone(), ctx)?;
        }
        return Ok(ELispExp::t());
    }
    let request = Request {
        prompt,
        confirm: false,
        default: None,
        on_cancel: keywords
            .get(":on-cancel")
            .cloned()
            .unwrap_or_else(ELispExp::nil),
        answer: Answer::Call {
            function: on_confirm,
            cache_key: key.filter(|_| add),
        },
    };
    open(ctx, env, Arc::new(request), None)?;
    Ok(ELispExp::t())
}

const PASSWORD_READ_DOC: &str = "(password-read PROMPT ON-CONFIRM &optional KEY &key \
         :on-cancel): Call ON-CONFIRM with the password the cache keeps under KEY, at once, or, \
         when it keeps none, ask for one as `read-passwd' does and call ON-CONFIRM with what is \
         typed. Return t. What is typed is not kept: `password-cache-add' keeps it, once it has \
         been seen to work, or `password-read-and-add' does both.\n\n\
         Example:\n\
         (password-read \"Password for the server: \" 'my-connect \"server\")";

primitive!(password_read, args, env, ctx, {
    read_cached("password-read", args, &env, ctx, false)
});

const PASSWORD_READ_AND_ADD_DOC: &str = "(password-read-and-add PROMPT ON-CONFIRM &optional KEY \
         &key :on-cancel): As `password-read', and keep what is typed in the cache under KEY \
         before ON-CONFIRM is called, when KEY is not nil and `password-cache' is on.";

primitive!(password_read_and_add, args, env, ctx, {
    read_cached("password-read-and-add", args, &env, ctx, true)
});

const PASSWORD_READ_FROM_CACHE_DOC: &str = "(password-read-from-cache KEY): Return the password \
         the cache keeps under KEY, or nil when it keeps none, it has expired or `password-cache' \
         is nil.";

primitive!(password_read_from_cache, args, env, ctx, {
    exact_arity(args, 1)?;
    Ok(match cache_key(args.first())? {
        Some(key) => from_cache(ctx, &env, &key).map_or_else(ELispExp::nil, ELispExp::string),
        None => ELispExp::nil(),
    })
});

const PASSWORD_IN_CACHE_P_DOC: &str = "(password-in-cache-p KEY): Return t when the cache keeps a \
         password under KEY that has not expired, and nil otherwise.";

primitive!(password_in_cache_p, args, env, ctx, {
    exact_arity(args, 1)?;
    let Some(key) = cache_key(args.first())? else {
        return Ok(ELispExp::nil());
    };
    Ok(ELispExp::boolean(
        password::cache_enabled(&env)
            && ctx.passwords_mut(|cache| cache.contains(&key, Instant::now())),
    ))
});

const PASSWORD_CACHE_ADD_DOC: &str = "(password-cache-add KEY PASSWORD): Keep PASSWORD in the \
         cache under KEY, in place of what was kept there, for `password-cache-expiry' seconds. \
         Return t, or nil, keeping nothing, while `password-cache' is nil.\n\n\
         Example:\n\
         (password-cache-add \"server\" password)";

primitive!(password_cache_add, args, env, ctx, {
    exact_arity(args, 2)?;
    let key = args::name(args.first())?;
    let mut text = args::text(args.get(1))?;
    let kept = add_to_cache(ctx, &env, &key, Secret::from_text(&text));
    password::wipe_string(&mut text);
    Ok(ELispExp::boolean(kept))
});

const PASSWORD_CACHE_REMOVE_DOC: &str = "(password-cache-remove KEY): Forget the password the \
         cache keeps under KEY, as one that did not work should be. Return t when there was \
         one.";

primitive!(password_cache_remove, args, _env, ctx, {
    exact_arity(args, 1)?;
    let key = args::name(args.first())?;
    Ok(ELispExp::boolean(
        ctx.passwords_mut(|cache| cache.remove(&key)),
    ))
});

const PASSWORD_RESET_DOC: &str = "(password-reset): Forget every password the cache keeps, and \
         say how many there were.";

primitive!(password_reset, _args, _env, ctx, {
    let count = ctx.passwords_mut(|cache| cache.clear());
    ctx.set_echo_message(&match count {
        0 => "The password cache was empty".to_string(),
        1 => "1 password forgotten".to_string(),
        n => format!("{n} passwords forgotten"),
    });
    Ok(ELispExp::number(count as f64))
});

/// Registers the primitives that read and keep passwords.
///
/// Called by [`super::install_primitives`]. Only `password-reset` is a
/// command: the others answer a prompt that is open, or are asked by code
/// that needs a password, and `M-x` would offer them where there is nothing
/// to ask about.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("read-passwd", read_passwd, READ_PASSWD_DOC);
    into.function(
        "read-passwd-self-insert",
        read_passwd_self_insert,
        READ_PASSWD_SELF_INSERT_DOC,
    );
    into.function(
        "read-passwd-delete-backward-char",
        read_passwd_delete_backward_char,
        READ_PASSWD_DELETE_BACKWARD_CHAR_DOC,
    );
    into.function(
        "read-passwd-delete-char",
        read_passwd_delete_char,
        READ_PASSWD_DELETE_CHAR_DOC,
    );
    into.function(
        "read-passwd-kill-line",
        read_passwd_kill_line,
        READ_PASSWD_KILL_LINE_DOC,
    );
    into.function(
        "read-passwd-erase",
        read_passwd_erase,
        READ_PASSWD_ERASE_DOC,
    );
    into.function("read-passwd-yank", read_passwd_yank, READ_PASSWD_YANK_DOC);
    into.function(
        "read-passwd-toggle-visibility",
        read_passwd_toggle_visibility,
        READ_PASSWD_TOGGLE_VISIBILITY_DOC,
    );
    into.function(
        "read-passwd-confirm",
        read_passwd_confirm,
        READ_PASSWD_CONFIRM_DOC,
    );
    into.function(
        "process-send-password",
        process_send_password,
        PROCESS_SEND_PASSWORD_DOC,
    );
    into.function("password-read", password_read, PASSWORD_READ_DOC);
    into.function(
        "password-read-and-add",
        password_read_and_add,
        PASSWORD_READ_AND_ADD_DOC,
    );
    into.function(
        "password-read-from-cache",
        password_read_from_cache,
        PASSWORD_READ_FROM_CACHE_DOC,
    );
    into.function(
        "password-in-cache-p",
        password_in_cache_p,
        PASSWORD_IN_CACHE_P_DOC,
    );
    into.function(
        "password-cache-add",
        password_cache_add,
        PASSWORD_CACHE_ADD_DOC,
    );
    into.function(
        "password-cache-remove",
        password_cache_remove,
        PASSWORD_CACHE_REMOVE_DOC,
    );
    into.command("password-reset", password_reset, &[], PASSWORD_RESET_DOC);
}
