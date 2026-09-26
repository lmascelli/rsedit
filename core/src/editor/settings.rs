//! The Lisp variables the editor reads, and their defaults.
//!
//! Each is a name Lisp may `setq` and a reader that copes with it being unset
//! or set to something of the wrong shape. The readers take an `Env` and no
//! editor state, which is what keeps them callable from anywhere without
//! raising an ordering question.

use super::*;

/// The name of the Lisp variable that arms the echo area's timeout.
pub const ECHO_MESSAGE_TIMEOUT: &str = "echo-message-timeout";

/// How long an echo message stays on screen when nothing sets
/// [`ECHO_MESSAGE_TIMEOUT`] to something else.
pub const DEFAULT_ECHO_MESSAGE_TIMEOUT: f64 = 5.0;

/// The names of the Lisp variables that size a minibuffer prompt.
pub const MINIBUFFER_WIDTH: &str = "minibuffer-width";

pub const MINIBUFFER_HEIGHT: &str = "minibuffer-height";

/// How large a prompt is when nothing says otherwise.
///
/// Wide enough for a path and narrow enough to read as a dialogue rather than
/// as part of the frame. Both are clamped to what the terminal can actually
/// hold, so these are a preference and not a promise -- a 40-column terminal
/// gets a 38-column prompt rather than one running off the edge.
///
/// Three rows, of which one is text: a border above and below, and the line
/// being typed between them.
pub const DEFAULT_MINIBUFFER_WIDTH: f64 = 60.0;

pub const DEFAULT_MINIBUFFER_HEIGHT: f64 = 3.0;

/// The name of the Lisp variable holding the mode-line format.
pub const MODE_LINE_FORMAT: &str = "mode-line-format";

/// What a window's status line says when nothing sets [`MODE_LINE_FORMAT`].
pub const DEFAULT_MODE_LINE_FORMAT: &str = " %* %b   %m   L%l C%c   %p ";

pub const MOUSE_MODE: &str = "mouse-mode";

pub const WINDOW_SEPARATOR: &str = "window-separator";

pub const DEFAULT_WINDOW_SEPARATOR: char = '\u{2502}';

/// The mode-line format as Lisp currently defines it.
///
/// Anything that is not a string falls back to the default rather than
/// blanking every status line in the editor: a mistyped format should look
/// wrong, not make the editor look broken.
pub(super) fn mode_line_format<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> String {
    match env.get_variable(MODE_LINE_FORMAT) {
        Some(ELispExp::String(format)) => format.to_string(),
        _ => DEFAULT_MODE_LINE_FORMAT.to_string(),
    }
}

/// Whether the editor is reading the mouse, as Lisp currently defines it.
///
/// Off unless something says otherwise, and the default `init.lisp` says
/// otherwise. It is a setting at all because it costs something: a terminal
/// reporting the mouse to the editor is not using it for its own selection, so
/// turning this on trades the terminal's copy-and-paste for the editor's. That
/// is the right trade for most people and an unpleasant surprise for anyone
/// who was never asked -- which is why upgrading does not make it for them.
pub fn mouse_mode<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.get_variable(MOUSE_MODE)
        .is_some_and(|value| !value.is_nil())
}

pub(super) fn window_separator<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> char {
    match env.get_variable(WINDOW_SEPARATOR) {
        Some(ELispExp::String(text)) => text.chars().next().unwrap_or(' '),
        Some(value) if value.is_nil() => ' ',
        _ => DEFAULT_WINDOW_SEPARATOR,
    }
}

/// The echo timeout as Lisp currently defines it, or `None` for "never
/// expires".
///
/// Only a finite, non-negative number arms the timeout. `nil` means the
/// message stays until something replaces it, and so does anything else --
/// an unbound variable, a string, a list, or an infinity. Refusing to guess
/// at a nonsensical value is the safe direction: the failure mode is a
/// message that outstays its welcome, not one that disappears before it is
/// read.
pub(super) fn echo_timeout<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
    match env.get_variable(ECHO_MESSAGE_TIMEOUT) {
        Some(ELispExp::Number(seconds)) if seconds.is_finite() => {
            Some(Duration::from_secs_f64(seconds.max(0.0)))
        }
        _ => None,
    }
}
