//! The minibuffer: `minibuffer-mode`, the mode of the prompt a line is read
//! in, the state of the prompt open, and its history. `minibuffer-read` calls
//! the function `*minibuffer-read-function*` names, `default-minibuffer-prompt`
//! unless something else is put there, so the mechanics are in Rust and the
//! prompt's look is not.
//!
//! Design: `doc/architettura-editor.typ, <minibuffer>`.
use crate::{
    BufferTrait, ELispExp, EditorState,
    editor::MINIBUFFER_READ_FUNCTION,
    input::{KeyCode, KeyEvent, KeyModifiers},
    managers::DEFAULT_HISTORY_LENGTH,
    modes::MajorMode,
};
use risp::Env;
use std::sync::Arc;

/// Sets NAME to VAL as `setq` does: the binding wherever it is up the scope
/// chain, or a new one in ENV when there is none. `Env::set_variable` always
/// makes a new one in ENV, which, from a primitive called with an environment
/// of its own, would hide the global from every later read.
pub(crate) fn setq<B: BufferTrait>(env: &Env<EditorState<B>>, name: &str, val: ELispExp<B>) {
    if !env.update_variable(name, val.clone()) {
        env.set_variable(name.to_string(), val);
    }
}

/// Puts CONTENT in the prompt in place of the whole answer, point after it,
/// and answers whether there was a prompt to put it in. Not a primitive: what
/// Tab, the history and a chosen completion put in the prompt goes through it.
///
/// The prompt by its name, whatever buffer is current: a completion strip, or
/// a call from Lisp, may be elsewhere, and what is meant for the prompt must
/// never land in a file's buffer. And through the two doors, as any edit: the
/// replacement is one undo step, and moves what is anchored to the text.
pub(crate) fn set_minibuffer_content<B: BufferTrait>(ctx: &EditorState<B>, content: &str) -> bool {
    use crate::primitives::edits::{delete_range, insert_text};
    ctx.with_buffer_mut(PROMPT_BUFFER, |buf| {
        let whole = buf.text.len();
        // A refusal, from a prompt made read-only, leaves it as it was.
        if delete_range(buf, 0, whole) && insert_text(buf, 0, content) {
            let end = buf.text.len();
            let (line, column) = buf.text.cursor_1d_to_2d(end);
            buf.text.cursor_move(line, column);
        }
    })
    .is_some()
}

/// The buffer a prompt is read in: the same one for every prompt, an
/// incremental search's included.
pub(crate) const PROMPT_BUFFER: &str = "*Minibuffer*";

/// The name of the Lisp variable holding the key the current prompt's history
/// is filed under.
pub(crate) const MINIBUFFER_HISTORY_KEY: &str = "*minibuffer-history-key*";

/// The name of the Lisp variable holding the input the candidate list in
/// `*minibuffer-completions*` was computed for, so that a second Tab on the
/// same input does not ask again. It vouches for that list, so it is reset
/// with it.
pub(crate) const MINIBUFFER_COMPLETIONS_FOR: &str = "*minibuffer-completions-for*";

/// The name of the Lisp variable capping how much one prompt remembers.
pub(crate) const HISTORY_LENGTH: &str = "history-length";

/// Returns the key of the history ring the open prompt reads and writes: its
/// HISTORY-KEY, or else the prompt's own text, or nothing when no prompt is
/// open.
pub(crate) fn history_key<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> String {
    match env.get_variable(MINIBUFFER_HISTORY_KEY) {
        Some(ELispExp::String(key)) | Some(ELispExp::Symbol(key)) if !key.is_empty() => {
            key.to_string()
        }
        _ => String::new(),
    }
}

/// Returns how many answers one prompt remembers, as Lisp has it now:
/// `history-length` when it is a number of at least zero, and 100 otherwise.
pub(crate) fn history_length<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> usize {
    // Zero means remember nothing, and is the floor so that a negative does
    // not become a very large `usize`.
    match env.number_at_least(HISTORY_LENGTH, 0.0) {
        Some(n) => n as usize,
        None => DEFAULT_HISTORY_LENGTH,
    }
}

/// Puts the prompt's state back to no prompt open: the buffer before it
/// current again, its callbacks and completions cleared, and its place in the
/// history forgotten. `minibuffer-cleanup` runs it when a prompt closes, and
/// `install_minibuffer` at boot, so that a prompt's variables are bound before
/// the first one opens.
pub(crate) fn reset_prompt_state<B: BufferTrait>(
    env: &Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) {
    if let Some(ELispExp::String(previous)) = env.get_variable("*minibuffer-previous-buffer*") {
        ctx.switch_to_buffer(&previous);
    }
    setq(env, "*minibuffer-on-confirm*", ELispExp::nil());
    setq(env, "*minibuffer-on-change*", ELispExp::nil());
    setq(env, "*minibuffer-on-cancel*", ELispExp::nil());
    setq(env, "*minibuffer-previous-buffer*", ELispExp::nil());
    setq(env, "*minibuffer-completions*", ELispExp::nil());
    setq(env, MINIBUFFER_COMPLETIONS_FOR, ELispExp::nil());
    setq(env, "*minibuffer-completion-index*", ELispExp::number(0f64));
    setq(env, MINIBUFFER_HISTORY_KEY, ELispExp::nil());
    // The entries stay, and only the place in them goes: a stale one would
    // make the next prompt's first `M-p` go on with this prompt's walk.
    ctx.history_mut(|history| history.end_walk());
}

/// Installs `minibuffer-mode`, the keymap and hooks a prompt runs under, and
/// names `default-minibuffer-prompt` as the function that reads a line. The
/// primitives its keys name are in `primitives/minibuffer.rs`.
pub fn install_minibuffer<B: BufferTrait>(
    editor_state: &EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
) {
    // The function `minibuffer-read` calls: `(setq *minibuffer-read-function*
    // 'my-own-prompt)` puts another prompt in its place, with the signature
    // `minibuffer-read`'s docstring gives.
    env.set_variable(
        MINIBUFFER_READ_FUNCTION.into(),
        ELispExp::symbol("default-minibuffer-prompt".into()),
    );

    let mut minibuffer_mode = MajorMode::new("minibuffer-mode");
    minibuffer_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Enter),
        ELispExp::symbol("minibuffer-confirm".into()),
    );
    minibuffer_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Esc),
        ELispExp::symbol("minibuffer-cancel".into()),
    );
    minibuffer_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Tab),
        ELispExp::symbol("minibuffer-complete".into()),
    );
    // Correcting a typo is part of reading a line, so Backspace is bound here
    // rather than left to the global map, which a `.lisp` file sets.
    minibuffer_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Backspace),
        ELispExp::symbol("delete-backward-char".into()),
    );
    // The history, in three spellings: `C-p` and `C-n`, Emacs' `M-p` and
    // `M-n`, and the arrows. A completion strip's transient keymap, consulted
    // first, takes `C-p`, `C-n` and the arrows while it is showing.
    let alt = |ch: char| KeyEvent {
        code: KeyCode::Char(ch),
        modifiers: KeyModifiers {
            alt: true,
            ..Default::default()
        },
    };
    let ctrl = |ch: char| KeyEvent {
        code: KeyCode::Char(ch),
        modifiers: KeyModifiers {
            ctrl: true,
            ..Default::default()
        },
    };
    for (key, command) in [
        (KeyEvent::new(KeyCode::Up), "history-previous"),
        (KeyEvent::new(KeyCode::Down), "history-next"),
        (alt('p'), "history-previous"),
        (alt('n'), "history-next"),
        (ctrl('p'), "history-previous"),
        (ctrl('n'), "history-next"),
        // Emacs' way out of a question, beside Escape: the global `C-g`,
        // `keyboard-quit`, knows nothing of prompts. A completion strip's
        // keymap takes it first while it is showing, so the first `C-g`
        // closes the strip and the second the prompt.
        (ctrl('g'), "minibuffer-cancel"),
    ] {
        minibuffer_mode
            .keymaps
            .insert_key(key, ELispExp::symbol(command.into()));
    }
    minibuffer_mode.hooks.insert(
        "after-close-hook".into(),
        vec![ELispExp::symbol("minibuffer-cleanup".into())],
    );

    editor_state.set_mode("minibuffer-mode", minibuffer_mode);
    // Bound before any prompt opens, so that the first one reads nil rather
    // than an unbound variable.
    reset_prompt_state(&env, editor_state);
}
