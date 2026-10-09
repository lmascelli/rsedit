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
    input::{KeyCode, KeyEvent, KeyModifiers},
    managers::DEFAULT_HISTORY_LENGTH,
    modes::MajorMode,
};
use risp::Env;
use std::sync::Arc;
/// Set NAME to VAL the way Lisp's `setq` special form does: update an
/// existing binding wherever it is up the scope chain if one exists,
/// otherwise declare it fresh in ENV's own scope. `Env::set_variable`
/// alone always declares fresh *locally* -- correct for a genuinely new
/// binding, but wrong for a global like `*minibuffer-on-confirm*` mutated
/// from inside a primitive that was itself called with some nested
/// per-call environment: it would create a shadow invisible to a later
/// read from a different frame, rather than updating the global.
pub(crate) fn setq<B: BufferTrait>(env: &Env<EditorState<B>>, name: &str, val: ELispExp<B>) {
    if !env.update_variable(name, val.clone()) {
        env.set_variable(name.to_string(), val);
    }
}

/// Replace the minibuffer buffer's contents with CONTENT, character by
/// character (mirroring what `self-insert` does per character, including
/// marking the buffer modified) after clearing it. Not a primitive --
/// purely an internal helper for `minibuffer-complete`'s Tab-cycling.
pub(crate) fn set_minibuffer_content<B: BufferTrait>(ctx: &EditorState<B>, content: &str) {
    ctx.with_current_buffer_mut(|buf| {
        while buf.text.cursor_pos() != (0, 0) {
            buf.text.delete();
        }
        for c in content.chars() {
            buf.text.insert(c);
            buf.is_modified = true;
        }
    });
}

/// The name of the Lisp variable holding the key the current prompt's history
/// is filed under.
pub(crate) const MINIBUFFER_HISTORY_KEY: &str = "*minibuffer-history-key*";

/// The name of the Lisp variable holding the input the candidate list in
/// `*minibuffer-completions*` was computed for, so that a second Tab on the
/// same input does not ask again.
///
/// It vouches for that list, so it goes wherever the list goes. Kept past the
/// prompt that set it, it vouched for the nil the list is reset to -- and the
/// next prompt's first Tab on the same input, which for a prompt nobody has
/// typed into yet is the empty string, was answered "No completions".
pub(crate) const MINIBUFFER_COMPLETIONS_FOR: &str = "*minibuffer-completions-for*";

/// The name of the Lisp variable capping how much one prompt remembers.
pub(crate) const HISTORY_LENGTH: &str = "history-length";

/// Which ring the prompt now open is reading and writing.
///
/// # Why the prompt's own text is the default
///
/// Every prompt in the editor gets a history without a single caller being
/// changed, which is the whole point: `M-x`, `find-file`, `switch-to-buffer`
/// and anything a module prompts for are all `minibuffer-read` calls, and a
/// history that only worked for the ones that opted in would be a history
/// nobody could rely on.
///
/// The cost is that two prompts worded identically share a ring. That is
/// usually right -- they are usually the same question -- and where it is not,
/// the caller says so with the optional HISTORY argument.
pub(crate) fn history_key<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> String {
    match env.get_variable(MINIBUFFER_HISTORY_KEY) {
        Some(ELispExp::String(key)) | Some(ELispExp::Symbol(key)) if !key.is_empty() => {
            key.to_string()
        }
        _ => String::new(),
    }
}

/// How much one prompt remembers, as Lisp currently has it.
pub(crate) fn history_length<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> usize {
    // Zero is allowed and means "remember nothing", which is a setting
    // somebody may genuinely want; it is the floor only to keep a negative
    // from becoming a very large `usize`.
    match env.number_at_least(HISTORY_LENGTH, 0.0) {
        Some(n) => n as usize,
        None => DEFAULT_HISTORY_LENGTH,
    }
}

/// Put the prompt's state back to "no prompt open".
///
/// Extracted from the `minibuffer-cleanup` primitive, which is now one line in
/// [`crate::primitives::minibuffer`] that calls this. Two callers want it and
/// only one of them is Lisp: `install_minibuffer` runs it once at boot, so that
/// the variables a prompt reads are bound before any prompt has ever opened.
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
    // The entries stay; only the place in them goes. A position into a ring
    // outlives nothing -- the line it was walking through is gone with the
    // prompt -- and a stale one would make the next prompt's first `M-p`
    // continue somebody else's walk.
    ctx.history_mut(|history| history.end_walk());
}

/// Install `minibuffer-mode`: the keymap and hooks a prompt runs under, and
/// the variable that says which function implements reading a line.
///
/// Built in Rust rather than read from a `.lisp` file because while a prompt is
/// open this keymap is the first one consulted -- a file that failed to load
/// would leave a prompt whose Return key did nothing.
///
/// The primitives these keys name are installed with every other primitive, by
/// [`crate::primitives::minibuffer`]; this binds them, and does not define them.
pub fn install_minibuffer<B: BufferTrait>(
    editor_state: &EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
) {
    // Names the function that actually implements `minibuffer-read`.
    // Rebind this (`(setq *minibuffer-read-function* 'my-own-prompt)`) to
    // replace the built-in minibuffer with a custom implementation; see
    // `minibuffer-read`'s docstring for the required signature.
    env.set_variable(
        "*minibuffer-read-function*".into(),
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
    // Correcting a typo is part of reading a line of input, so it is bound
    // here with the rest of the prompt's mechanics rather than left to the
    // global map -- where it lives in a `.lisp` file that can fail to load.
    // Without it a mistyped prompt can only be abandoned and started again.
    minibuffer_mode.keymaps.insert_key(
        KeyEvent::new(KeyCode::Backspace),
        ELispExp::symbol("delete-backward-char".into()),
    );
    // Recall, bound here with the rest of the prompt's mechanics rather than
    // in a `.lisp` file that can fail to load. A prompt that has forgotten
    // what you typed into it a minute ago is one you retype the long path
    // into, and the way out of a missing module is itself a prompt.
    //
    // # Why three spellings, and why none of them needs a condition
    //
    // `C-p'/`C-n' are what the hands already do, and in a one-line prompt they
    // have nothing else to mean. `M-p'/`M-n' are Emacs' own. The arrows are
    // what somebody who has never used Emacs reaches for.
    //
    // All six would collide with the completion strip, which binds `C-n',
    // `C-p' and the arrows to move through its candidates -- except that the
    // strip installs a *transient* keymap, and a transient keymap is consulted
    // before every other (see `resolve_key_sequence`). So while candidates are
    // showing these are not reached at all, and the moment the strip goes down
    // they are. Neither side tests for the other, which is the only version of
    // this that stays true when a third thing wants the same keys.
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
    // Bound before any prompt has opened, so that the first one reads a nil
    // rather than an unbound variable.
    reset_prompt_state(&env, editor_state);
}
