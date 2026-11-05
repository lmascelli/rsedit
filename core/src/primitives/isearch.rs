//! Incremental search from Lisp: the four commands that start one, the
//! primitives an open search runs on, from its prompt's keys and its hook,
//! and the count of its matches, made at once in a small buffer and on a
//! thread of its own in a large one. Where point lands and what the prompt
//! reports are [`crate::feature::isearch`], with `isearch-mode`.
//!
//! Design: `doc/architettura-editor.typ, <ricerca-incrementale>`, and
//! `doc/architettura-editor.typ, <conteggio>` for the count.
use super::*;
use crate::buffer::Mark;
use crate::feature::isearch::{self, COUNT_AT_ONCE, COUNT_WORK};
use crate::text::isearch::{COUNT_LIMIT, Counting, Direction, Isearch, MatchCount};
use crate::text::search::Pattern;
use std::sync::Arc;

/// Makes SESSION's count of PATTERN, typed as TYPED, or asks for it, unless
/// it already has it for this version of the buffer: at once in a buffer of
/// [`COUNT_AT_ONCE`] characters or fewer, on the thread [`COUNT_WORK`]
/// otherwise, which owes `isearch--count-arrived` when it is done.
fn count<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    session: &mut Isearch,
    typed: &str,
    pattern: &Pattern,
) {
    if !isearch::lazy_count(env) {
        session.counting = None;
        return;
    }
    let Some((len, version)) =
        ctx.with_buffer(&session.buffer, |buf| (buf.text.len(), buf.version))
    else {
        return;
    };
    if session
        .counting
        .as_ref()
        .is_some_and(|counting| counting.is_for(typed, version))
    {
        return;
    }
    let counting = Counting::new(typed, version);
    session.counting = Some(counting.clone());
    if len <= COUNT_AT_ONCE {
        // A count still being made for an earlier pattern is of no use now.
        ctx.work().cancel_named(COUNT_WORK);
        let text = ctx
            .with_buffer(&session.buffer, |buf| buf.text.to_string())
            .unwrap_or_default();
        if let Some((starts, truncated)) = pattern.starts(&text, COUNT_LIMIT, || true) {
            let _ = counting.result.set(MatchCount { starts, truncated });
        }
        return;
    }
    let buffer = session.buffer.clone();
    let pattern = pattern.clone();
    let result = counting.result;
    ctx.spawn_work(COUNT_WORK, move |state, stop, id| {
        // Copied under the buffer's lock, and counted with it given back.
        let Some((text, now)) =
            state.with_buffer(&buffer, |buf| (buf.text.to_string(), buf.version))
        else {
            return;
        };
        // Changed since it was asked for: the next update asks again.
        if now != version {
            return;
        }
        let Some((starts, truncated)) = pattern.starts(&text, COUNT_LIMIT, || !stop.is_cancelled())
        else {
            return;
        };
        if result.set(MatchCount { starts, truncated }).is_ok() {
            state.owe_callback(
                COUNT_WORK,
                id,
                ELispExp::primitive(isearch_count_arrived, None),
                Vec::new(),
            );
        }
    });
}

pub const ISEARCH_COUNT_ARRIVED_DOC: &str = "(isearch--count-arrived): Show the search's report \
         again, with the count of its matches just made. Not called directly -- the thread that \
         counts owes it.";

primitive!(isearch_count_arrived, _args, _env, ctx, {
    // The prompt's text before the session: one lock at a time.
    let typed = isearch::pattern_text(ctx);
    if let Some(report) = ctx.runtime(|runtime| runtime.isearch().map(|s| s.report(&typed))) {
        ctx.set_echo_message(&report);
    }
    Ok(ELispExp::nil())
});

/// Opens the prompt and records where the search began.
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
    let opened = risp::call_callable(
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
    );

    // A prompt refused, as one is while another is open, leaves no search
    // behind it to follow the keys typed in the prompt that is.
    if opened.is_err() {
        ctx.take_isearch();
    }
    opened
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
        session.pattern = None;
        session.counting = None;
        session.failing = false;
        ctx.work().cancel_named(COUNT_WORK);
        let report = session.report("");
        ctx.begin_isearch(session);
        ctx.set_echo_message(&report);
        return Ok(ELispExp::nil());
    }

    // A regexp is invalid for most of the time it is being typed -- `[a-` is
    // on the way to `[a-z]` -- so a failure to compile is reported the same
    // way a failure to match is, and typing the next character fixes it.
    match Pattern::new(&text, session.regexp, isearch::case_fold(&env)) {
        Ok(pattern) => {
            match isearch::scan(ctx, &session, &pattern) {
                Some(found) => {
                    isearch::show(ctx, &session, &found);
                    session.found = Some(found);
                    session.failing = false;
                }
                None => {
                    session.found = None;
                    session.failing = true;
                }
            }
            count(ctx, &env, &mut session, &text, &pattern);
            session.pattern = Some(pattern);
        }
        Err(_) => {
            session.found = None;
            session.pattern = None;
            session.counting = None;
            session.failing = true;
        }
    }

    let report = session.report(&text);
    ctx.begin_isearch(session);
    ctx.set_echo_message(&report);
    Ok(ELispExp::nil())
});

/// Moves an open search on, for `C-s` and `C-r` inside it: to the next match,
/// round the buffer after a failure, or the other way. Turning round does not
/// move on, so that `C-s C-r` walks back over the same matches.
fn repeat<B: BufferTrait>(
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
    Ok(ELispExp::nil())
}

pub const ISEARCH_REPEAT_FORWARD_DOC: &str = "(isearch-repeat-forward): Go on to the next match, \
         turning the search round if it was going backward; after a failed search, start again \
         from the top of the buffer. C-s inside a search.";

primitive!(isearch_repeat_forward, _args, _env, ctx, {
    repeat(ctx, Direction::Forward)
});

pub const ISEARCH_REPEAT_BACKWARD_DOC: &str = "(isearch-repeat-backward): Go back to the previous \
         match, turning the search round if it was going forward; after a failed search, start \
         again from the end of the buffer. C-r inside a search.";

primitive!(isearch_repeat_backward, _args, _env, ctx, {
    repeat(ctx, Direction::Backward)
});

// ---------------------------------------------------------------------------
// Ending one
// ---------------------------------------------------------------------------

pub const ISEARCH_EXIT_DOC: &str = "(isearch-exit): End the search, leaving point at the match \
         and the mark, inactive, where the search started. What Return in a search runs, once the \
         prompt has closed.";

primitive!(isearch_exit, _args, _env, ctx, {
    let Some(session) = ctx.take_isearch() else {
        return Ok(ELispExp::nil());
    };
    ctx.work().cancel_named(COUNT_WORK);
    // Point is already at the match -- every update put it there. While the
    // search ran, the mark was the match's other end, so that the match drew
    // as a selection; now it goes where the search started, as in Emacs, so
    // that `C-x C-x' goes back there. Inactive either way: the match is not a
    // selection the next command should act on.
    let origin = session.origin;
    let moved = ctx
        .with_buffer_mut(&session.buffer, |buf| {
            if buf.text.cursor_pos_1d() == origin {
                if let Some(mark) = buf.mark.as_mut() {
                    mark.active = false;
                }
                return false;
            }
            let mut mark = Mark::new(origin.min(buf.text.len()));
            mark.active = false;
            buf.mark = Some(mark);
            true
        })
        .unwrap_or(false);
    if session.found.is_none() {
        ctx.set_echo_message("Search failed");
    } else if moved {
        ctx.set_echo_message("Mark saved where search started");
    }
    Ok(ELispExp::nil())
});

pub const ISEARCH_ABORT_DOC: &str = "(isearch-abort): Abandon the search and put point back where \
         it started. What C-g and Escape in a search run, once the prompt has closed.";

primitive!(isearch_abort, _args, _env, ctx, {
    let Some(session) = ctx.take_isearch() else {
        return Ok(ELispExp::nil());
    };
    ctx.work().cancel_named(COUNT_WORK);
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

/// Registers the primitives of incremental search.
///
/// Called by [`super::install_primitives`]. Only the four that start a search
/// are commands: the others work on an open search, and `M-x` would offer
/// them where there is none.
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
    into.function(
        "isearch--count-arrived",
        isearch_count_arrived,
        ISEARCH_COUNT_ARRIVED_DOC,
    );
}
