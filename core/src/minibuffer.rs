//! The built-in minibuffer: a small, always-available prompt for reading a
//! single line of input from the user, with optional Tab-completion.
//!
//! Unlike most editor behavior (which lives in `.lisp` files under
//! `core/lisp/` and can be freely redefined or omitted), the minibuffer's
//! read/confirm/cancel/complete mechanics are hardcoded here in Rust: too
//! much else (M-x, M-:, and eventually find-file prompts, search, ...)
//! depends on being able to read a line of input for it to be something a
//! missing or broken Lisp file could silently take out.
//!
//! The public entry point is `minibuffer-read`. What actually renders and
//! drives the prompt is decided by `*minibuffer-read-function*`, a Lisp
//! variable naming a function with the same signature as `minibuffer-read`
//! itself. It defaults to `default-minibuffer-prompt`, a small floating
//! window docked to the last few lines of the frame. Anything wanting a
//! fancier minibuffer (a real popup completion list, fuzzy matching, ...)
//! can rebind it -- every caller of `minibuffer-read` picks that up
//! automatically -- so *this* mechanism being hardcoded doesn't lock in the
//! *implementation* it happens to ship with.
use crate::{
    BufferTrait, ELispExp, EditorState,
    input::{KeyCode, KeyEvent},
    lisp::{Env, EvalError, call_callable},
    modes::MajorMode,
    primitive,
};
use std::sync::Arc;

/// Set NAME to VAL the way Lisp's `setq` special form does: update an
/// existing binding wherever it is up the scope chain if one exists,
/// otherwise declare it fresh in ENV's own scope. `Env::set_variable`
/// alone always declares fresh *locally* -- correct for a genuinely new
/// binding, but wrong for a global like `*minibuffer-on-confirm*` mutated
/// from inside a primitive that was itself called with some nested
/// per-call environment: it would create a shadow invisible to a later
/// read from a different frame, rather than updating the global.
fn setq<B: BufferTrait>(env: &Env<EditorState<B>>, name: &str, val: ELispExp<B>) {
    if !env.update_variable(name, val.clone()) {
        env.set_variable(name.to_string(), val);
    }
}

/// Replace the minibuffer buffer's contents with CONTENT, character by
/// character (mirroring what `self-insert` does per character, including
/// marking the buffer modified) after clearing it. Not a primitive --
/// purely an internal helper for `minibuffer-complete`'s Tab-cycling.
fn set_minibuffer_content<B: BufferTrait>(ctx: &EditorState<B>, content: &str) {
    let buf = ctx.get_current_buffer();
    ctx.mutate_buffer(buf, |buf| {
        while buf.text.cursor_pos() != (0, 0) {
            buf.text.delete();
        }
        for c in content.chars() {
            buf.text.insert(c);
            buf.is_modified = true;
        }
    });
}

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
    if let Some(ELispExp::String(previous)) = env.get_variable("*minibuffer-previous-buffer*") {
        ctx.switch_to_buffer(&previous);
    }
    setq(&env, "*minibuffer-on-confirm*", ELispExp::nil());
    setq(&env, "*minibuffer-on-change*", ELispExp::nil());
    setq(&env, "*minibuffer-on-cancel*", ELispExp::nil());
    setq(&env, "*minibuffer-previous-buffer*", ELispExp::nil());
    setq(&env, "*minibuffer-completions*", ELispExp::nil());
    setq(
        &env,
        "*minibuffer-completion-index*",
        ELispExp::number(0f64),
    );
    Ok(ELispExp::nil())
});

const MINIBUFFER_CONFIRM_DOC: &str = "(minibuffer-confirm): Called when the user presses Return in the minibuffer. \
         Closes the minibuffer, then -- if *minibuffer-on-confirm* is set -- \
         calls it with the minibuffer's final contents as its one argument.";

primitive!(minibuffer_confirm, _args, env, ctx, {
    let input = ctx
        .get_current_buffer()
        .read()
        .expect("Failed to acquire read lock on current buffer")
        .text
        .to_string();
    let on_confirm = env.get_variable("*minibuffer-on-confirm*");

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
         input hasn't changed since) cycle through the rest.";

primitive!(minibuffer_complete, _args, env, ctx, {
    let current = ctx
        .get_current_buffer()
        .read()
        .expect("Failed to acquire read lock on current buffer")
        .text
        .to_string();

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

    let still_cycling = !items.is_empty()
        && matches!(items.get(index), Some(ELispExp::String(s)) if s.as_str() == current);

    if still_cycling {
        let next_index = (index + 1) % items.len();
        setq(
            &env,
            "*minibuffer-completion-index*",
            ELispExp::number(next_index as f64),
        );
        if let Some(ELispExp::String(s)) = items.get(next_index) {
            set_minibuffer_content(ctx, s.as_str());
        }
    } else if let Some(on_change) = env.get_variable("*minibuffer-on-change*") {
        if on_change.is_truthy() {
            let candidates =
                call_callable(&on_change, &[ELispExp::string(current)], env.clone(), ctx)?;
            setq(&env, "*minibuffer-completions*", candidates.clone());
            setq(&env, "*minibuffer-completion-index*", ELispExp::number(0.0));
            if let Some(ELispExp::String(first)) = candidates.iter().next() {
                set_minibuffer_content(ctx, &first);
            }
        }
    }

    Ok(ELispExp::nil())
});

const DEFAULT_MINIBUFFER_PROMPT_DOC: &str = "(default-minibuffer-prompt PROMPT ON-CONFIRM ON-CHANGE ON-CANCEL): The \
         built-in *minibuffer-read-function*: a floating window docked to the \
         bottom 3 lines of the frame, titled PROMPT. Not normally called \
         directly -- see `minibuffer-read'.";

primitive!(default_minibuffer_prompt, args, env, ctx, {
    if args.len() != 4 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 4,
            got: args.len(),
        });
    }
    let title = if let ELispExp::String(s) = &args[0] {
        Some(s.to_string())
    } else {
        None
    };

    setq(
        &env,
        "*minibuffer-previous-buffer*",
        ELispExp::string(ctx.get_current_buffer_name()),
    );
    setq(&env, "*minibuffer-on-confirm*", args[1].clone());
    setq(&env, "*minibuffer-on-change*", args[2].clone());
    setq(&env, "*minibuffer-on-cancel*", args[3].clone());
    setq(&env, "*minibuffer-completions*", ELispExp::nil());
    setq(&env, "*minibuffer-completion-index*", ELispExp::number(0.0));

    let frame_height = match env.get_variable("frame-height") {
        Some(ELispExp::Number(n)) => n,
        _ => return Err(EvalError::UnboundVariable("frame-height".into())),
    };
    let frame_width = match env.get_variable("frame-width") {
        Some(ELispExp::Number(n)) => n,
        _ => return Err(EvalError::UnboundVariable("frame-width".into())),
    };

    ctx.open_floating_window(
        "*Minibuffer*",
        1,
        (frame_height - 4.0) as isize,
        (frame_width - 2.0) as usize,
        3,
        title,
        Some("minibuffer-mode".into()),
    );

    Ok(ELispExp::t())
});

const MINIBUFFER_READ_DOC: &str = "(minibuffer-read PROMPT ON-CONFIRM ON-CHANGE ON-CANCEL): \
         Read a line of input from the user via a minibuffer prompt. PROMPT \
         is shown as the window's title. ON-CONFIRM is called with the final \
         input string when the user presses Return. ON-CHANGE, if non-nil, \
         is called with the current input string whenever the user presses \
         Tab, and must return a list of completion candidate strings; \
         repeated Tab presses cycle through them. ON-CANCEL, if non-nil, is \
         called with no arguments when the user presses Escape.\n\n\
         Which implementation actually runs is controlled by \
         *minibuffer-read-function* -- rebind it to replace the built-in \
         minibuffer with a custom implementation; every caller of \
         `minibuffer-read' picks up the change automatically.\n\n\
         Example:\n\
         (minibuffer-read \"Eval:\" 'my-on-confirm nil nil)";

primitive!(minibuffer_read, args, env, ctx, {
    if args.len() != 4 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 4,
            got: args.len(),
        });
    }
    let read_fn = env
        .get_variable("*minibuffer-read-function*")
        .ok_or_else(|| EvalError::UnboundVariable("*minibuffer-read-function*".into()))?;
    call_callable(&read_fn, args, env.clone(), ctx)
});

pub fn install_minibuffer<B: BufferTrait>(
    editor_state: &EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
) {
    env.set_function(
        "minibuffer-cleanup".into(),
        ELispExp::primitive(
            minibuffer_cleanup_primitive,
            Some(MINIBUFFER_CLEANUP_DOC.into()),
        ),
    );
    env.set_function(
        "minibuffer-confirm".into(),
        ELispExp::primitive(minibuffer_confirm, Some(MINIBUFFER_CONFIRM_DOC.into())),
    );
    env.set_function(
        "minibuffer-cancel".into(),
        ELispExp::primitive(minibuffer_cancel, Some(MINIBUFFER_CANCEL_DOC.into())),
    );
    env.set_function(
        "minibuffer-complete".into(),
        ELispExp::primitive(minibuffer_complete, Some(MINIBUFFER_COMPLETE_DOC.into())),
    );
    env.set_function(
        "default-minibuffer-prompt".into(),
        ELispExp::primitive(
            default_minibuffer_prompt,
            Some(DEFAULT_MINIBUFFER_PROMPT_DOC.into()),
        ),
    );
    env.set_function(
        "minibuffer-read".into(),
        ELispExp::primitive(minibuffer_read, Some(MINIBUFFER_READ_DOC.into())),
    );

    // Names the function that actually implements `minibuffer-read`.
    // Rebind this (`(setq *minibuffer-read-function* 'my-own-prompt)`) to
    // replace the built-in minibuffer with a custom implementation; see
    // `minibuffer-read`'s docstring for the required signature.
    env.set_variable(
        "*minibuffer-read-function*".into(),
        ELispExp::symbol("default-minibuffer-prompt".into()),
    );

    let mut minibuffer_mode = MajorMode::new("minibuffer-mode");
    minibuffer_mode.keymaps.insert(
        KeyEvent::new(KeyCode::Enter),
        ELispExp::symbol("minibuffer-confirm".into()),
    );
    minibuffer_mode.keymaps.insert(
        KeyEvent::new(KeyCode::Esc),
        ELispExp::symbol("minibuffer-cancel".into()),
    );
    minibuffer_mode.keymaps.insert(
        KeyEvent::new(KeyCode::Tab),
        ELispExp::symbol("minibuffer-complete".into()),
    );
    minibuffer_mode.hooks.insert(
        "after-close-hook".into(),
        vec![ELispExp::symbol("minibuffer-cleanup".into())],
    );

    editor_state.set_mode("minibuffer-mode", minibuffer_mode);
    let _ = minibuffer_cleanup_primitive(&[], env.clone(), editor_state);
}
