//! Incremental search: `isearch-mode`, the mode of the prompt `C-s` opens,
//! whose `post-command-hook` searches again after every command, and the
//! mechanics its primitives share, where point lands and how a scan is set
//! up. It searches the buffer the session names, never the current one,
//! which is the prompt.
//!
//! Design: `doc/architettura-editor.typ, <ricerca-incrementale>`.
use crate::{
    BufferTrait, ELispExp, EditorState,
    buffer::{Buffer, Mark},
    feature::minibuffer::PROMPT_BUFFER,
    input::{KeyCode, KeyEvent, KeyModifiers},
    modes::MajorMode,
    text::isearch::{Direction, Isearch},
    text::search::{Match, Pattern},
};
use risp::Env;
use std::sync::Arc;

/// The Lisp variable that decides whether searching ignores case: Emacs' own
/// name, and Emacs' own default, on.
pub(crate) const CASE_FOLD: &str = "case-fold-search";

/// Returns whether searches ignore case, as Lisp has it now; unbound counts as
/// on. The one reading of `case-fold-search` for incremental search, replace,
/// occur, grep and `scan-buffer`.
pub(crate) fn case_fold<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(CASE_FOLD, true)
}

/// Returns what has been typed into the prompt so far, or an empty string
/// when no prompt is open, which a search finds nothing for.
pub(crate) fn pattern_text<B: BufferTrait>(ctx: &EditorState<B>) -> String {
    ctx.with_buffer(PROMPT_BUFFER, |buf| buf.text.to_string())
        .unwrap_or_default()
}

/// Puts BUF's point at POINT, within its text, and its mark at MARK.
pub(crate) fn place<B: BufferTrait>(buf: &mut Buffer<B>, point: usize, mark: Option<usize>) {
    buf.mark = mark.map(Mark::new);
    let target = point.min(buf.text.len());
    let (line, col) = buf.text.cursor_1d_to_2d(target);
    buf.text.cursor_move(line, col);
}

/// Shows FOUND in the searched buffer as the region: point at the far end in
/// the session's direction, where the reader is looking and where a confirmed
/// search leaves it, and the mark at the near end.
pub(crate) fn show<B: BufferTrait>(ctx: &EditorState<B>, session: &Isearch, found: &Match) {
    let (point, mark) = match session.direction {
        Direction::Forward => (found.end, found.start),
        Direction::Backward => (found.start, found.end),
    };
    ctx.with_buffer_mut(&session.buffer, |buf| place(buf, point, Some(mark)));
}

/// Returns the next match of PATTERN in the searched buffer, from the
/// session's position in its direction, or `None` when there is none or the
/// buffer is gone.
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

/// Returns how long the searched buffer is, for wrapping.
pub(crate) fn searched_len<B: BufferTrait>(ctx: &EditorState<B>, session: &Isearch) -> usize {
    ctx.with_buffer(&session.buffer, |buf| buf.text.len())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The mode
// ---------------------------------------------------------------------------

/// Installs `isearch-mode`, the keymap and hooks a search prompt runs under,
/// and makes `case-fold-search` t when it has no value. The primitives its
/// keys name are in `primitives/isearch.rs`.
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
    // Return and Escape are the minibuffer's own commands: they close the
    // prompt, and then call the callbacks the search gave it.
    isearch_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Enter),
        ELispExp::symbol("minibuffer-confirm".into()),
    );
    isearch_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Esc),
        ELispExp::symbol("minibuffer-cancel".into()),
    );
    // `C-g` is `keyboard-quit` in the global map, which knows nothing of
    // prompts; in a search it means what Escape means.
    isearch_mode
        .keymaps
        .insert_key(ctrl('g'), ELispExp::symbol("minibuffer-cancel".into()));
    // Shortening the pattern is part of typing one, as in `minibuffer-mode`;
    // deleting is a command, so the hook searches again after it.
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
    // after it.
    isearch_mode.hooks.insert(
        "post-command-hook".into(),
        vec![ELispExp::symbol("isearch-update".into())],
    );
    // The cleanup of every prompt, as in `minibuffer-mode`: the buffer before
    // it current again, and the callbacks cleared.
    isearch_mode.hooks.insert(
        "after-close-hook".into(),
        vec![ELispExp::symbol("minibuffer-cleanup".into())],
    );

    editor_state.set_mode("isearch-mode", isearch_mode);
    // Set here rather than in a `.lisp` file, so that the default holds with
    // no configuration loaded and `C-h v` finds the variable bound.
    if env.get_variable(CASE_FOLD).is_none() {
        env.set_variable(CASE_FOLD.into(), ELispExp::t());
    }
}
