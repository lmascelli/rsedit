//! What the terminal sends, in the editor's words: crossterm's keys and mouse
//! events as the editor's own, and nothing it has no word for.
//!
//! Design: `doc/architettura-editor.typ, <tui>`.
use crossterm::event::{
    KeyCode as CrossKeyCode, KeyModifiers as CrossModifiers, MouseButton as CrossMouseButton,
    MouseEventKind as CrossMouseKind,
};
use rsedit_core::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseKind};

/// Returns EVENT as the editor's mouse event, or `None` for a kind the editor
/// has no word for: a bare move, and the horizontal wheel.
pub fn translate_mouse(event: crossterm::event::MouseEvent) -> Option<MouseEvent> {
    let button = |button: CrossMouseButton| match button {
        CrossMouseButton::Left => MouseButton::Left,
        CrossMouseButton::Middle => MouseButton::Middle,
        CrossMouseButton::Right => MouseButton::Right,
    };
    let kind = match event.kind {
        CrossMouseKind::Down(b) => MouseKind::Down(button(b)),
        CrossMouseKind::Up(b) => MouseKind::Up(button(b)),
        CrossMouseKind::Drag(b) => MouseKind::Drag(button(b)),
        CrossMouseKind::ScrollUp => MouseKind::ScrollUp,
        CrossMouseKind::ScrollDown => MouseKind::ScrollDown,
        _ => return None,
    };
    Some(MouseEvent {
        kind,
        column: event.column,
        row: event.row,
        modifiers: KeyModifiers {
            ctrl: event.modifiers.contains(CrossModifiers::CONTROL),
            alt: event.modifiers.contains(CrossModifiers::ALT),
            shift: event.modifiers.contains(CrossModifiers::SHIFT),
        },
    })
}

// TODO(CMD-2): Delete, Home, End, PageUp, PageDown, Insert and the function
// keys are refused.
/// Returns KEY_EVENT as the editor's key, or `None` for a key the editor has
/// no name for. Shift with a character is the capital; Shift-Tab is Tab with
/// Shift held, as `S-<tab>` names it.
pub fn translate_key(key_event: crossterm::event::KeyEvent) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers {
        ctrl: key_event.modifiers.contains(CrossModifiers::CONTROL),
        alt: key_event.modifiers.contains(CrossModifiers::ALT),
        shift: key_event.modifiers.contains(CrossModifiers::SHIFT),
    };

    let code = match key_event.code {
        CrossKeyCode::Char(c) if key_event.modifiers.contains(CrossModifiers::SHIFT) => {
            modifiers.shift = false;
            KeyCode::Char(c.to_ascii_uppercase())
        }
        CrossKeyCode::Char(c) => KeyCode::Char(c),
        CrossKeyCode::Left => KeyCode::Left,
        CrossKeyCode::Right => KeyCode::Right,
        CrossKeyCode::Up => KeyCode::Up,
        CrossKeyCode::Down => KeyCode::Down,
        CrossKeyCode::Backspace => KeyCode::Backspace,
        CrossKeyCode::Enter => KeyCode::Enter,
        CrossKeyCode::Tab => KeyCode::Tab,
        CrossKeyCode::BackTab => {
            modifiers.shift = true;
            KeyCode::Tab
        }
        CrossKeyCode::Esc => KeyCode::Esc,
        _ => return None,
    };

    Some(KeyEvent { code, modifiers })
}
