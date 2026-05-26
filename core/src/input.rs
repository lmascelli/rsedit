use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    None,
    Char(char),
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    // Add more as needed (Tab, Esc, F1...)
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

pub fn default_keymaps() -> HashMap<KeyEvent, String> {
    let mut maps = HashMap::new();

    maps.insert(KeyEvent {
        code: KeyCode::Char('q'),
        modifiers: KeyModifiers {
            ctrl: true,
            .. Default::default()
        }
    }, "quit".into());

    for c in ' '..='~' {
        let event = KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::default(), // No modifiers (Ctrl/Alt off)
        };
        maps.insert(event, "self-insert".into());
    }
    
    maps
}
