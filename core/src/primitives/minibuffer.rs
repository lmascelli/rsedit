//! The minibuffer from Lisp: opening a prompt, which takes what to do with
//! the answer as a callback, answering it, cancelling it, completing in it,
//! and walking its history. What the prompt holds and which history it files
//! under are [`crate::feature::minibuffer`], with `minibuffer-mode`.
//!
//! Design: `doc/architettura-editor.typ, <minibuffer>`.
use super::*;
use crate::editor::{
    COMPLETION_INVALIDATED_FUNCTION, COMPLETION_READ_FUNCTION, DEFAULT_MINIBUFFER_HEIGHT,
    DEFAULT_MINIBUFFER_WIDTH, MINIBUFFER_HEIGHT, MINIBUFFER_READ_FUNCTION, MINIBUFFER_WIDTH,
};
use crate::feature::minibuffer;
use crate::managers::Recalled;
use risp::call_callable;
use std::sync::Arc;

const MINIBUFFER_CLEANUP_DOC: &str = "(minibuffer-cleanup): Put the prompt's state back to no \
         prompt open, and make the buffer that was current before the prompt current again. Run \
         when the prompt's buffer closes, however it closed; not called directly.";

primitive!(minibuffer_cleanup_primitive, _args, env, ctx, {
    minibuffer::reset_prompt_state(&env, ctx);
    Ok(ELispExp::nil())
});

const MINIBUFFER_CONFIRM_DOC: &str = "(minibuffer-confirm): Close the prompt, remember what was \
         typed in its history, and call its ON-CONFIRM with the text. Return in a prompt.";

primitive!(minibuffer_confirm, _args, env, ctx, {
    let input = ctx.with_current_buffer(|buf| buf.text.to_string());
    let on_confirm = env.get_variable("*minibuffer-on-confirm*");

    // Remembered before the prompt closes, because closing it runs
    // `minibuffer-cleanup`, which is where the key is forgotten.
    let key = minibuffer::history_key(&env);
    let limit = minibuffer::history_length(&env);
    if !key.is_empty() {
        ctx.history_mut(|history| history.remember(&key, &input, limit));
    }

    ctx.close_buffer("*Minibuffer*", &env);

    if let Some(on_confirm) = on_confirm {
        if on_confirm.is_truthy() {
            call_callable(&on_confirm, &[ELispExp::string(input)], env.clone(), ctx)?;
        }
    }
    Ok(ELispExp::nil())
});

const MINIBUFFER_CANCEL_DOC: &str = "(minibuffer-cancel): Close the prompt and call its \
         ON-CANCEL, when it has one, with no arguments. Escape in a prompt.";

primitive!(minibuffer_cancel, _args, env, ctx, {
    let on_cancel = env.get_variable("*minibuffer-on-cancel*");

    ctx.close_buffer("*Minibuffer*", &env);

    if let Some(on_cancel) = on_cancel {
        if on_cancel.is_truthy() {
            call_callable(&on_cancel, &[], env.clone(), ctx)?;
        }
    }
    Ok(ELispExp::nil())
});

const MINIBUFFER_COMPLETE_DOC: &str = "(minibuffer-complete): Complete what is typed in the \
         prompt. Its ON-CHANGE is asked for the candidates, once for each different text. When \
         `*completion-read-function*' names a function, that function is given them all; \
         otherwise the first is put in the prompt, and each Tab after it the next. Tab in a \
         prompt.";

const MINIBUFFER_CHOOSE_COMPLETION_DOC: &str = "(minibuffer-choose-completion VALUE): Put VALUE \
         in the prompt in place of what is typed, and return VALUE. The function \
         `*completion-read-function*' is given to call with the candidate chosen at a prompt.";

primitive!(minibuffer_choose_completion, args, _env, ctx, {
    let Some(ELispExp::String(value)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    minibuffer::set_minibuffer_content(ctx, value);
    Ok(args[0].clone())
});

/// The input the candidate list in `*minibuffer-completions*` was computed
/// for. See [`minibuffer::MINIBUFFER_COMPLETIONS_FOR`].
const ASKED_ABOUT: &str = minibuffer::MINIBUFFER_COMPLETIONS_FOR;

primitive!(minibuffer_complete, _args, env, ctx, {
    let current = ctx.with_current_buffer(|buf| buf.text.to_string());

    let completions = env.get_variable("*minibuffer-completions*");
    let index = match env.get_variable("*minibuffer-completion-index*") {
        Some(ELispExp::Number(n)) => n as usize,
        _ => 0,
    };

    // While the prompt shows one of the candidates offered last, Tab puts in
    // the next, round the end; at the first Tab, or after typing, ON-CHANGE
    // is asked for new ones.
    let items: Vec<ELispExp<B>> = completions
        .as_ref()
        .map(|list| list.iter().collect())
        .unwrap_or_default();

    // A presenter takes the whole job: it is handed the candidates and the way
    // to report a choice, and nothing below runs. Asked for *before* the
    // cycling state is consulted, because a presenter has no use for an index
    // into a list it is about to show all of.
    if let Some(present) = env.get_variable(COMPLETION_READ_FUNCTION)
        && present.is_truthy()
    {
        // Asked again only when the input has changed: a second Tab cannot
        // change the answer, and a candidate function may be costly, as the
        // manual's is, which lists every page installed.
        let asked_about = match env.get_variable(ASKED_ABOUT) {
            Some(ELispExp::String(text)) => Some(text.to_string()),
            _ => None,
        };
        let known = env
            .get_variable("*minibuffer-completions*")
            .filter(|_| asked_about.as_deref() == Some(current.as_str()));
        let candidates = match known {
            Some(known) => known,
            None => {
                let fresh = match env.get_variable("*minibuffer-on-change*") {
                    Some(on_change) if on_change.is_truthy() => call_callable(
                        &on_change,
                        &[ELispExp::string(current.clone())],
                        env.clone(),
                        ctx,
                    )?,
                    _ => ELispExp::nil(),
                };
                minibuffer::setq(&env, ASKED_ABOUT, ELispExp::string(current.clone()));
                fresh
            }
        };
        minibuffer::setq(&env, "*minibuffer-completions*", candidates.clone());
        call_callable(
            &present,
            &[
                candidates,
                ELispExp::symbol("minibuffer-choose-completion".into()),
            ],
            env.clone(),
            ctx,
        )?;
        return Ok(ELispExp::nil());
    }

    let still_cycling = !items.is_empty()
        && matches!(items.get(index), Some(ELispExp::String(s)) if s.as_str() == current);

    if still_cycling {
        let next_index = (index + 1) % items.len();
        minibuffer::setq(
            &env,
            "*minibuffer-completion-index*",
            ELispExp::number(next_index as f64),
        );
        if let Some(ELispExp::String(s)) = items.get(next_index) {
            minibuffer::set_minibuffer_content(ctx, s.as_str());
        }
    } else if let Some(on_change) = env.get_variable("*minibuffer-on-change*") {
        if on_change.is_truthy() {
            let candidates =
                call_callable(&on_change, &[ELispExp::string(current)], env.clone(), ctx)?;
            minibuffer::setq(&env, "*minibuffer-completions*", candidates.clone());
            minibuffer::setq(&env, "*minibuffer-completion-index*", ELispExp::number(0.0));
            if let Some(ELispExp::String(first)) = candidates.iter().next() {
                minibuffer::set_minibuffer_content(ctx, &first);
            }
        }
    }

    Ok(ELispExp::nil())
});

const HISTORY_PREVIOUS_DOC: &str = "(history-previous): Put in the prompt the answer given at it \
         before the one it shows, keeping what was half typed for `history-next' to bring back. \
         Return t, or nil at the oldest. C-p, M-p and the up arrow in a prompt; while a \
         completion strip is showing, C-p and the up arrow move in the strip instead.";

const HISTORY_NEXT_DOC: &str = "(history-next): Put in the prompt the answer given at it after \
         the one it shows, and past the newest what was being typed. Return t, or nil when the \
         history is not being walked. C-n, M-n and the down arrow in a prompt.";

/// Takes one step through the prompt's history, back or on, and puts what it
/// finds in the prompt: t, or nil when there is no step to take.
fn step_history<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    backwards: bool,
) -> ELispExp<B> {
    let key = minibuffer::history_key(env);
    if key.is_empty() {
        return ELispExp::nil();
    }
    // Read before the walk is asked, because the walk needs it: the first step
    // back stashes what is in the prompt now, and by the time the answer comes
    // back it will have been overwritten.
    let current = ctx.with_current_buffer(|buf| buf.text.to_string());
    let recalled: Recalled = ctx.history_mut(|history| {
        if backwards {
            history.previous(&key, &current)
        } else {
            history.next(&key)
        }
    });
    match recalled {
        Some(text) => {
            minibuffer::set_minibuffer_content(ctx, &text);
            ELispExp::t()
        }
        // Left exactly as it is. Rewriting the prompt with a copy of itself
        // would look the same as a step, and "you are at the end" is the one
        // thing the key has to be able to say.
        None => ELispExp::nil(),
    }
}

primitive!(history_previous, _args, env, ctx, {
    Ok(step_history(ctx, &env, true))
});

primitive!(history_next, _args, env, ctx, {
    Ok(step_history(ctx, &env, false))
});

const MINIBUFFER_HISTORY_DOC: &str = "(minibuffer-history KEY): Return what has been typed at the \
         prompt whose history is KEY, newest first, as a list of strings. A prompt's history is \
         its PROMPT, unless `minibuffer-read' was given another KEY.\n\n\
         Example:\n\
         (minibuffer-history \"M-x\")";

primitive!(minibuffer_history, args, _env, ctx, {
    let Some(ELispExp::String(key) | ELispExp::Symbol(key)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String or Symbol".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    Ok(ELispExp::proper_list(
        ctx.history(|history| history.entries(key))
            .into_iter()
            .map(ELispExp::string)
            .collect(),
    ))
});

const CLEAR_MINIBUFFER_HISTORY_DOC: &str = "(clear-minibuffer-history KEY): Forget what has been \
         typed at the prompt whose history is KEY. Return t when there was something to forget.";

primitive!(clear_minibuffer_history, args, _env, ctx, {
    let Some(ELispExp::String(key) | ELispExp::Symbol(key)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String or Symbol".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    Ok(ELispExp::boolean(
        ctx.history_mut(|history| history.forget(key)),
    ))
});

const DEFAULT_MINIBUFFER_PROMPT_DOC: &str = "(default-minibuffer-prompt PROMPT ON-CONFIRM \
         ON-CHANGE ON-CANCEL &optional MODE KEY): The prompt `minibuffer-read' opens unless \
         `*minibuffer-read-function*' names another: a window in the middle of the frame titled \
         PROMPT, `minibuffer-width' columns by `minibuffer-height' rows, or what the frame can \
         hold. The arguments are those of `minibuffer-read'; not called directly.";

primitive!(default_minibuffer_prompt, args, env, ctx, {
    if !(4..=6).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 4,
            got: args.len(),
        });
    }
    // TODO(MIN-4): a MODE of nil is taken as the mode named nil, which has
    // none of these keys.
    // The mode the prompt's buffer opens in. Defaults to `minibuffer-mode`,
    // which is what gives it Return, Escape and Tab; a caller that wants the
    // prompt to behave differently -- incremental search, whose keys mean
    // something else entirely -- names its own mode instead of reimplementing
    // the prompt.
    let mode = match args.get(4) {
        Some(ELispExp::String(name)) | Some(ELispExp::Symbol(name)) => name.to_string(),
        _ => "minibuffer-mode".to_string(),
    };
    let title = if let ELispExp::String(s) = &args[0] {
        Some(s.to_string())
    } else {
        None
    };

    minibuffer::setq(
        &env,
        "*minibuffer-previous-buffer*",
        ELispExp::string(ctx.get_current_buffer_name()),
    );
    minibuffer::setq(&env, "*minibuffer-on-confirm*", args[1].clone());
    minibuffer::setq(&env, "*minibuffer-on-change*", args[2].clone());
    minibuffer::setq(&env, "*minibuffer-on-cancel*", args[3].clone());
    minibuffer::setq(&env, "*minibuffer-completions*", ELispExp::nil());
    minibuffer::setq(&env, ASKED_ABOUT, ELispExp::nil());
    minibuffer::setq(&env, "*minibuffer-completion-index*", ELispExp::number(0.0));
    // Which ring M-p and M-n will walk. The prompt's own text unless the
    // caller named one, so every prompt in the editor has a history without a
    // single caller having to ask for it.
    minibuffer::setq(
        &env,
        minibuffer::MINIBUFFER_HISTORY_KEY,
        match args.get(5) {
            Some(ELispExp::String(key)) | Some(ELispExp::Symbol(key)) if !key.is_empty() => {
                ELispExp::string(key.to_string())
            }
            _ => ELispExp::string(title.clone().unwrap_or_default()),
        },
    );
    // A prompt opening is not a continuation of the last one's walk.
    ctx.history_mut(|history| history.end_walk());

    let frame_height = match env.get_variable("frame-height") {
        Some(ELispExp::Number(n)) => n,
        _ => return Err(EvalError::UnboundVariable("frame-height".into())),
    };
    let frame_width = match env.get_variable("frame-width") {
        Some(ELispExp::Number(n)) => n,
        _ => return Err(EvalError::UnboundVariable("frame-width".into())),
    };

    let (x, y, width, height) = centred(
        frame_width as usize,
        frame_height as usize,
        sized(&env, MINIBUFFER_WIDTH, DEFAULT_MINIBUFFER_WIDTH),
        sized(&env, MINIBUFFER_HEIGHT, DEFAULT_MINIBUFFER_HEIGHT),
    );

    // TODO(MIN-1): every prompt opens the one buffer, which a second prompt
    // open at the same time shares.
    ctx.open_floating_window("*Minibuffer*", x, y, width, height, title, Some(mode), &env);

    Ok(ELispExp::t())
});

/// Returns the size the variable NAME asks for, or DEFAULT when it says
/// nothing usable, as nil, a string or a negative does: the prompt must open.
fn sized<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>, name: &str, default: f64) -> usize {
    match env.get_variable(name) {
        Some(ELispExp::Number(n)) if n.is_finite() && n >= 1.0 => n as usize,
        _ => default as usize,
    }
}

/// Returns a rectangle of WIDTH by HEIGHT in the middle of a frame of
/// FRAME_WIDTH by FRAME_HEIGHT, made smaller where it would not fit, with a
/// column of margin on each side.
fn centred(
    frame_width: usize,
    frame_height: usize,
    width: usize,
    height: usize,
) -> (isize, isize, usize, usize) {
    // The echo area owns the bottom row, so the space a prompt may be centred
    // in is one row shorter than the frame. Centring in the whole of it would
    // put a tall prompt's last row underneath a message.
    let available_height = frame_height.saturating_sub(1);
    let width = width.min(frame_width.saturating_sub(2)).max(1);
    let height = height.min(available_height).max(1);
    let x = (frame_width.saturating_sub(width) / 2) as isize;
    let y = (available_height.saturating_sub(height) / 2) as isize;
    (x, y, width, height)
}

const COMPLETION_INVALIDATE_DOC: &str = "(completion-invalidate): Say that the completion \
         candidates may have changed, for a list still being built in the background. The next \
         Tab asks for them again, and the function `*completion-invalidated-function*' names, \
         when there is one, is called with no arguments so that a list on screen can be shown \
         again. Harmless with no prompt open.";

primitive!(completion_invalidate, _args, env, ctx, {
    // Forgotten rather than recomputed: recomputing here would call the
    // candidate function whether or not anybody was going to look, and the
    // whole reason this exists is that the candidate function is expensive.
    minibuffer::setq(&env, ASKED_ABOUT, ELispExp::nil());
    if let Some(presenter) = env.get_variable(COMPLETION_INVALIDATED_FUNCTION)
        && presenter.is_truthy()
    {
        call_callable(&presenter, &[], env.clone(), ctx)?;
    }
    Ok(ELispExp::nil())
});

const MINIBUFFER_READ_DOC: &str = "(minibuffer-read PROMPT ON-CONFIRM ON-CHANGE ON-CANCEL \
         &optional MODE KEY): Open a prompt titled PROMPT, and return at once. ON-CONFIRM is \
         called with the text typed when Return is pressed, and ON-CANCEL, when not nil, with no \
         arguments when Escape is. ON-CHANGE, when not nil, is called with the text when Tab is \
         pressed and returns the candidates, a list of strings. MODE is the prompt's major mode, \
         `minibuffer-mode' by default, and KEY names its history, PROMPT by default. Refused in a \
         background job.\n\n\
         Example:\n\
         (minibuffer-read \"Name:\" (lambda (name) (message \"Hello %s\" name)) nil nil)";

primitive!(minibuffer_read, args, env, ctx, {
    if !(4..=6).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 4,
            got: args.len(),
        });
    }
    // Refused in a background job, where it can still be an error: its prompt
    // would be a window over a command nobody started, answered by a key
    // meant for whatever the user was doing.
    if crate::editor::in_worker() {
        return Err(EvalError::RuntimeMessage(
            "minibuffer-read: a background worker cannot open a prompt".into(),
        ));
    }
    let read_fn = env
        .get_variable(MINIBUFFER_READ_FUNCTION)
        .ok_or_else(|| EvalError::UnboundVariable(MINIBUFFER_READ_FUNCTION.into()))?;
    call_callable(&read_fn, args, env.clone(), ctx)
});

/// Registers the primitives of the minibuffer.
///
/// Called by [`super::install_primitives`]. Only the two history commands are
/// commands: the rest answer a prompt that is open, and `M-x` would offer
/// them where there is none.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function(
        "minibuffer-cleanup",
        minibuffer_cleanup_primitive,
        MINIBUFFER_CLEANUP_DOC,
    );
    into.function(
        "minibuffer-confirm",
        minibuffer_confirm,
        MINIBUFFER_CONFIRM_DOC,
    );
    into.function(
        "minibuffer-cancel",
        minibuffer_cancel,
        MINIBUFFER_CANCEL_DOC,
    );
    into.function(
        "minibuffer-complete",
        minibuffer_complete,
        MINIBUFFER_COMPLETE_DOC,
    );
    into.function(
        "minibuffer-choose-completion",
        minibuffer_choose_completion,
        MINIBUFFER_CHOOSE_COMPLETION_DOC,
    );
    into.function(
        "default-minibuffer-prompt",
        default_minibuffer_prompt,
        DEFAULT_MINIBUFFER_PROMPT_DOC,
    );
    into.function("minibuffer-read", minibuffer_read, MINIBUFFER_READ_DOC);
    into.function(
        "completion-invalidate",
        completion_invalidate,
        COMPLETION_INVALIDATE_DOC,
    );
    into.function(
        "minibuffer-history",
        minibuffer_history,
        MINIBUFFER_HISTORY_DOC,
    );
    into.function(
        "clear-minibuffer-history",
        clear_minibuffer_history,
        CLEAR_MINIBUFFER_HISTORY_DOC,
    );

    // Commands as well as functions, so that `M-x' reaches them and so that
    // Lisp can rebind them by name. They take no arguments, so there is
    // nothing to collect.
    into.command(
        "history-previous",
        history_previous,
        &[],
        HISTORY_PREVIOUS_DOC,
    );
    into.command("history-next", history_next, &[], HISTORY_NEXT_DOC);
}
