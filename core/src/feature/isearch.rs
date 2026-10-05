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
//! # Where the primitives are
//!
//! [`crate::primitives::isearch`], with every other primitive in the editor.
//! What is left here is the search *mechanics* -- where point lands, what the
//! prompt says, how a scan is set up -- and `isearch-mode` itself, which is a
//! keymap that has to exist whatever configuration loaded.
//!
//! The two were one file, and the file read as a feature told twice: ten
//! primitives interleaved with the eight functions they call.
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
    lisp::Env,
    modes::MajorMode,
    text::search::{Direction, Isearch, Match, Pattern},
};
use std::sync::Arc;

/// The Lisp variable that decides whether searching ignores case.
///
/// Emacs' own name and Emacs' own default: on. A search for "the" that skipped
/// "The" would be surprising far more often than it would be useful.
pub(crate) const CASE_FOLD: &str = "case-fold-search";

/// The buffer the prompt lives in. The same one every other prompt uses -- an
/// incremental search is still a line of text being read.
pub(crate) const PROMPT_BUFFER: &str = "*Minibuffer*";

/// Whether searches should currently ignore case.
///
/// Unbound counts as on, so a `.lisp` file that failed to load cannot turn case
/// folding off by omission.
///
/// Shared with replace rather than copied there: one variable that two
/// commands read differently would be worse than either default.
pub(crate) fn case_fold<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(CASE_FOLD, true)
}

/// What has been typed into the prompt so far.
///
/// Empty when the prompt is not open, which is also the right answer: a search
/// with nothing to look for matches nothing.
pub(crate) fn pattern_text<B: BufferTrait>(ctx: &EditorState<B>) -> String {
    ctx.with_buffer(PROMPT_BUFFER, |buf| buf.text.to_string())
        .unwrap_or_default()
}

/// Put point at POSITION in BUF, and the mark wherever a match wants it.
pub(crate) fn place<B: BufferTrait>(buf: &mut Buffer<B>, point: usize, mark: Option<usize>) {
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
pub(crate) fn show<B: BufferTrait>(ctx: &EditorState<B>, session: &Isearch, found: &Match) {
    let (point, mark) = match session.direction {
        Direction::Forward => (found.end, found.start),
        Direction::Backward => (found.start, found.end),
    };
    ctx.with_buffer_mut(&session.buffer, |buf| place(buf, point, Some(mark)));
}

/// One scan, against the pattern as it currently reads.
///
/// Returns `None` both when the pattern is unusable -- empty, or a regexp that
/// does not compile -- and when it simply did not match. The caller reports
/// those differently but acts on them the same way, which is why they are one
/// answer here.
pub(crate) fn scan<B: BufferTrait>(
    ctx: &EditorState<B>,
    session: &Isearch,
    pattern: &Pattern,
) -> Option<Match> {
    ctx.with_buffer(&session.buffer, |buf| match session.direction {
        Direction::Forward => pattern.search_forward(&buf.text, session.from, buf.text.len()),
        Direction::Backward => pattern.search_backward(&buf.text, session.from, 0),
    })?
}

/// How long the buffer being searched is, for wrapping.
pub(crate) fn searched_len<B: BufferTrait>(ctx: &EditorState<B>, session: &Isearch) -> usize {
    ctx.with_buffer(&session.buffer, |buf| buf.text.len())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The mode
// ---------------------------------------------------------------------------

/// Install `isearch-mode`: the keymap and hooks a search prompt runs under.
///
/// Built in Rust rather than read from a `.lisp` file because while the prompt
/// is open this keymap is the first one consulted, so a file that failed to
/// load would leave a prompt whose keys did not mean what the prompt says they
/// mean.
///
/// The primitives these keys name are installed with every other primitive, by
/// [`crate::primitives::isearch`]; this binds them, and does not define them.
pub fn install_isearch<B: BufferTrait>(
    editor_state: &EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
) {
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
