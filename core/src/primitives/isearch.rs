//! Incremental search, as Lisp sees it.
//!
//! Ten primitives: four that start a search and six that an open prompt's keys
//! run. The search itself -- the session, where point lands, what the prompt
//! reports -- is [`crate::feature::isearch`], and so is `isearch-mode`.
//!
//! # Why the split is here and not elsewhere
//!
//! A primitive is a *name Lisp can call*, and every one of them in this editor
//! is under `primitives/`. The mechanics are not: they are called by these and
//! by the mode, and they would be exactly as useful to a second search written
//! in Lisp.
use super::*;
use crate::feature::isearch;
use crate::text::search::{Direction, Isearch, Pattern};
use std::sync::Arc;

/// Open the prompt and record where the search began.
fn begin<B: BufferTrait>(
    env: Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
    direction: Direction,
    regexp: bool,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    // Read before the prompt opens, because opening it makes the minibuffer
    // current and this position would then be the wrong buffer's.
    let buffer = ctx.get_current_buffer_name();
    let origin = ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d());
    let session = Isearch::new(buffer, origin, direction, regexp);
    let prompt = session.report("");
    ctx.begin_isearch(session);

    let reader = env
        .get_function("minibuffer-read")
        .ok_or_else(|| EvalError::UndefinedFunction("minibuffer-read".into()))?;
    risp::call_callable(
        &reader,
        &[
            ELispExp::string(prompt),
            ELispExp::primitive(isearch_exit, None),
            // No completion: there is nothing to complete a search pattern
            // against, and Tab is free for anything a later mode wants.
            ELispExp::nil(),
            ELispExp::primitive(isearch_abort, None),
            ELispExp::string("isearch-mode".into()),
        ],
        env.clone(),
        ctx,
    )
}

pub const ISEARCH_FORWARD_DOC: &str = "(isearch-forward): Search forward as you type: a prompt \
         opens, and each character typed moves to the first match of what has been typed so far. \
         Inside the search C-s goes on to the next match and C-r back, Return stops at the match, \
         and C-g or Escape goes back to where the search started. Case is ignored unless \
         `case-fold-search' is nil.";

primitive!(isearch_forward, _args, env, ctx, {
    begin(env, ctx, Direction::Forward, false)
});

pub const ISEARCH_BACKWARD_DOC: &str = "(isearch-backward): Like `isearch-forward', searching \
         backward from point.";

primitive!(isearch_backward, _args, env, ctx, {
    begin(env, ctx, Direction::Backward, false)
});

pub const ISEARCH_FORWARD_REGEXP_DOC: &str = "(isearch-forward-regexp): Like `isearch-forward', \
         for a regular expression. A pattern still being typed, and not yet valid, reads as a \
         failing search.";

primitive!(isearch_forward_regexp, _args, env, ctx, {
    begin(env, ctx, Direction::Forward, true)
});

pub const ISEARCH_BACKWARD_REGEXP_DOC: &str = "(isearch-backward-regexp): Like \
         `isearch-forward-regexp', searching backward from point.";

primitive!(isearch_backward_regexp, _args, env, ctx, {
    begin(env, ctx, Direction::Backward, true)
});

// ---------------------------------------------------------------------------
// Running one
// ---------------------------------------------------------------------------

pub const ISEARCH_UPDATE_DOC: &str = "(isearch-update): Search again for what the prompt holds, \
         and move to what is found. Run after every command while a search is open; not called \
         directly.";

primitive!(isearch_update, _args, env, ctx, {
    let Some(mut session) = ctx.take_isearch() else {
        return Ok(ELispExp::nil());
    };
    let text = isearch::pattern_text(ctx);

    // Nothing typed yet, or everything backspaced away. Point goes back to
    // where the search started: with no pattern there is no match, and leaving
    // point at the last one would make the search look like it had found
    // something it had not.
    if text.is_empty() {
        let origin = session.origin;
        ctx.with_buffer_mut(&session.buffer, |buf| isearch::place(buf, origin, None));
        session.found = None;
        session.failing = false;
        let report = session.report("");
        ctx.begin_isearch(session);
        ctx.set_echo_message(&report);
        return Ok(ELispExp::nil());
    }

    // A regexp is invalid for most of the time it is being typed -- `[a-` is
    // on the way to `[a-z]` -- so a failure to compile is reported the same
    // way a failure to match is, and typing the next character fixes it.
    match Pattern::new(&text, session.regexp, isearch::case_fold(&env)) {
        Ok(pattern) => match isearch::scan(ctx, &session, &pattern) {
            Some(found) => {
                isearch::show(ctx, &session, &found);
                session.found = Some(found);
                session.failing = false;
            }
            None => {
                session.found = None;
                session.failing = true;
            }
        },
        Err(_) => {
            session.found = None;
            session.failing = true;
        }
    }

    let report = session.report(&text);
    ctx.begin_isearch(session);
    ctx.set_echo_message(&report);
    Ok(ELispExp::nil())
});

/// `C-s` and `C-r` from inside a search: go to the next match, or turn around.
///
/// Changing direction does not move on -- it re-runs the scan the other way
/// from where it already is, so `C-s C-r` walks back over the same matches
/// rather than skipping one.
fn repeat<B: BufferTrait>(
    env: Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
    direction: Direction,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let Some(mut session) = ctx.take_isearch() else {
        return Ok(ELispExp::nil());
    };

    if session.direction != direction {
        session.direction = direction;
        // Turned around at the current match rather than past it: the match on
        // screen is where the user is looking, and stepping over it before
        // searching the other way would silently skip it.
        if let Some(found) = &session.found {
            session.from = match direction {
                Direction::Forward => found.start,
                Direction::Backward => found.end,
            };
        }
    } else if session.failing {
        // Already run out once, and asked again: that second press is the
        // decision to go round. See `Isearch::wrap`.
        let len = isearch::searched_len(ctx, &session);
        session.wrap(len);
    } else {
        session.advance();
    }

    ctx.begin_isearch(session);
    // The scan itself is not repeated here -- `isearch-update` does it, and it
    // runs after this command like it runs after any other. One place searches,
    // so a repeat and a keystroke cannot come to different conclusions.
    let _ = env;
    Ok(ELispExp::nil())
}

pub const ISEARCH_REPEAT_FORWARD_DOC: &str = "(isearch-repeat-forward): Go on to the next match, \
         turning the search round if it was going backward; after a failed search, start again \
         from the top of the buffer. C-s inside a search.";

primitive!(isearch_repeat_forward, _args, env, ctx, {
    repeat(env, ctx, Direction::Forward)
});

pub const ISEARCH_REPEAT_BACKWARD_DOC: &str = "(isearch-repeat-backward): Go back to the previous \
         match, turning the search round if it was going forward; after a failed search, start \
         again from the end of the buffer. C-r inside a search.";

primitive!(isearch_repeat_backward, _args, env, ctx, {
    repeat(env, ctx, Direction::Backward)
});

// ---------------------------------------------------------------------------
// Ending one
// ---------------------------------------------------------------------------

pub const ISEARCH_EXIT_DOC: &str = "(isearch-exit): End the search, leaving point at the match. \
         What Return in a search runs, once the prompt has closed.";

primitive!(isearch_exit, _args, _env, ctx, {
    let Some(session) = ctx.take_isearch() else {
        return Ok(ELispExp::nil());
    };
    // Point is already at the match -- every update put it there. All that is
    // left is to say what happened, and to stop the mark making the match look
    // like a selection the next command should act on.
    ctx.with_buffer_mut(&session.buffer, |buf| {
        if let Some(mark) = buf.mark.as_mut() {
            mark.active = false;
        }
    });
    ctx.set_echo_message(&match session.found {
        Some(_) => "Mark saved where search started".to_string(),
        None => "Search failed".to_string(),
    });
    Ok(ELispExp::nil())
});

pub const ISEARCH_ABORT_DOC: &str = "(isearch-abort): Abandon the search and put point back where \
         it started. What C-g and Escape in a search run, once the prompt has closed.";

primitive!(isearch_abort, _args, _env, ctx, {
    let Some(session) = ctx.take_isearch() else {
        return Ok(ELispExp::nil());
    };
    let origin = session.origin;
    ctx.with_buffer_mut(&session.buffer, |buf| isearch::place(buf, origin, None));
    ctx.set_echo_message("Quit");
    Ok(ELispExp::nil())
});

pub const ISEARCH_ACTIVE_P_DOC: &str = "(isearch-active-p): Return t while a search is open, and \
         nil otherwise.";

primitive!(isearch_active_p, _args, _env, ctx, {
    Ok(if ctx.isearch_active() {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

/// Register this module's primitives: incremental search.
///
/// Called by [`super::install_primitives`]. The four entry points are commands
/// so that `M-x` reaches them; the six that answer an open prompt are not --
/// `M-x isearch-repeat-forward` with no search running has nothing to repeat,
/// and offering it in the completion list would be offering a mistake.
///
/// All ten are registered here, including the six that used to be installed by
/// `install_isearch` alongside the mode. They are Rust either way, so nothing
/// about a `.lisp` file failing to load is different; what is different is that
/// "where are the isearch primitives registered" now has one answer.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("isearch-forward", isearch_forward, &[], ISEARCH_FORWARD_DOC);
    into.command(
        "isearch-backward",
        isearch_backward,
        &[],
        ISEARCH_BACKWARD_DOC,
    );
    into.command(
        "isearch-forward-regexp",
        isearch_forward_regexp,
        &[],
        ISEARCH_FORWARD_REGEXP_DOC,
    );
    into.command(
        "isearch-backward-regexp",
        isearch_backward_regexp,
        &[],
        ISEARCH_BACKWARD_REGEXP_DOC,
    );

    into.function("isearch-update", isearch_update, ISEARCH_UPDATE_DOC);
    into.function("isearch-exit", isearch_exit, ISEARCH_EXIT_DOC);
    into.function("isearch-abort", isearch_abort, ISEARCH_ABORT_DOC);
    into.function(
        "isearch-repeat-forward",
        isearch_repeat_forward,
        ISEARCH_REPEAT_FORWARD_DOC,
    );
    into.function(
        "isearch-repeat-backward",
        isearch_repeat_backward,
        ISEARCH_REPEAT_BACKWARD_DOC,
    );
    into.function("isearch-active-p", isearch_active_p, ISEARCH_ACTIVE_P_DOC);
}
