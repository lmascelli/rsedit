//! egui's keys in the editor's words: the character a key with Ctrl or Alt
//! stands for, and the keys with a name of their own.
//!
//! Design: `doc/architettura-editor.typ, <gui>`.
use eframe::egui;
use rsedit_core::{KeyCode, KeyEvent, KeyModifiers};

/// Returns the character Shift makes of C on a US keyboard: for the symbols
/// egui has no key for, `_` or `<`, when no text says better.
fn shifted(c: char) -> char {
    match c {
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '`' => '~',
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        c => c.to_ascii_uppercase(),
    }
}

/// Returns the editor's key for a key pressed with MODIFIERS holding Ctrl or
/// Alt: the character TEXT reports, when there is one, since the keyboard's
/// layout decides punctuation and some keys have no egui key; FALLBACK, the
/// key's own character, otherwise. `None` without Ctrl or Alt, and for text
/// that is typing, as AltGr's.
pub(super) fn character_shortcut(
    fallback: Option<char>,
    mut modifiers: KeyModifiers,
    text: Option<&str>,
) -> Option<KeyEvent> {
    if !modifiers.ctrl && !modifiers.alt {
        return None;
    }
    let c = if let Some(text) = text {
        let mut chars = text.chars();
        let c = chars.next()?;
        if chars.next().is_some() || c.is_control() {
            return None;
        }
        // AltGr, and Option composing past ASCII, type their text.
        if modifiers.alt && (modifiers.ctrl || !c.is_ascii()) {
            return None;
        }
        c
    } else {
        let c = fallback?;
        if modifiers.shift { shifted(c) } else { c }
    };
    modifiers.shift = false; // Shift is already represented by the character.
    Some(KeyEvent {
        code: KeyCode::Char(c),
        modifiers,
    })
}

/// Returns egui's MODIFIERS as the editor's.
pub(super) fn mods(modifiers: egui::Modifiers) -> KeyModifiers {
    KeyModifiers {
        ctrl: modifiers.ctrl,
        alt: modifiers.alt,
        shift: modifiers.shift,
    }
}

/// Returns the character KEY stands for, lower case, or `None` for a key
/// that is not one.
pub(super) fn character(key: egui::Key) -> Option<char> {
    use egui::Key;
    match key {
        Key::Space => Some(' '),
        Key::Minus => Some('-'),
        Key::Plus => Some('+'),
        Key::Equals => Some('='),
        Key::OpenBracket => Some('['),
        Key::CloseBracket => Some(']'),
        Key::Backslash => Some('\\'),
        Key::Slash => Some('/'),
        Key::Comma => Some(','),
        Key::Period => Some('.'),
        Key::Semicolon => Some(';'),
        Key::Quote => Some('\''),
        Key::Backtick => Some('`'),
        _ => {
            let name = key.symbol_or_name();
            let mut chars = name.chars();
            let first = chars.next()?;
            (chars.next().is_none() && first.is_ascii_graphic())
                .then_some(first.to_ascii_lowercase())
        }
    }
}

/// Returns the editor's name for KEY when it has one of its own: the arrows,
/// Escape, Return, Backspace and Tab.
pub(super) fn special_key(key: egui::Key) -> Option<KeyCode> {
    use egui::Key;
    Some(match key {
        Key::ArrowLeft => KeyCode::Left,
        Key::ArrowRight => KeyCode::Right,
        Key::ArrowUp => KeyCode::Up,
        Key::ArrowDown => KeyCode::Down,
        Key::Escape => KeyCode::Esc,
        Key::Enter => KeyCode::Enter,
        Key::Backspace => KeyCode::Backspace,
        Key::Tab => KeyCode::Tab,
        _ => return None,
    })
}
