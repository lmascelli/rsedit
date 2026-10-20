//! The Lisp variables the editor reads, and their defaults.
//!
//! Each is a name Lisp may `setq` and a reader that copes with it being unset
//! or set to something of the wrong shape. The readers take an `Env` and no
//! editor state, which is what keeps them callable from anywhere without
//! raising an ordering question.

use super::*;
use crate::feature::isearch::CASE_FOLD;
use crate::feature::minibuffer::HISTORY_LENGTH;
use crate::modes::autosave::{AUTO_SAVE, AUTO_SAVE_DIRECTORY, AUTO_SAVE_INTERVAL};
use crate::modes::watcher::{WATCH_FILE_INTERVAL, WATCH_FILES};
use risp::VARIABLE_DOCUMENTATION;

/// A Lisp variable the editor reads, and what `C-h v` says about it.
pub struct Setting {
    pub name: &'static str,
    pub doc: &'static str,
}

/// The settings the editor reads, with their documentation. Each starts with
/// a value, set in `create_global_env`, so `C-h v` also shows what it is.
pub const SETTINGS: &[Setting] = &[
    Setting {
        name: ECHO_MESSAGE_TIMEOUT,
        doc: "How many seconds a message stays in the echo area. Each new message starts the \
              count again; nil, or anything but a number, keeps a message until another \
              replaces it.\n\n\
              Example:\n\
              \x20 (setq echo-message-timeout nil)",
    },
    Setting {
        name: MODE_LINE_FORMAT,
        doc: "What the status line at the bottom of each window shows: this text, with each \
              escape replaced.\n\n\
              \x20 %b  the buffer's name     %f  the file's path, or the buffer's name\n\
              \x20 %m  the major mode        %s  the mode's own state, as vim-mode's NORMAL\n\
              \x20 %l  the line, from 1      %c  the column, from 0\n\
              \x20 %p  where point is: All, Top, Bot or a percentage\n\
              \x20 %*  ** when modified, -- when not    %%  a per cent sign\n\n\
              An unknown escape is shown as written; anything but a string shows the \
              default.\n\n\
              Example:\n\
              \x20 (setq mode-line-format \" %b  L%l C%c \")",
    },
    Setting {
        name: DISPLAY_LINE_NUMBERS,
        doc: "Whether windows show line numbers down their left edge: nil for none, 'relative \
              for each line's distance from point, anything else for each line's own number. \
              A window too narrow to spare the columns shows none. `toggle-line-numbers' turns \
              them on and off.\n\n\
              Example:\n\
              \x20 (setq display-line-numbers 'relative)",
    },
    Setting {
        name: DISPLAY_LINE_NUMBERS_WIDTH,
        doc: "The fewest digits the line numbers take room for, so that the text does not \
              shift sideways as the file grows. More are used when a number needs them.",
    },
    Setting {
        name: LISP_PATH,
        doc: "The directories `eval-file' searches, in order, for a module named without a \
              directory or an extension: (eval-file \"dired\") loads dired.lisp from the first \
              one that has it. It starts as the lisp folder of the editor's data directory.\n\n\
              Example:\n\
              \x20 (setq lisp-path (cons (expand-file-name \"~/lisp\") lisp-path))",
    },
    Setting {
        name: AUTO_SAVE,
        doc: "Whether the editor keeps a copy of each file buffer with unsaved changes, for \
              `recover-file' to bring back after a crash: nil turns it off. The copy is #name# \
              beside the file, or in `auto-save-directory'.",
    },
    Setting {
        name: AUTO_SAVE_INTERVAL,
        doc: "How many seconds pass between one round of `auto-save' copies and the next, one at \
              least. A change counts from the next round.",
    },
    Setting {
        name: AUTO_SAVE_DIRECTORY,
        doc: "The directory every `auto-save' copy goes in, with the file's whole path in its \
              name; nil keeps each copy beside its file. A leading ~ is the home directory, and \
              the directory is made when the first copy is written.\n\n\
              Example:\n\
              \x20 (setq auto-save-directory \"~/.cache/rsedit\")",
    },
    Setting {
        name: WATCH_FILES,
        doc: "Whether the editor notices files changing on disk: a buffer with no unsaved \
              changes is read again, and one with unsaved changes is marked, so that saving it \
              asks first. nil turns it off.",
    },
    Setting {
        name: WATCH_FILE_INTERVAL,
        doc: "How many seconds pass between looks at the open files, a tenth at least; each look \
              takes eight of the buffers, in turn. A change counts from the next look.",
    },
    Setting {
        name: OPEN_DIRECTORY_CALLBACK,
        doc: "The function `find-file' hands a directory to, with its path; nil makes find-file \
              say that the path is a directory. Loading dired sets it to `dired'.",
    },
    Setting {
        name: MOUSE_MODE,
        doc: "Whether the editor reads the mouse. While it does, the terminal's own selection is \
              not there; most terminals give it back while Shift is held. `mouse-mode-toggle' \
              turns it on and off.",
    },
    Setting {
        name: WINDOW_SEPARATOR,
        doc: "The character drawn between windows side by side, as a string; nil draws a \
              space.\n\n\
              Example:\n\
              \x20 (setq window-separator \"|\")",
    },
    Setting {
        name: AFTER_RESIZE_HOOK,
        doc: "The functions called, with the new width and height in columns and rows, each \
              time the terminal is resized.",
    },
    Setting {
        name: CASE_FOLD,
        doc: "Whether searching ignores case. C-s, the replace commands, `occur', `grep' and \
              `scan-buffer' read it; nil makes them match case exactly. `string-match' does \
              not read it.",
    },
    Setting {
        name: CASE_REPLACE,
        doc: "Whether a replacement takes the case of what it replaces, so that replacing \
              colour with color turns Colour into Color and COLOUR into COLOR. nil puts it in \
              as typed.",
    },
    Setting {
        name: REPLACE_READ_FUNCTION,
        doc: "The function the replace commands call, with no arguments, to offer each match, \
              or nil for their own question. It answers by calling `replace-this', \
              `replace-skip', `replace-rest', `replace-back', `replace-done' or \
              `replace-abandon'.",
    },
    Setting {
        name: HISTORY_LENGTH,
        doc: "How many answers each prompt remembers, the oldest going first; 0 remembers \
              none.",
    },
    Setting {
        name: MINIBUFFER_WIDTH,
        doc: "How many columns wide a prompt is, its border included. A frame too narrow for \
              it gets one as wide as it can hold.",
    },
    Setting {
        name: MINIBUFFER_HEIGHT,
        doc: "How many rows tall a prompt is, its border included: 3 is one line of text \
              between two of border.",
    },
    Setting {
        name: MINIBUFFER_READ_FUNCTION,
        doc: "The function that opens a prompt, called with the arguments of \
              `minibuffer-read'. Every prompt goes through it, so one of your own replaces \
              them all; `default-minibuffer-prompt' is the editor's.",
    },
    Setting {
        name: COMPLETION_READ_FUNCTION,
        doc: "The function that shows completion candidates, called with the list and a \
              function to call with the one chosen; nil puts them in the prompt one Tab at a \
              time. The completion module sets it to its strip.",
    },
    Setting {
        name: COMPLETION_FILTER_FUNCTION,
        doc: "The function that matches completion candidates against what is typed, called \
              with the text and the candidates and returning those that match; nil matches \
              the start. The completion module sets it to `fuzzy-filter'.",
    },
    Setting {
        name: COMPLETION_INVALIDATED_FUNCTION,
        doc: "The function `completion-invalidate' calls, with no arguments, so that \
              candidates on screen are shown again; nil for none. The completion module sets \
              it.",
    },
    Setting {
        name: COMMAND_COMPLETION_FUNCTION,
        doc: "The function M-x asks for candidates, with what is typed; nil asks \
              `command-completions'. The completion module sets it to one matching anywhere \
              in a name.",
    },
    Setting {
        name: COMMAND_ARG_COMPLETION_FUNCTION,
        doc: "A function that completes every argument a command asks for, called with the \
              kind of argument -- file, buffer, string or number -- and what is typed; nil \
              leaves each kind to the editor.",
    },
    Setting {
        name: KEY_CAPTURE_FUNCTION,
        doc: "The function the next key sequence is handed to instead of being run, or nil. \
              `read-key-sequence' sets it, and it is nil again once the keys have been handed \
              over.",
    },
];

/// Gives every setting in [`SETTINGS`] its documentation, where `defvar` puts
/// a variable's.
pub fn document_settings<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) {
    for setting in SETTINGS {
        env.put_property(
            setting.name,
            VARIABLE_DOCUMENTATION,
            ELispExp::string(setting.doc.to_string()),
        );
    }
}

/// The name of the Lisp variable listing where `eval-file` looks for a module.
pub const LISP_PATH: &str = "lisp-path";

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

/// The name of the Lisp variable naming the function that opens a prompt.
pub const MINIBUFFER_READ_FUNCTION: &str = "*minibuffer-read-function*";

/// The name of the Lisp variable naming the function that shows completion
/// candidates.
pub const COMPLETION_READ_FUNCTION: &str = "*completion-read-function*";

/// The name of the Lisp variable naming the function that matches completion
/// candidates against what is typed.
pub const COMPLETION_FILTER_FUNCTION: &str = "*completion-filter-function*";

/// The name of the Lisp variable naming the function told that completion
/// candidates have changed.
pub const COMPLETION_INVALIDATED_FUNCTION: &str = "*completion-invalidated-function*";

/// The name of the Lisp variable naming the function M-x completes with.
pub const COMMAND_COMPLETION_FUNCTION: &str = "*command-completion-function*";

/// The name of the Lisp variable naming the function that completes every
/// argument a command asks for.
pub const COMMAND_ARG_COMPLETION_FUNCTION: &str = "*command-arg-completion-function*";

/// The name of the Lisp variable holding the mode-line format.
pub const MODE_LINE_FORMAT: &str = "mode-line-format";

/// What a window's status line says when nothing sets [`MODE_LINE_FORMAT`].
pub const DEFAULT_MODE_LINE_FORMAT: &str = " %* %b   %m %s  L%l C%c   %p ";

pub const MOUSE_MODE: &str = "mouse-mode";

/// The name of the Lisp variable holding the function `find-file` hands a
/// directory to.
pub const OPEN_DIRECTORY_CALLBACK: &str = "*open-directory-callback*";

/// The name of the Lisp variable listing what to call when the frame is
/// resized.
pub const AFTER_RESIZE_HOOK: &str = "after-resize-hook";

/// The name of the Lisp variable that turns case-preserving replacement off.
pub const CASE_REPLACE: &str = "case-replace";

/// The name of the Lisp variable naming the function that offers each match
/// of a replace.
pub const REPLACE_READ_FUNCTION: &str = "*replace-read-function*";

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
