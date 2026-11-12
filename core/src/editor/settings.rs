//! The Lisp variables the editor reads: their names, their first values, what
//! `C-h v` says of them, and a reader for each that copes with one unset or
//! set to the wrong shape. A reader takes an `Env` and nothing of the editor,
//! and is called before any of the editor's locks is taken.
//!
//! Design: `doc/architettura-editor.typ, <avvio>`.

use super::*;
use crate::editor::frame::HIGHLIGHT_ALL_MATCHES;
use crate::editor::session::SAVE_HISTORY;
use crate::editor::visits::{RECENT_FILES_LENGTH, SAVE_PLACE};
use crate::feature::isearch::{CASE_FOLD, ISEARCH_LAZY_COUNT};
use crate::feature::minibuffer::HISTORY_LENGTH;
use crate::jobs::autosave::{AUTO_SAVE, AUTO_SAVE_DIRECTORY, AUTO_SAVE_INTERVAL};
use crate::jobs::watcher::{WATCH_FILE_INTERVAL, WATCH_FILES};
use crate::primitives::fill::{
    FILL_COLUMN, FILL_COMMENTS, FILL_SEPARATE_COMMENTS, FILL_SEPARATE_LIST_ITEMS,
};
use crate::primitives::verbs::DELETE_TRAILING_LINES;
use risp::VARIABLE_DOCUMENTATION;

/// A Lisp variable the editor reads, and what `C-h v` says about it.
pub struct Setting {
    /// The variable's name.
    pub name: &'static str,
    /// What `C-h v` shows for it.
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
              \x20 %n  the minor modes on, each after a space, as \" Fill\"\n\
              \x20 %l  the line, from 1      %c  the column, from 0\n\
              \x20 %p  All when the whole buffer is in the window, or where point is:\n\
              \x20     Top, Bot or a percentage\n\
              \x20 %i  pending I/O, error cause and last auto-save outcome/time (UTC)\n\
              \x20 %*  ** when modified, -- when not    %%  a per cent sign\n\n\
              An unknown escape is shown as written; anything but a string shows the \
              default.\n\n\
              Example:\n\
              \x20 (setq mode-line-format \" %b  L%l C%c \")",
    },
    Setting {
        name: DISPLAY_LINE_NUMBERS_TYPE,
        doc: "Which numbers `display-line-numbers-mode' draws left of a window's text: \
              'relative for how far each line is from point's, which shows its own number, and \
              anything else for each line's own. A window too narrow to spare the columns \
              shows none.\n\n\
              Example:\n\
              \x20 (setq display-line-numbers-type 'relative)",
    },
    Setting {
        name: crate::primitives::ask::USE_SHORT_ANSWERS,
        doc: "Whether a question asked with `yes-or-no' -- quitting with unsaved changes, \
              overwriting a file changed on disk, deleting in dired -- is answered with one \
              key, y or n, as `y-or-n' is, rather than by typing yes or no and Return. nil \
              unless you set it, as in Emacs.\n\n\
              Example:\n\
              \x20 (setq use-short-answers t)",
    },
    Setting {
        name: THIS_COMMAND,
        doc: "The command being run, as a symbol, while it runs and while the hooks after it \
              do, `post-self-insert-hook' and `post-command-hook' among them; nil before the \
              first. A hook that must tell Return's `newline' from a typed character reads it. \
              Set by the editor: setting it changes nothing.\n\n\
              Example:\n\
              \x20 (eq this-command 'newline)",
    },
    Setting {
        name: SCROLL_CONSERVATIVELY,
        doc: "How many lines a window scrolls to bring point back into view when it has \
              moved off the top or the bottom, as C-n on the last row does. When point went \
              further -- a search, `goto-line', a jump to an error -- its line is put in the \
              middle of the window instead, as C-l puts it. 0 always centres, as Emacs does \
              by default; above 100 never centres.\n\n\
              Example:\n\
              \x20 (setq scroll-conservatively 101)",
    },
    Setting {
        name: DISPLAY_LINE_NUMBERS_WIDTH,
        doc: "The fewest digits the line numbers take room for, so that the text does not \
              shift sideways as the file grows. More are used when a number needs them.",
    },
    Setting {
        name: LISP_PATH,
        doc: "The directories `require' and `eval-file' search, in order, for a module named \
              without a directory or an extension: (require 'dired) loads dired.lisp from the \
              first one that has it. It starts as the lisp folder of the configuration \
              directory, then each plugin's directory, then the base, features and languages \
              folders of the editor's data directory.\n\n\
              Example:\n\
              \x20 (setq lisp-path (cons (expand-file-name \"~/lisp\") lisp-path))",
    },
    Setting {
        name: THEME_PATH,
        doc: "The directories the themes are looked for in, in order: a theme in one shadows \
              a theme of the same name in the ones after it. It starts as the themes folder of \
              the configuration directory, then each plugin's, then the editor's own.\n\n\
              Example:\n\
              \x20 (setq theme-path (cons (expand-file-name \"~/themes\") theme-path))",
    },
    Setting {
        name: AUTO_SAVE,
        doc: "Whether the editor keeps a copy of each file buffer with unsaved changes, for \
              `recover-file' to bring back after a crash: nil turns it off. The copy is #name# \
              in `auto-save-directory', the local cache by default.",
    },
    Setting {
        name: AUTO_SAVE_INTERVAL,
        doc: "How many seconds pass between one round of `auto-save' copies and the next, one at \
              least. A change counts from the next round.",
    },
    Setting {
        name: AUTO_SAVE_DIRECTORY,
        doc: "The directory every `auto-save' copy goes in, with the file's whole path in its \
              name. The default is the platform cache, in its autosave subdirectory; \
              nil explicitly keeps copies beside their files. Cache errors are reported, with \
              no fallback to the file directory. A leading ~ is the home directory, and \
              the directory is made when the first copy is written.\n\n\
              Example:\n\
              \x20 (setq auto-save-directory \"~/.cache/rsedit\")",
    },
    Setting {
        name: crate::editor::file_io::TIMEOUT,
        doc: "Seconds before a pending file operation is reported as slow: 10 by default, a \
              tenth at least, or nil for no warning. This does not cancel a system call. A \
              pending save keeps its reservation until it finishes, preventing overlapping \
              writes; four saves, two reads, two watcher checks and one recovery job may run \
              at once. Synchronous Lisp calls remain synchronous.",
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
        name: HIGHLIGHT_ALL_MATCHES,
        doc: "Whether a search, and a replace, draw every match on screen in lazy-highlight \
              besides the one they are at. nil draws that one only.",
    },
    Setting {
        name: ISEARCH_LAZY_COUNT,
        doc: "Whether a search counts its matches, and shows before its prompt which of them \
              it is at and how many there are, as 3/17. In a large buffer the count arrives a \
              moment after the key. nil counts nothing.",
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
        name: SAVE_HISTORY,
        doc: "Whether the answers given at the prompts are written to the session file when \
              the editor quits, and read back when it starts: nil keeps them for one run \
              only.",
    },
    Setting {
        name: RECENT_FILES_LENGTH,
        doc: "How many files `recent-files' lists, the oldest going first; 0 lists none.\n\n\
              Example:\n\
              \x20 (setq recent-files-length 200)",
    },
    Setting {
        name: SAVE_PLACE,
        doc: "Whether point is remembered in each file when its buffer is closed or the editor \
              quits, and put back there when the file is opened again: nil opens every file \
              at its start.",
    },
    Setting {
        name: MINIBUFFER_LINES,
        doc: "How many rows the minibuffer's window takes at the bottom of the frame, above \
              the echo area: 1 unless you change it.",
    },
    // TODO(MIN-10)
    Setting {
        name: MINIBUFFER_WIDTH,
        doc: "How many columns wide a prompt is, its border included. A frame too narrow for \
              it gets one as wide as it can hold.",
    },
    // TODO(MIN-10)
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
        name: WORKER_FUEL,
        doc: "How much Lisp one step of background work may evaluate, apart from what commands \
              are given: a step of a `define-worker' worker or of a `background-call' job that \
              spends it all is stopped, and its job fails. It is read when a job starts; \
              anything but a number from 1 up counts as 200000.\n\n\
              Example:\n\
              \x20 (setq worker-fuel 400000)",
    },
    Setting {
        name: KEY_CAPTURE_FUNCTION,
        doc: "The function the next key sequence is handed to instead of being run, or nil. \
              `read-key-sequence' sets it, and it is nil again once the keys have been handed \
              over.",
    },
    Setting {
        name: RESULTS_VISIT_SAME_WINDOW,
        doc: "Whether result visits replace the selected window's buffer. \
              With nil, prefer the source window or open another window.",
    },
    Setting {
        name: RESULTS_REUSE_BUFFER,
        doc: "Whether a new run replaces the newest output of the same producer. \
              Replacement stops its previous job.",
    },
    Setting {
        name: RESULTS_DISPLAY_LOCATION,
        doc: "Where output is displayed: same-window or bottom. \
              An invalid value uses bottom.",
    },
    Setting {
        name: FILL_COLUMN,
        doc: "The column `fill-paragraph' and `auto-fill-mode' keep lines to, 70 unless you \
              change it. A mode's own fill-column property wins over this in its buffers, and \
              a buffer's own, set by `set-fill-column', over both.\n\n\
              Example:\n\
              \x20 (setq fill-column 78)\n\
              \x20 (put 'rust-mode 'fill-column 100)",
    },
    Setting {
        name: FILL_SEPARATE_LIST_ITEMS,
        doc: "Whether filling treats each bullet or numbered item as a separate paragraph. \
              Non-nil by default: continuation lines hang under the item's text, including \
              inside comments. nil treats markers as ordinary words.\n\n\
              Example:\n\
              \x20 (setq fill-separate-list-items t)",
    },
    Setting {
        name: FILL_SEPARATE_COMMENTS,
        doc: "Whether each line comment is filled separately, instead of joining consecutive \
              lines with the same opener. nil by default. Independent of \
              `fill-separate-list-items'; `fill-comments' must be non-nil.\n\n\
              Example:\n\
              \x20 (setq fill-separate-comments t)",
    },
    Setting {
        name: FILL_COMMENTS,
        doc: "Whether filling and automatic filling may change line comments in modes \
              that declare comment syntax. Non-nil by default; nil leaves them alone. \
              Code is always left alone.\n\n\
              Example:\n\
              \x20 (setq fill-comments nil)",
    },
    Setting {
        name: crate::ui::fill_column::SHOW_GUIDE,
        doc: "Whether buffers with `auto-fill-mode' or `auto-fill-column' enabled show a \
              vertical guide at their effective `fill-column'. Non-nil by default. \
              Only empty display cells are replaced; the buffer text is unchanged.\n\n\
              Example:\n\
              \x20 (setq auto-fill-column-show-guide nil)",
    },
    Setting {
        name: crate::ui::fill_column::GUIDE_CHARACTER,
        doc: "The vertical fill guide's character, as a one-character string. The default \
              is \"│\". Control characters, whitespace, combining characters, wide characters \
              and other values fall back to the default. Its face is `fill-column-indicator'.\n\n\
              Example:\n\
              \x20 (setq auto-fill-column-character \"|\")",
    },
    Setting {
        name: DELETE_TRAILING_LINES,
        doc: "Whether `delete-trailing-whitespace', on a whole buffer, and \
              `whitespace-cleanup-mode', on a save, also delete the blank lines at the buffer's \
              end, all but the newline that ends the last line of text.",
    },
    Setting {
        name: TAB_WIDTH,
        doc: "How many columns a Tab is worth: how far Tab indents, and how far a Tab in the \
              text reaches when it is drawn, to the next multiple of it. A mode can have its \
              own, as a tab-width property, which wins over this in its buffers. Drawn, a Tab \
              is between 1 and 64 columns wide, and anything but a number counts as 4.\n\n\
              Example:\n\
              \x20 (setq tab-width 8)\n\
              \x20 (put 'makefile-mode 'tab-width 8)",
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

/// The name of the Lisp variable listing where `require` and `eval-file` look
/// for a module.
pub const LISP_PATH: &str = "lisp-path";

/// The name of the Lisp variable listing where the themes are looked for.
pub const THEME_PATH: &str = "theme-path";

/// The name of the Lisp variable that arms the echo area's timeout.
pub const ECHO_MESSAGE_TIMEOUT: &str = "echo-message-timeout";

/// How long an echo message stays on screen when nothing sets
/// [`ECHO_MESSAGE_TIMEOUT`] to something else.
pub const DEFAULT_ECHO_MESSAGE_TIMEOUT: f64 = 5.0;

/// The name of the Lisp variable holding how many rows the minibuffer's
/// window takes.
pub const MINIBUFFER_LINES: &str = "minibuffer-lines";

/// How many rows the minibuffer's window takes when nothing says otherwise:
/// one, as in Emacs.
pub const DEFAULT_MINIBUFFER_LINES: f64 = 1.0;

// TODO(MIN-10)
/// The name of the Lisp variable holding how wide a prompt is.
pub const MINIBUFFER_WIDTH: &str = "minibuffer-width";

// TODO(MIN-10)
/// The name of the Lisp variable holding how high a prompt is.
pub const MINIBUFFER_HEIGHT: &str = "minibuffer-height";

// TODO(MIN-10)
/// How wide a prompt is when nothing says otherwise: wide enough for a path,
/// and narrow enough to read as a dialogue rather than as part of the frame.
///
/// Clamped, like the height, to what the terminal holds: a preference, not a
/// promise, and a 40-column terminal gets a 38-column prompt rather than one
/// running off its edge.
pub const DEFAULT_MINIBUFFER_WIDTH: f64 = 60.0;

// TODO(MIN-10)
/// How high a prompt is when nothing says otherwise: three rows, the line being
/// typed between two borders.
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

/// The name of the Lisp variable holding how much one step of background work
/// may evaluate.
pub const WORKER_FUEL: &str = "worker-fuel";

/// The name of the Lisp variable naming the function M-x completes with.
pub const COMMAND_COMPLETION_FUNCTION: &str = "*command-completion-function*";

/// The name of the Lisp variable naming the function that completes every
/// argument a command asks for.
pub const COMMAND_ARG_COMPLETION_FUNCTION: &str = "*command-arg-completion-function*";

/// The name of the Lisp variable holding the mode-line format.
pub const MODE_LINE_FORMAT: &str = "mode-line-format";

/// What a window's status line says when nothing sets [`MODE_LINE_FORMAT`].
pub const DEFAULT_MODE_LINE_FORMAT: &str = " %*%i %b   %m%n %s  L%l C%c   %p ";

/// The name of the Lisp variable that turns the mouse on.
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

/// The name of the Lisp variable holding the character the separators between
/// windows are drawn with.
pub const WINDOW_SEPARATOR: &str = "window-separator";

/// The name of the Lisp variable, and of the property of a mode, that says
/// how many columns a Tab is worth.
pub const TAB_WIDTH: &str = "tab-width";

/// The name of the Lisp variable that says which numbers the gutter shows
/// where `display-line-numbers-mode` is on.
///
/// `'relative` counts from point, and anything else is ordinary absolute
/// numbering -- the answers Emacs gives to the same name, so somebody who
/// already knows one does not have to learn the other.
pub const DISPLAY_LINE_NUMBERS_TYPE: &str = "display-line-numbers-type";

/// The name of the Lisp variable holding the command being run, which the
/// hooks after it read.
pub const THIS_COMMAND: &str = "this-command";

/// The name of the Lisp variable holding how far point may leave a window
/// and have it scroll just enough, rather than centre point's line.
pub const SCROLL_CONSERVATIVELY: &str = "scroll-conservatively";

/// How far point may leave a window before its line is centred, when nothing
/// says otherwise: one line, so that `C-n` and `C-p` at an edge scroll line by
/// line, and anything that jumps further centres.
pub const DEFAULT_SCROLL_CONSERVATIVELY: f64 = 1.0;

/// Returns how far point may leave a window before its line is centred, as
/// Lisp has it now: `scroll-conservatively` when it is a number of at least
/// zero, the default otherwise. Read before composition takes any lock, as
/// the gutter's settings are.
pub(super) fn scroll_conservatively<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> usize {
    match env.get_variable(SCROLL_CONSERVATIVELY) {
        // Capped: anything past the threshold means the same, never centre.
        Some(ELispExp::Number(n)) if n.is_finite() && n >= 0.0 => {
            (n as usize).min(crate::ui::NEVER_RECENTER + 1)
        }
        _ => DEFAULT_SCROLL_CONSERVATIVELY as usize,
    }
}

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

/// What the separators are drawn with when nothing sets [`WINDOW_SEPARATOR`]:
/// a line, `│`.
pub const DEFAULT_WINDOW_SEPARATOR: char = '\u{2502}';

/// Returns VALUE as a Tab's width, or `None` when it is not a number of
/// columns: what a mode's property and the variable are both read with.
fn tab_width_value<B: BufferTrait>(value: Option<ELispExp<B>>) -> Option<usize> {
    match value {
        Some(ELispExp::Number(n)) if n.is_finite() && n >= 1.0 => {
            Some((n as usize).min(crate::ui::layout::MAX_TAB_WIDTH))
        }
        _ => None,
    }
}

/// Returns how many columns a Tab reaches across in each of MODES, as Lisp
/// defines it now: the mode's own `tab-width` property, or the variable, or
/// [`DEFAULT_TAB_WIDTH`](crate::ui::layout::DEFAULT_TAB_WIDTH).
pub(crate) fn tab_widths<B: BufferTrait>(
    env: &Arc<Env<EditorState<B>>>,
    modes: impl IntoIterator<Item = String>,
) -> crate::ui::layout::TabWidths {
    let default = tab_width_value(env.get_variable(TAB_WIDTH))
        .unwrap_or(crate::ui::layout::DEFAULT_TAB_WIDTH);
    let by_mode = modes
        .into_iter()
        .filter_map(|mode| {
            let width = tab_width_value(env.get_property(&mode, TAB_WIDTH))?;
            Some((mode, width))
        })
        .collect();
    crate::ui::layout::TabWidths { default, by_mode }
}

/// Returns the format of the status line, as Lisp defines it now.
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

/// Returns whether the editor reads the mouse, as Lisp defines it now.
///
/// Off unless something says otherwise, and the `init.lisp` written for
/// somebody who has none says otherwise. A setting, because it costs
/// something: a terminal reporting the mouse to the editor no longer selects
/// with it, so turning it on trades the terminal's copy and paste for the
/// editor's -- the right trade for most, and a surprise for whoever was never
/// asked.
pub fn mouse_mode<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.get_variable(MOUSE_MODE)
        .is_some_and(|value| !value.is_nil())
}

/// Returns how the gutter is set, as Lisp defines it now: the numbers of a
/// window whose buffer has `display-line-numbers-mode` on.
///
/// Read here rather than inside composition for the reason the mode-line
/// format is: the environment has locks of its own, and composition runs with
/// the window and buffer locks already held.
pub(super) fn line_numbers<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> GutterSpec {
    let numbers = match env.get_variable(DISPLAY_LINE_NUMBERS_TYPE) {
        // A symbol, because that is how it is written -- but a string too,
        // since `(setq display-line-numbers-type "relative")` is the mistake
        // everybody makes once and there is nothing else it could have meant.
        Some(ELispExp::Symbol(name)) if &**name == "relative" => LineNumbers::Relative,
        Some(ELispExp::String(name)) if &**name == "relative" => LineNumbers::Relative,
        // Any other value gets the ordinary kind. Guessing at a misspelt
        // `'reltaive` would be worse than showing numbers that are merely not
        // the ones asked for: the mistake is visible either way, and this way
        // the feature still works.
        _ => LineNumbers::Absolute,
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

/// Returns the character the separators are drawn with, as Lisp defines it
/// now: the first of a string, a blank for nil or an empty string, and the
/// default for anything else.
pub(super) fn window_separator<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> char {
    match env.get_variable(WINDOW_SEPARATOR) {
        Some(ELispExp::String(text)) => text.chars().next().unwrap_or(' '),
        Some(value) if value.is_nil() => ' ',
        _ => DEFAULT_WINDOW_SEPARATOR,
    }
}

/// Returns how long a message stays in the echo area, as Lisp defines it now,
/// or `None` for never expiring.
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

/// The name of the Lisp variable that sends a visit to an entry of a listing
/// to the listing's own window.
pub const RESULTS_VISIT_SAME_WINDOW: &str = "results-visit-same-window";

/// The name of the Lisp variable that lets a new run take the place of the
/// newest listing of the same producer.
pub const RESULTS_REUSE_BUFFER: &str = "results-reuse-buffer";

/// The name of the Lisp variable saying where a listing is shown: `bottom` or
/// `same-window`.
pub const RESULTS_DISPLAY_LOCATION: &str = "results-display-location";

/// Returns whether `results-visit-same-window` is set.
pub(super) fn results_visit_same_window<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.get_variable(RESULTS_VISIT_SAME_WINDOW)
        .is_some_and(|value| value.is_truthy())
}

/// Returns whether `results-reuse-buffer` is set.
pub(super) fn results_reuse_buffer<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.get_variable(RESULTS_REUSE_BUFFER)
        .is_some_and(|value| value.is_truthy())
}

/// Returns whether `results-display-location` is `same-window`; anything else
/// is `bottom`.
pub(super) fn results_display_same_window<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    matches!(
        env.get_variable(RESULTS_DISPLAY_LOCATION),
        Some(ELispExp::Symbol(value) | ELispExp::String(value))
            if value.as_ref() == "same-window"
    )
}
