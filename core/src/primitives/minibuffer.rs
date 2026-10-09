//! The minibuffer, as Lisp sees it.
//!
//! Twelve primitives: opening a prompt, answering it, cancelling it,
//! completing in it, and walking its history. The prompt's mechanics -- what
//! its contents are, which history ring it files under, what "no prompt open"
//! means -- are [`crate::feature::minibuffer`], and so is `minibuffer-mode`.
//!
//! # Why every one of these takes its answer as a callback
//!
//! The editor never blocks. A prompt opens, the command that opened it
//! returns, and the answer arrives later -- in another command, when a key is
//! pressed. So reading a line cannot *return* the line, and what it can do is
//! say what to do with it.
use super::*;
use crate::editor::{
    DEFAULT_MINIBUFFER_HEIGHT, DEFAULT_MINIBUFFER_WIDTH, MINIBUFFER_HEIGHT, MINIBUFFER_WIDTH,
};
use crate::feature::minibuffer;
use crate::lisp::call_callable;
use crate::managers::Recalled;
use std::sync::Arc;

const MINIBUFFER_CLEANUP_DOC: &str = "(minibuffer-cleanup): Runs via `minibuffer-mode's \
         after-close-hook once the minibuffer buffer closes, however it \
         closed (confirm, cancel, or otherwise): switches back to \
         *minibuffer-previous-buffer* (the buffer that was current before \
         the minibuffer opened -- `close-buffer` alone only restores window \
         *focus*, not which buffer is \"current\", so this is done \
         explicitly), then clears *minibuffer-on-confirm*, \
         *minibuffer-on-change*, *minibuffer-on-cancel*, \
         *minibuffer-previous-buffer*, *minibuffer-completions* and \
         *minibuffer-completion-index*, so the next prompt starts from a \
         clean slate.";

primitive!(minibuffer_cleanup_primitive, _args, env, ctx, {
    minibuffer::reset_prompt_state(&env, ctx);
    Ok(ELispExp::nil())
});

const MINIBUFFER_CONFIRM_DOC: &str = "(minibuffer-confirm): Called when the user presses Return in the minibuffer. \
         Closes the minibuffer, then -- if *minibuffer-on-confirm* is set -- \
         calls it with the minibuffer's final contents as its one argument.";

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

const MINIBUFFER_CANCEL_DOC: &str = "(minibuffer-cancel): Called when the user presses Escape in the minibuffer. \
         Closes the minibuffer, then -- if *minibuffer-on-cancel* is set -- \
         calls it with no arguments.";

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

const MINIBUFFER_COMPLETE_DOC: &str = "(minibuffer-complete): Called when the user presses Tab in the minibuffer. \
         The first press after a change to the input calls \
         *minibuffer-on-change* with the current input to compute completion \
         candidates and shows the first one; further presses (as long as the \
         input hasn't changed since) cycle through the rest.\n\n\
         The candidate function is asked once per distinct input, whichever way the candidates \
         are presented. Pressing Tab again cannot change the answer, and a candidate function \
         may be expensive -- the manual's walks every page installed -- so asking again would \
         be paying for the same list twice, on the thread that draws.\n\n\
         If *completion-read-function* is set, it is called instead of cycling, with the \
         candidate list and the symbol `minibuffer-choose-completion' -- which it calls with \
         whichever candidate the user picked. Setting that variable is how a module replaces \
         the way completions are presented without this, or any of its callers, knowing that \
         it did. Unset, the cycling below is all there is, so nothing here depends on such a \
         module existing.";

const MINIBUFFER_CHOOSE_COMPLETION_DOC: &str = "(minibuffer-choose-completion VALUE): Replace the \
         minibuffer's contents with VALUE. Returns VALUE.\n\n\
         This is the symbol handed to *completion-read-function* as the thing to call when the \
         user picks a candidate. A presenter is given it rather than being told to write to the \
         minibuffer itself, so that it needs to know nothing about where the completion came \
         from -- the same presenter serves a prompt, and would serve a buffer.";

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

    // Still showing one of the last candidates we offered -> advance to
    // the next one, wrapping around. Otherwise -- either the very first
    // Tab press, or the user typed something since the last completion --
    // ask *minibuffer-on-change* for a fresh candidate list.
    let items: Vec<ELispExp<B>> = completions
        .as_ref()
        .map(|list| list.iter().collect())
        .unwrap_or_default();

    // A presenter takes the whole job: it is handed the candidates and the way
    // to report a choice, and nothing below runs. Asked for *before* the
    // cycling state is consulted, because a presenter has no use for an index
    // into a list it is about to show all of.
    if let Some(present) = env.get_variable("*completion-read-function*")
        && present.is_truthy()
    {
        // Asked again only when the input has changed since the last time.
        //
        // Pressing Tab a second time cannot change what the answer is, and a
        // candidate function may be expensive -- the manual's lists every page
        // installed. This used to ask on every press, so holding Tab down
        // walked a directory once per keystroke, on the thread that draws.
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

const HISTORY_PREVIOUS_DOC: &str = "(history-previous): Replace the minibuffer's contents with the previous thing \
         typed at this prompt. Bound in the minibuffer to C-p, M-p and the up arrow.\n\n\
         While a completion strip is showing, those keys move through the candidates instead: \
         the strip installs a transient keymap, and a transient keymap is consulted before \
         every other. So this is reached exactly when a prompt is open and no completion is \
         being offered, without either of them having to know about the other.\n\n\
         The first step remembers whatever was half-typed, and walking forward past the newest \
         entry brings it back -- looking through the history costs nothing, so there is no reason \
         not to look.\n\n\
         Does nothing at the oldest entry, and nothing at a prompt with no history.";

const HISTORY_NEXT_DOC: &str = "(history-next): The other way through the history from \
         `history-previous'. Bound in the minibuffer to C-n, M-n and the down arrow.\n\n\
         One step forward from the newest entry restores what was being typed when the walk \
         started.";

/// One step through the history, in whichever direction.
///
/// Both keys do the same three things -- ask the ring, put the answer in the
/// buffer, leave the walk where it now is -- and differ only in which question
/// they ask.
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

const MINIBUFFER_HISTORY_DOC: &str = "(minibuffer-history KEY): What has been typed at the prompt filed under KEY, newest \
         first, as a list of strings. KEY is the prompt's own text unless the prompt named \
         something else.";

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

const CLEAR_MINIBUFFER_HISTORY_DOC: &str = "(clear-minibuffer-history KEY): Forget what has been typed at the prompt filed under KEY. \
         Returns t if there was anything to forget.";

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

const DEFAULT_MINIBUFFER_PROMPT_DOC: &str = "(default-minibuffer-prompt PROMPT ON-CONFIRM ON-CHANGE ON-CANCEL \
         &optional MODE): The built-in *minibuffer-read-function*: a floating window in the \
         middle of the frame, titled PROMPT, in major mode MODE (default \
         `minibuffer-mode'). Not normally called directly -- see `minibuffer-read'.\n\n\
         `minibuffer-width' and `minibuffer-height' say how large it is, in columns and rows \
         including its border; both are clamped to what the frame can hold. It used to be \
         docked along the bottom, which put it exactly where anything else wanting a strip \
         there -- a list of completions -- also goes.\n\n\
         Example:\n\
         (setq minibuffer-width 40)   ; a narrower prompt";

primitive!(default_minibuffer_prompt, args, env, ctx, {
    if !(4..=6).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 4,
            got: args.len(),
        });
    }
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

    ctx.open_floating_window("*Minibuffer*", x, y, width, height, title, Some(mode));

    Ok(ELispExp::t())
});

/// A size asked for in Lisp, or DEFAULT when the variable says nothing usable.
///
/// A nonsense value -- nil, a string, a negative -- falls back rather than
/// signalling. Getting a prompt you did not expect is recoverable; being unable
/// to open one at all means `M-x` stops working, and the way out of that is
/// also a prompt.
fn sized<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>, name: &str, default: f64) -> usize {
    match env.get_variable(name) {
        Some(ELispExp::Number(n)) if n.is_finite() && n >= 1.0 => n as usize,
        _ => default as usize,
    }
}

/// A rectangle of WIDTH by HEIGHT in the middle of a frame, clamped to fit.
///
/// # Why the middle
///
/// Because the bottom is taken. A prompt docked along the bottom edge sits
/// exactly where anything else wanting a strip there goes -- a list of
/// completions, and later a compilation log -- and two things drawn in one
/// place is not a layout, it is a collision. The middle is the one region
/// nothing else claims.
///
/// # Why clamped rather than refused
///
/// A prompt must always be openable: it is how `M-x` works, and how the user
/// would run whatever command fixed the setting. So a width larger than the
/// terminal becomes the terminal's width rather than an error, and one
/// character of margin is kept on each side so the border does not merge with
/// the frame's own edge.
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

/// The function a module registers to be told that candidates have changed.
///
/// The same shape as `*completion-read-function*`, which is the established
/// way a presenter attaches itself: a module that shows candidates sets this,
/// a module whose candidates are built in the background calls
/// `completion-invalidate`, and neither has to have been loaded for the other
/// to work.
const COMPLETION_INVALIDATED: &str = "*completion-invalidated-function*";

const COMPLETION_INVALIDATE_DOC: &str = "(completion-invalidate): Say that the completion \
         candidates may have changed, whoever is showing them.\n\n\
         For a module whose candidate list is built in the background -- the manual's list of \
         pages is the one in the tree. The list it handed out a moment ago was the truth at the \
         time and is now short, and there is no other way for whatever is showing it to find \
         that out: nothing was typed.\n\n\
         Two things happen. The answer remembered for the current input is forgotten, so the \
         next Tab asks the candidate function again rather than repeating what it was told. And \
         if `*completion-invalidated-function*' is set -- a presenter registers itself there, as \
         it does with `*completion-read-function*' -- it is called with no arguments, so a \
         list already on screen can redraw with what there is now.\n\n\
         Safe with no prompt open and with no presenter loaded: it is a signal, and nobody \
         having to hear it is a normal state of affairs.\n\n\
         This is the call a background job's ON-PROGRESS makes -- see `background-call'. It is \
         made on the thread that runs commands, which is what makes redrawing from it safe.";

primitive!(completion_invalidate, _args, env, ctx, {
    // Forgotten rather than recomputed: recomputing here would call the
    // candidate function whether or not anybody was going to look, and the
    // whole reason this exists is that the candidate function is expensive.
    minibuffer::setq(&env, ASKED_ABOUT, ELispExp::nil());
    if let Some(presenter) = env.get_variable(COMPLETION_INVALIDATED)
        && presenter.is_truthy()
    {
        call_callable(&presenter, &[], env.clone(), ctx)?;
    }
    Ok(ELispExp::nil())
});

const MINIBUFFER_READ_DOC: &str = "(minibuffer-read PROMPT ON-CONFIRM ON-CHANGE ON-CANCEL &optional MODE): \
         Read a line of input from the user via a minibuffer prompt. PROMPT \
         is shown as the window's title. ON-CONFIRM is called with the final \
         input string when the user presses Return. ON-CHANGE, if non-nil, \
         is called with the current input string whenever the user presses \
         Tab, and must return a list of completion candidate strings; \
         repeated Tab presses cycle through them. ON-CANCEL, if non-nil, is \
         called with no arguments when the user presses Escape.\n\n\
         MODE, if given, is the major mode the prompt's buffer opens in, and \
         therefore which keymap answers keys typed into it. It defaults to \
         `minibuffer-mode'. A prompt whose keys mean something other than \
         \"edit a line of text\" -- incremental search is the one in the tree \
         -- names its own mode here rather than reimplementing the prompt.\n\n\
         Which implementation actually runs is controlled by \
         *minibuffer-read-function* -- rebind it to replace the built-in \
         minibuffer with a custom implementation; every caller of \
         `minibuffer-read' picks up the change automatically.\n\n\
         Example:\n\
         (minibuffer-read \"Eval:\" 'my-on-confirm nil nil)";

primitive!(minibuffer_read, args, env, ctx, {
    if !(4..=6).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 4,
            got: args.len(),
        });
    }
    // A prompt is a conversation with whoever is at the keyboard, and a
    // background worker is not talking to them. One opened from a worker's
    // turn would be a window over a command nobody started, and the keystroke
    // that answered it would arrive in the middle of whatever the user was
    // actually doing -- so this is refused where it can still be reported as
    // an error, rather than half-working somewhere confusing.
    //
    // A worker that genuinely needs an answer asks for it the way anything
    // else asynchronous does: it leaves something for a command to find.
    if crate::background::worker::in_worker() {
        return Err(EvalError::RuntimeMessage(
            "minibuffer-read: a background worker cannot open a prompt".into(),
        ));
    }
    let read_fn = env
        .get_variable("*minibuffer-read-function*")
        .ok_or_else(|| EvalError::UnboundVariable("*minibuffer-read-function*".into()))?;
    call_callable(&read_fn, args, env.clone(), ctx)
});

/// Register this module's primitives: reading a line of input.
///
/// Called by [`super::install_primitives`]. These used to be registered by
/// `install_minibuffer`, beside the mode -- twelve `env.set_function` calls
/// spelled out in full, which is what a `Registry` is for.
///
/// Only the two history commands are commands: the rest answer a prompt that
/// is already open, and `M-x minibuffer-confirm` with nothing to confirm is
/// not an offer worth making.
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
