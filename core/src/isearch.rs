//! Incremental search: a prompt whose every keystroke re-runs the search.
//!
//! # The shape of it
//!
//! `C-s` opens a minibuffer prompt in `isearch-mode` and records where point
//! was. From then on the work is done by that mode's `post-command-hook`:
//! every command run while the prompt is open -- which is every character typed
//! into it, and every backspace -- gets one turn of `isearch-update`, which
//! re-searches and moves point to what it found.
//!
//! That hook is the whole mechanism, and it is worth saying why it is the right
//! one. There is no blocking "read a key" in this editor: keys arrive one per
//! turn of the event loop, so the read-extend-search loop an incremental search
//! wants cannot be written as a loop. `post-command-hook` already fires exactly
//! once per command, after it has run, scoped to the current buffer's major
//! mode -- which is precisely "the pattern may have changed; look again".
//!
//! # Why it needs its own mode
//!
//! `minibuffer-mode` is shared by every prompt in the editor: `M-x`,
//! `find-file`, `M-:`. A hook hung on it would fire for all of them and have to
//! ask "is this one mine?" every time. `isearch-mode` scopes the hook to the
//! prompt that wants it, and it is also where the keys that mean something
//! different here live -- `C-s` repeats the search rather than starting one.
//!
//! It is not a *replacement* for the minibuffer, though. Return and Escape are
//! bound to `minibuffer-confirm` and `minibuffer-cancel`, the prompt is opened
//! through `minibuffer-read`, and closing it runs `minibuffer-cleanup` like any
//! other prompt. What differs is the keymap and the hook; everything else is
//! the machinery that was already there.
//!
//! # The buffer problem
//!
//! While the prompt is open the *current buffer is the minibuffer*. Everything
//! here therefore works on a buffer by **name**, taken from the session -- a
//! search that used `get_current_buffer` would search the one-line prompt the
//! user is typing into. This is the single thing most likely to be got wrong by
//! a later change, and `an_isearch_searches_the_file_not_the_prompt` is the
//! test that says so.
//!
//! # Why this is Rust and not a `.lisp` file
//!
//! For the reason the minibuffer itself is: a search that a missing or broken
//! configuration file could silently remove is not a feature of the editor. The
//! pieces it is built out of -- `minibuffer-read`, `add-hook`, `point`,
//! `goto-char` -- are all reachable from Lisp, so a *different* incremental
//! search can be written there; this one cannot be taken away.
use crate::{
    BufferTrait, ELispExp, EditorState,
    buffer::{Buffer, Mark},
    input::{KeyCode, KeyEvent, KeyModifiers},
    lisp::{Env, EvalError},
    modes::MajorMode,
    primitive,
    search::{Direction, Isearch, Match, Pattern},
};
use std::sync::Arc;

/// The Lisp variable that decides whether searching ignores case.
///
/// Emacs' own name and Emacs' own default: on. A search for "the" that skipped
/// "The" would be surprising far more often than it would be useful.
const CASE_FOLD: &str = "case-fold-search";

/// The buffer the prompt lives in. The same one every other prompt uses -- an
/// incremental search is still a line of text being read.
const PROMPT_BUFFER: &str = "*Minibuffer*";

/// Whether searches should currently ignore case.
///
/// Unbound counts as on, so a `.lisp` file that failed to load cannot turn case
/// folding off by omission.
fn case_fold<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    match env.get_variable(CASE_FOLD) {
        Some(value) => value.is_truthy(),
        None => true,
    }
}

/// What has been typed into the prompt so far.
///
/// Empty when the prompt is not open, which is also the right answer: a search
/// with nothing to look for matches nothing.
fn pattern_text<B: BufferTrait>(ctx: &EditorState<B>) -> String {
    match ctx.get_buffer(PROMPT_BUFFER) {
        Some(buffer) => buffer
            .read()
            .expect("Failed to acquire read lock on the prompt buffer")
            .text
            .to_string(),
        None => String::new(),
    }
}

/// Put point at POSITION in BUF, and the mark wherever a match wants it.
fn place<B: BufferTrait>(buf: &mut Buffer<B>, point: usize, mark: Option<usize>) {
    buf.mark = mark.map(Mark::new);
    let target = point.min(buf.text.len());
    let (line, col) = buf.text.cursor_1d_to_2d(target);
    buf.text.cursor_move(line, col);
}

/// Show FOUND in the searched buffer: point at the far end, mark at the near
/// one, so the match is the region and the machinery that already draws a
/// selection draws it.
///
/// Point at the end going forwards and at the start going backwards, which is
/// where the reader is looking either way -- and, when the search is confirmed,
/// where point is left for good.
fn show<B: BufferTrait>(ctx: &EditorState<B>, session: &Isearch, found: &Match) {
    let Some(buffer) = ctx.get_buffer(&session.buffer) else {
        return;
    };
    let (point, mark) = match session.direction {
        Direction::Forward => (found.end, found.start),
        Direction::Backward => (found.start, found.end),
    };
    ctx.mutate_buffer(buffer, |buf| place(buf, point, Some(mark)));
}

/// One scan, against the pattern as it currently reads.
///
/// Returns `None` both when the pattern is unusable -- empty, or a regexp that
/// does not compile -- and when it simply did not match. The caller reports
/// those differently but acts on them the same way, which is why they are one
/// answer here.
fn scan<B: BufferTrait>(
    ctx: &EditorState<B>,
    session: &Isearch,
    pattern: &Pattern,
) -> Option<Match> {
    let buffer = ctx.get_buffer(&session.buffer)?;
    let buf = buffer
        .read()
        .expect("Failed to acquire read lock on the buffer being searched");
    match session.direction {
        Direction::Forward => pattern.search_forward(&buf.text, session.from, buf.text.len()),
        Direction::Backward => pattern.search_backward(&buf.text, session.from, 0),
    }
}

/// How long the buffer being searched is, for wrapping.
fn searched_len<B: BufferTrait>(ctx: &EditorState<B>, session: &Isearch) -> usize {
    ctx.get_buffer(&session.buffer)
        .map(|buffer| {
            buffer
                .read()
                .expect("Failed to acquire read lock on the buffer being searched")
                .text
                .len()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Starting one
// ---------------------------------------------------------------------------

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
    let origin = {
        let buf = ctx.get_current_buffer();
        let buf = buf
            .read()
            .expect("Failed to acquire read lock on current buffer");
        buf.text.cursor_pos_1d()
    };
    let session = Isearch::new(buffer, origin, direction, regexp);
    let prompt = session.report("");
    ctx.begin_isearch(session);

    let reader = env
        .get_function("minibuffer-read")
        .ok_or_else(|| EvalError::UndefinedFunction("minibuffer-read".into()))?;
    crate::lisp::call_callable(
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

pub const ISEARCH_FORWARD_DOC: &str = "(isearch-forward): Search forward, incrementally: a prompt \
         opens and the buffer jumps to the first match of whatever has been typed so far, \
         re-searching on every keystroke.\n\n\
         `C-s' again goes to the next match, `C-r' turns around, Return stops and leaves point at \
         the match, and Escape or `C-g' abandons the search and puts point back where it started. \
         A search that runs out says so; repeating it then wraps around the buffer.\n\n\
         Case is ignored unless `case-fold-search' is nil.\n\n\
         Example:\n\
         (define-key nil \"C-s\" 'isearch-forward)";

primitive!(isearch_forward, _args, env, ctx, {
    begin(env, ctx, Direction::Forward, false)
});

pub const ISEARCH_BACKWARD_DOC: &str = "(isearch-backward): Like `isearch-forward', but searching \
         backward from point.";

primitive!(isearch_backward, _args, env, ctx, {
    begin(env, ctx, Direction::Backward, false)
});

pub const ISEARCH_FORWARD_REGEXP_DOC: &str = "(isearch-forward-regexp): Like `isearch-forward', but \
         what is typed is a regular expression. `^' and `$' match at the beginning and end of a \
         line. A pattern that is not yet a valid regexp is reported rather than signalled -- half \
         a regexp is what every complete one looks like while it is being typed.";

primitive!(isearch_forward_regexp, _args, env, ctx, {
    begin(env, ctx, Direction::Forward, true)
});

pub const ISEARCH_BACKWARD_REGEXP_DOC: &str = "(isearch-backward-regexp): Like \
         `isearch-forward-regexp', but searching backward from point.";

primitive!(isearch_backward_regexp, _args, env, ctx, {
    begin(env, ctx, Direction::Backward, true)
});

// ---------------------------------------------------------------------------
// Running one
// ---------------------------------------------------------------------------

pub const ISEARCH_UPDATE_DOC: &str = "(isearch-update): Re-run the search for whatever is in the \
         prompt and move to what it finds. Run from `isearch-mode's post-command-hook after every \
         command, which is what makes the search incremental; not normally called directly.";

primitive!(isearch_update, _args, env, ctx, {
    let Some(mut session) = ctx.take_isearch() else {
        return Ok(ELispExp::nil());
    };
    let text = pattern_text(ctx);

    // Nothing typed yet, or everything backspaced away. Point goes back to
    // where the search started: with no pattern there is no match, and leaving
    // point at the last one would make the search look like it had found
    // something it had not.
    if text.is_empty() {
        if let Some(buffer) = ctx.get_buffer(&session.buffer) {
            let origin = session.origin;
            ctx.mutate_buffer(buffer, |buf| place(buf, origin, None));
        }
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
    match Pattern::new(&text, session.regexp, case_fold(&env)) {
        Ok(pattern) => match scan(ctx, &session, &pattern) {
            Some(found) => {
                show(ctx, &session, &found);
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
        let len = searched_len(ctx, &session);
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

pub const ISEARCH_REPEAT_FORWARD_DOC: &str = "(isearch-repeat-forward): Go to the next match \
         forward. Bound to `C-s' inside an incremental search; turns the search around if it was \
         going backward. Repeating a search that has run out wraps to the beginning of the buffer.";

primitive!(isearch_repeat_forward, _args, env, ctx, {
    repeat(env, ctx, Direction::Forward)
});

pub const ISEARCH_REPEAT_BACKWARD_DOC: &str = "(isearch-repeat-backward): Go to the previous match. \
         Bound to `C-r' inside an incremental search; turns the search around if it was going \
         forward. Repeating a search that has run out wraps to the end of the buffer.";

primitive!(isearch_repeat_backward, _args, env, ctx, {
    repeat(env, ctx, Direction::Backward)
});

// ---------------------------------------------------------------------------
// Ending one
// ---------------------------------------------------------------------------

pub const ISEARCH_EXIT_DOC: &str = "(isearch-exit): Stop the search, leaving point at the match. \
         The prompt's on-confirm callback -- what Return in an incremental search runs, after \
         `minibuffer-confirm' has closed the prompt.";

primitive!(isearch_exit, _args, _env, ctx, {
    let Some(session) = ctx.take_isearch() else {
        return Ok(ELispExp::nil());
    };
    // Point is already at the match -- every update put it there. All that is
    // left is to say what happened, and to stop the mark making the match look
    // like a selection the next command should act on.
    if let Some(buffer) = ctx.get_buffer(&session.buffer) {
        ctx.mutate_buffer(buffer, |buf| {
            if let Some(mark) = buf.mark.as_mut() {
                mark.active = false;
            }
        });
    }
    ctx.set_echo_message(&match session.found {
        Some(_) => "Mark saved where search started".to_string(),
        None => "Search failed".to_string(),
    });
    Ok(ELispExp::nil())
});

pub const ISEARCH_ABORT_DOC: &str = "(isearch-abort): Abandon the search and put point back where \
         it was when the search started. The prompt's on-cancel callback -- what Escape and `C-g' \
         in an incremental search run, after `minibuffer-cancel' has closed the prompt.\n\n\
         Going back is the point of it: an incremental search moves point as you type, so \
         abandoning one has to undo that or a mistyped search would leave you somewhere you never \
         chose to be.";

primitive!(isearch_abort, _args, _env, ctx, {
    let Some(session) = ctx.take_isearch() else {
        return Ok(ELispExp::nil());
    };
    if let Some(buffer) = ctx.get_buffer(&session.buffer) {
        let origin = session.origin;
        ctx.mutate_buffer(buffer, |buf| place(buf, origin, None));
    }
    ctx.set_echo_message("Quit");
    Ok(ELispExp::nil())
});

pub const ISEARCH_ACTIVE_P_DOC: &str = "(isearch-active-p): Return t while an incremental search is \
         running, nil otherwise.";

primitive!(isearch_active_p, _args, _env, ctx, {
    Ok(if ctx.isearch_active() {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

// ---------------------------------------------------------------------------
// The mode
// ---------------------------------------------------------------------------

/// Install `isearch-mode` and the primitives that drive it.
///
/// Built in Rust rather than read from a `.lisp` file for the reason the
/// minibuffer's own mode is: while the prompt is open this keymap is the first
/// one consulted, so a file that failed to load would leave a prompt whose keys
/// did not mean what the prompt says they mean.
pub fn install_isearch<B: BufferTrait>(
    editor_state: &EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
) {
    macro_rules! install {
        ($name:literal, $func:path, $doc:expr) => {
            env.set_function($name.into(), ELispExp::primitive($func, Some($doc.into())));
        };
    }
    install!("isearch-update", isearch_update, ISEARCH_UPDATE_DOC);
    install!("isearch-exit", isearch_exit, ISEARCH_EXIT_DOC);
    install!("isearch-abort", isearch_abort, ISEARCH_ABORT_DOC);
    install!(
        "isearch-repeat-forward",
        isearch_repeat_forward,
        ISEARCH_REPEAT_FORWARD_DOC
    );
    install!(
        "isearch-repeat-backward",
        isearch_repeat_backward,
        ISEARCH_REPEAT_BACKWARD_DOC
    );
    install!("isearch-active-p", isearch_active_p, ISEARCH_ACTIVE_P_DOC);

    let ctrl = |c: char| KeyEvent {
        code: KeyCode::Char(c),
        modifiers: KeyModifiers {
            ctrl: true,
            ..Default::default()
        },
    };

    let mut isearch_mode = MajorMode::new("isearch-mode");
    // Return and Escape are the minibuffer's own commands, not new ones: they
    // close the prompt, and *then* call the callbacks the search installed. The
    // prompt is a prompt; only what it does with the answer is different.
    isearch_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Enter),
        ELispExp::symbol("minibuffer-confirm".into()),
    );
    isearch_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Esc),
        ELispExp::symbol("minibuffer-cancel".into()),
    );
    // `C-g` is bound globally to `keyboard-quit`, which abandons a half-typed
    // key sequence but knows nothing about prompts. Inside a search it has to
    // mean what Escape means, so it is bound here to say so.
    isearch_mode
        .keymaps
        .insert_key(ctrl('g'), ELispExp::symbol("minibuffer-cancel".into()));
    // Bound here for the reason `minibuffer-mode` binds it: shortening the
    // pattern is part of typing one, and the search has to re-run when it
    // happens -- which it does, because deleting is a command like any other
    // and the hook runs after every command.
    isearch_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Backspace),
        ELispExp::symbol("delete-backward-char".into()),
    );
    isearch_mode
        .keymaps
        .insert_key(ctrl('s'), ELispExp::symbol("isearch-repeat-forward".into()));
    isearch_mode.keymaps.insert_key(
        ctrl('r'),
        ELispExp::symbol("isearch-repeat-backward".into()),
    );
    // Every character typed into the prompt runs `self-insert`, and this runs
    // after it. That is the whole of "incremental".
    isearch_mode.hooks.insert(
        "post-command-hook".into(),
        vec![ELispExp::symbol("isearch-update".into())],
    );
    // The same cleanup every prompt needs: switch back to the buffer that was
    // current before, and clear the callbacks. Shared with `minibuffer-mode`
    // rather than reimplemented, because closing a prompt is closing a prompt.
    isearch_mode.hooks.insert(
        "after-close-hook".into(),
        vec![ELispExp::symbol("minibuffer-cleanup".into())],
    );

    editor_state.set_mode("isearch-mode", isearch_mode);
    let _ = env.get_variable(CASE_FOLD).is_none().then(|| {
        // Set here rather than in a `.lisp` file so the default holds with no
        // configuration loaded, and so `describe`-style introspection finds it
        // bound.
        env.set_variable(CASE_FOLD.into(), ELispExp::t())
    });
}
