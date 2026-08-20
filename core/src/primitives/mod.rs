use crate::{
    BufferTrait, ELispExp, EditorState,
    input::{KeyCode, KeyEvent, KeyModifiers},
    lisp::{Env, EvalError, LispContext},
    modes::{MajorMode, SyntaxRule},
    ui::{Face, FloatingWindow, Rect, Window},
};

fn parse_key_sequence(seq: &str) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers::default();
    let mut chars = seq.chars().peekable();

    if seq.starts_with("C-") {
        modifiers.ctrl = true;
        chars.nth(0);
        chars.nth(0);
    } else if seq.starts_with("M-") {
        modifiers.alt = true;
        chars.nth(0);
        chars.nth(0);
    } else if seq.starts_with("C-M-") {
        modifiers.ctrl = true;
        modifiers.alt = true;
        chars.nth(0);
        chars.nth(0);
        chars.nth(0);
        chars.nth(0);
    }

    let key_code = match chars.collect::<String>().as_str() {
        "<ret>" | "<Return>" => KeyCode::Enter,
        "<esc>" | "<Escape>" => KeyCode::Esc,
        "tab" | "<Tab>" => KeyCode::Tab,
        "<backspace>" => KeyCode::Backspace,
        "<up>" => KeyCode::Up,
        "<down>" => KeyCode::Down,
        "<left>" => KeyCode::Left,
        "<right>" => KeyCode::Right,
        s if s.len() == 1 => KeyCode::Char(
            s.chars()
                .next()
                .expect(&format!("Failed to interpret the sequence {seq}")),
        ),
        _ => return None,
    };

    Some(KeyEvent {
        code: key_code,
        modifiers,
    })
}

#[macro_export]
macro_rules! primitive {
    ($func_name:ident, $args:ident, $env:ident, $ctx:ident, $body:block) => {
        pub fn $func_name<B: BufferTrait>(
            $args: &[ELispExp<B>],
            $env: std::sync::Arc<Env<EditorState<B>>>,
            $ctx: &EditorState<B>,
        ) -> Result<ELispExp<B>, EvalError> {
            $body
        }
    };
}

pub mod buffers;
pub mod edits;
pub mod general;
pub mod io;
pub mod modes;
pub mod ui;
