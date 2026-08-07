use std::collections::HashMap;
use crate::{
    buffer::{BufferTrait},
    ELispExp,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    None,
    Char(char),
    Backspace,
    Enter,
    Esc,
    Left,
    Right,
    Up,
    Down,
    // Add more as needed (Tab, F1...)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub caps_lock_as_ctrl: bool, // Placeholder for future GUI frontends!
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyEvent {
    pub fn new(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::default(),
        }
    }
}

/// Fills a keymaps map with all the ascii char self-insert char so the editor
/// can handle the typing of letters, digits and most of symbols.
pub fn fill_default_keymaps<B: BufferTrait>(keymaps: &mut HashMap<KeyEvent, ELispExp<B>>) {
    // -------------------------------- EDITOR ---------------------------------
    keymaps.insert(
        KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        },
        ELispExp::list(vec![ELispExp::symbol("quit".into())]),
    );

    // -------------------------------- BUFFER ---------------------------------
    keymaps.insert(
        KeyEvent {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        },
        ELispExp::list(vec![ELispExp::symbol("save-buffer".into())]),
    );

    keymaps.insert(
        KeyEvent {
            code: KeyCode::Left,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::list(vec![ELispExp::symbol("backward-char".into())]),
    );
    keymaps.insert(
        KeyEvent {
            code: KeyCode::Right,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::list(vec![ELispExp::symbol("forward-char".into())]),
    );
    keymaps.insert(
        KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::list(vec![ELispExp::symbol("previous-line".into())]),
    );
    keymaps.insert(
        KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::list(vec![ELispExp::symbol("next-line".into())]),
    );
    keymaps.insert(
        KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::default(),
        },
        ELispExp::list(vec![ELispExp::symbol("insert-newline".into())]),
    );

    for c in ' '..='~' {
        let event = KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::default(), // No modifiers (Ctrl/Alt off)
        };
        keymaps.insert(event,
            ELispExp::list(vec![
                ELispExp::symbol("self-insert".into()),
                ELispExp::string(c.into()),
            ]),
        );
    }
}
