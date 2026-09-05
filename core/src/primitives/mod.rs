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
        ) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
            $body
        }
    };
}

mod buffers;
mod commands;
mod edits;
mod general;
mod io;
mod modes;
mod ui;

pub fn install_primitives<B: BufferTrait>(
    state: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
) {
    macro_rules! insert_fn {
        ($name:literal, $func:path) => {
            env.set_function($name.into(), ELispExp::primitive($func, None));
        };
        ($name:literal, $func:path, $doc:expr) => {
            env.set_function($name.into(), ELispExp::primitive($func, Some($doc.into())));
        };
    }

    /// Install a primitive *and* register it as a command in one place, so a
    /// built-in command's argument spec sits next to its implementation and
    /// its docstring rather than in a `.lisp` file that could drift or fail to
    /// load. User-defined commands take the other route, `defcommand`, which
    /// expands to a `defun` plus a `register-command` call; both end up in the
    /// same registry, and neither knows about the other.
    ///
    /// Specs are parsed here, at boot, so a malformed one is a startup panic
    /// rather than a surprise the first time someone runs the command.
    macro_rules! insert_cmd {
        ($name:literal, $func:path, $specs:expr, $doc:expr) => {
            insert_fn!($name, $func, $doc);
            state.register_command(
                $name,
                $specs
                    .iter()
                    .map(|code: &&str| {
                        $crate::commands::ArgSpec::parse(code)
                            .unwrap_or_else(|why| panic!("built-in command `{}`: {why}", $name))
                    })
                    .collect(),
            );
        };
    }

    // The registry itself. These are plain functions, not commands: they are
    // how Lisp inspects and extends the command set, not things a user runs
    // from M-x.
    insert_fn!(
        "register-command",
        commands::register_command,
        commands::REGISTER_COMMAND_DOC
    );
    insert_fn!("commandp", commands::commandp, commands::COMMANDP_DOC);
    insert_fn!(
        "command-args",
        commands::command_args,
        commands::COMMAND_ARGS_DOC
    );
    insert_fn!(
        "all-commands",
        commands::all_commands,
        commands::ALL_COMMANDS_DOC
    );
    insert_fn!(
        "execute-extended-command",
        commands::execute_extended_command,
        commands::EXECUTE_EXTENDED_COMMAND_DOC
    );
    insert_fn!(
        "command-completions",
        commands::command_completions,
        commands::COMMAND_COMPLETIONS_DOC
    );
    insert_cmd!(
        "command-execute-prompt",
        commands::command_execute_prompt,
        [] as [&str; 0],
        commands::COMMAND_EXECUTE_PROMPT_DOC
    );
    insert_fn!(
        "all-buffer-names",
        commands::all_buffer_names,
        commands::ALL_BUFFER_NAMES_DOC
    );
    insert_fn!(
        "call-interactively",
        commands::call_interactively,
        commands::CALL_INTERACTIVELY_DOC
    );

    insert_cmd!("quit", general::quit, [] as [&str; 0], general::QUIT_DOC);
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
    insert_cmd!(
        "insert-newline",
        edits::insert_newline,
        [] as [&str; 0],
        edits::INSERT_NEWLINE_DOC
    );
    insert_cmd!(
        "delete-backward-char",
        edits::delete_backward_char,
        [] as [&str; 0],
        edits::DELETE_BACKWARD_CHAR_DOC
    );
    insert_cmd!(
        "backward-char",
        edits::backward_char,
        [] as [&str; 0],
        edits::BACKWARD_CHAR_DOC
    );
    insert_cmd!(
        "forward-char",
        edits::forward_char,
        [] as [&str; 0],
        edits::FORWARD_CHAR_DOC
    );
    insert_cmd!(
        "previous-line",
        edits::previous_line,
        [] as [&str; 0],
        edits::PREVIOUS_LINE_DOC
    );
    insert_cmd!(
        "next-line",
        edits::next_line,
        [] as [&str; 0],
        edits::NEXT_LINE_DOC
    );
    insert_cmd!(
        "find-file",
        io::find_file,
        ["fFind file: "],
        io::FIND_FILE_DOC
    );
    insert_cmd!(
        "save-buffer",
        io::save_buffer,
        [] as [&str; 0],
        io::SAVE_BUFFER_DOC
    );
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
