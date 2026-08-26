use crate::{
    BufferTrait, ELispExp, EditorState,
    input::{KeyCode, KeyEvent, KeyModifiers},
    lisp::{Env, EvalError},
    modes::MajorMode,
    primitive,
};
use std::sync::Arc;

const MINIBUFFER_CLEANUP_DOC: &str = r#"Runs via after-close-hook once the minibuffer buffer closes, however it closed (confirm, cancel, or otherwise)."#;

primitive!(minibuffer_cleanup_primitive, _args, env, _ctx, {
    env.set_variable("*minibuffer-on-confirm*".into(), ELispExp::nil());
    env.set_variable("*minibuffer-on-change*".into(), ELispExp::nil());
    env.set_variable("*minibuffer-on-cancel*".into(), ELispExp::nil());
    env.set_variable("*minibuffer-previous-buffer*".into(), ELispExp::nil());
    env.set_variable("*minibuffer-completions*".into(), ELispExp::nil());
    env.set_variable(
        "*minibuffer-completion-index*".into(),
        ELispExp::number(0f64),
    );
    Ok(ELispExp::nil())
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

    editor_state.set_mode("minibuffer-mode", minibuffer_mode);
    let _ = minibuffer_cleanup_primitive(&[], env.clone(), editor_state);
}
