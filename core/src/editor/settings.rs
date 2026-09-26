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

/// The name of the Lisp variable that turns the line-number gutter on.
///
/// `nil` or unbound is off, `'relative` counts from point, and anything else
/// truthy is ordinary absolute numbering -- the same three answers Emacs
/// gives to the same name, so somebody who already knows one does not have to
/// learn the other.
pub const DISPLAY_LINE_NUMBERS: &str = "display-line-numbers";

/// The name of the Lisp variable holding the gutter's minimum width, in
/// digits.
pub const DISPLAY_LINE_NUMBERS_WIDTH: &str = "display-line-numbers-width";

/// How many digits the gutter reserves when nothing sets
/// [`DISPLAY_LINE_NUMBERS_WIDTH`].
///
/// Three, which covers most of what anybody reads at once. The point of a
/// minimum is that the text does not shuffle sideways as a file grows past
/// its ninth line and then its ninety-ninth; the point of it being small is
/// that a three-line buffer does not lose six columns to whitespace.
pub const DEFAULT_LINE_NUMBER_DIGITS: usize = 3;

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

/// How the gutter is configured, as Lisp currently defines it.
///
/// Read here rather than inside composition for the reason the mode-line
/// format is: the environment has locks of its own, and composition runs with
/// the window and buffer locks already held.
pub(super) fn line_numbers<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> GutterSpec {
    let numbers = match env.get_variable(DISPLAY_LINE_NUMBERS) {
        None => LineNumbers::Off,
        Some(value) if value.is_nil() => LineNumbers::Off,
        // A symbol, because that is how it is written -- but a string too,
        // since `(setq display-line-numbers "relative")` is the mistake
        // everybody makes once and there is nothing else it could have meant.
        Some(ELispExp::Symbol(name)) if &**name == "relative" => LineNumbers::Relative,
        Some(ELispExp::String(name)) if &**name == "relative" => LineNumbers::Relative,
        // Any other truthy value asks for numbers, and gets the ordinary
        // kind. Guessing at a misspelt `'reltaive` would be worse than showing
        // numbers that are merely not the ones asked for: the mistake is
        // visible either way, and this way the feature still works.
        Some(_) => LineNumbers::Absolute,
    };
    let min_digits = match env.get_variable(DISPLAY_LINE_NUMBERS_WIDTH) {
        Some(ELispExp::Number(digits)) if digits.is_finite() && digits >= 1.0 => {
            // Clamped, because this is multiplied into a column of blanks on
            // every row of every window: a fat-fingered 1e9 should be a wide
            // gutter, not an allocation the size of the frame.
            (digits as usize).min(MAX_LINE_NUMBER_DIGITS)
        }
        _ => DEFAULT_LINE_NUMBER_DIGITS,
    };
    GutterSpec {
        numbers,
        min_digits,
    }
}

/// The widest a gutter's minimum may be asked to be.
///
/// Twenty digits is more than a `usize` of lines can reach, so nothing real
/// is refused by this -- it exists so that a nonsense value is a wide gutter
/// rather than a window that is all gutter.
const MAX_LINE_NUMBER_DIGITS: usize = 20;

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
