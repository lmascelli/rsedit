use crate::{
    BufferTrait, ELispExp, EditorState,
    input::{KeyCode, KeyEvent, KeyModifiers},
    lisp::{Env, EvalError, LispContext},
    modes::{MajorMode, SyntaxRule},
    ui::Face,
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

mod buffers;
mod edits;
mod general;
mod io;
mod modes;
mod ui;

pub fn install_primitives<B: BufferTrait>(env: &std::sync::Arc<Env<EditorState<B>>>) {
    macro_rules! insert_fn {
        ($name:literal, $func:path) => {
            env.set_function($name.into(), ELispExp::primitive($func, None));
        };
        ($name:literal, $func:path, $doc:expr) => {
            env.set_function($name.into(), ELispExp::primitive($func, Some($doc.into())));
        };
    }

    insert_fn!("quit", general::quit, general::QUIT_DOC);
    insert_fn!("eval-file", general::eval_file, general::EVAL_FILE_DOC);
    insert_fn!("define-key", general::define_key, general::DEFINE_KEY_DOC);
    insert_fn!("log", general::log, general::LOG_DOC);
    insert_fn!("all-logs", general::all_logs, general::ALL_LOGS_DOC);
    insert_fn!("backtrace", general::backtrace, general::BACKTRACE_DOC);
    insert_fn!(
        "set-echo-message",
        general::set_echo_message,
        general::SET_ECHO_MESSAGE_DOC
    );
    insert_fn!("make-mode", modes::make_mode, modes::MAKE_MODE_DOC);
    insert_fn!("add-hook", modes::add_hook, modes::ADD_HOOK_DOC);
    insert_fn!(
        "add-syntax-rule",
        modes::add_syntax_rule,
        modes::ADD_SYNTAX_RULE_DOC
    );
    insert_fn!("self-insert", edits::self_insert, edits::SELF_INSERT_DOC);
    insert_fn!(
        "insert-newline",
        edits::insert_newline,
        edits::INSERT_NEWLINE_DOC
    );
    insert_fn!(
        "delete-backward-char",
        edits::delete_backward_char,
        edits::DELETE_BACKWARD_CHAR_DOC
    );
    insert_fn!(
        "backward-char",
        edits::backward_char,
        edits::BACKWARD_CHAR_DOC
    );
    insert_fn!("forward-char", edits::forward_char, edits::FORWARD_CHAR_DOC);
    insert_fn!(
        "previous-line",
        edits::previous_line,
        edits::PREVIOUS_LINE_DOC
    );
    insert_fn!("next-line", edits::next_line, edits::NEXT_LINE_DOC);
    insert_fn!("find-file", io::find_file, io::FIND_FILE_DOC);
    insert_fn!("save-buffer", io::save_buffer, io::SAVE_BUFFER_DOC);
    insert_fn!(
        "make-floating-window",
        ui::make_floating_window,
        ui::MAKE_FLOATING_WINDOW_DOC
    );
    insert_fn!("close-floating-window", ui::close_floating_window);
    insert_fn!(
        "switch-to-buffer",
        buffers::switch_to_buffer,
        buffers::SWITCH_TO_BUFFER_DOC
    );
    insert_fn!(
        "current-buffer",
        buffers::current_buffer,
        buffers::CURRENT_BUFFER_DOC
    );
    insert_fn!(
        "buffer-create",
        buffers::buffer_create,
        buffers::BUFFER_CREATE_DOC
    );
    insert_fn!(
        "close-buffer",
        buffers::close_buffer,
        buffers::CLOSE_BUFFER_DOC
    );
    insert_fn!(
        "buffer-string",
        buffers::buffer_string,
        buffers::BUFFER_STRING_DOC
    );
    insert_fn!(
        "clear-buffer",
        buffers::clear_buffer,
        buffers::CLEAR_BUFFER_DOC
    );
}
