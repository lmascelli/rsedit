use crate::{
    BufferTrait, ELispExp, EditorState,
    input::{KeyCode, KeyEvent, KeyModifiers},
    lisp::{Env, EvalError, LispContext},
    modes::{MajorMode, SyntaxRegion, SyntaxRule},
    ui::Face,
};

/// Parse a whole binding: one or more keys, separated by spaces.
///
/// A part that fails to parse fails the whole sequence rather than being
/// dropped. Silently binding `C-x` when `"C-x C-"` was written would define a
/// prefix that swallows the real binding underneath it.
fn parse_key_sequence(seq: &str) -> Option<Vec<KeyEvent>> {
    let parts: Vec<&str> = seq.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let keys: Vec<KeyEvent> = parts.iter().filter_map(|part| parse_key(part)).collect();
    if keys.len() != parts.len() {
        return None;
    }
    Some(keys)
}

/// Parse one key: optional `C-`, `M-` or `C-M-` modifiers, then a key name.
fn parse_key(seq: &str) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers::default();
    let mut chars = seq.chars().peekable();

    // Longest prefix first. Tested after `C-` this branch could never run,
    // since every `C-M-x` starts with `C-`, so `C-M-` bindings silently became
    // plain `C-` ones with a stray `M-` left in the key name -- which then
    // failed to parse and dropped the binding on the floor.
    if let Some(rest) = seq.strip_prefix("C-M-") {
        modifiers.ctrl = true;
        modifiers.alt = true;
        chars = rest.chars().peekable();
    } else if let Some(rest) = seq.strip_prefix("C-") {
        modifiers.ctrl = true;
        chars = rest.chars().peekable();
    } else if let Some(rest) = seq.strip_prefix("M-") {
        modifiers.alt = true;
        chars = rest.chars().peekable();
    }

    let key_code = match chars.collect::<String>().as_str() {
        "<ret>" | "<Return>" => KeyCode::Enter,
        "<esc>" | "<Escape>" => KeyCode::Esc,
        "tab" | "<Tab>" => KeyCode::Tab,
        "<backspace>" => KeyCode::Backspace,
        // Spelt out because a bare space is impossible to see in a key name,
        // and `C-<space>` is how `set-mark` is bound.
        "<space>" | " " => KeyCode::Char(' '),
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
pub(crate) mod edits;
mod general;
pub(crate) mod io;
mod modes;
mod region;
mod theme;
mod ui;
mod windows;

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
    insert_fn!(
        "define-repeat-key",
        general::define_repeat_key,
        general::DEFINE_REPEAT_KEY_DOC
    );
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
    insert_fn!(
        "add-syntax-region",
        modes::add_syntax_region,
        modes::ADD_SYNTAX_REGION_DOC
    );
    insert_fn!(
        "add-auto-mode",
        modes::add_auto_mode,
        modes::ADD_AUTO_MODE_DOC
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
        ["p"],
        edits::BACKWARD_CHAR_DOC
    );
    insert_cmd!(
        "forward-char",
        edits::forward_char,
        ["p"],
        edits::FORWARD_CHAR_DOC
    );
    insert_cmd!(
        "previous-line",
        edits::previous_line,
        ["p"],
        edits::PREVIOUS_LINE_DOC
    );
    insert_cmd!(
        "delete-char",
        edits::delete_char,
        ["p"],
        edits::DELETE_CHAR_DOC
    );
    insert_cmd!(
        "kill-line",
        edits::kill_line,
        [] as [&str; 0],
        edits::KILL_LINE_DOC
    );
    insert_cmd!(
        "kill-whole-line",
        edits::kill_whole_line,
        [] as [&str; 0],
        edits::KILL_WHOLE_LINE_DOC
    );
    insert_cmd!("kill-word", edits::kill_word, ["p"], edits::KILL_WORD_DOC);
    insert_cmd!(
        "backward-kill-word",
        edits::backward_kill_word,
        ["p"],
        edits::BACKWARD_KILL_WORD_DOC
    );
    insert_cmd!(
        "kill-paragraph",
        edits::kill_paragraph,
        [] as [&str; 0],
        edits::KILL_PARAGRAPH_DOC
    );
    insert_cmd!(
        "backward-kill-paragraph",
        edits::backward_kill_paragraph,
        [] as [&str; 0],
        edits::BACKWARD_KILL_PARAGRAPH_DOC
    );
    insert_cmd!("undo", edits::undo, [] as [&str; 0], edits::UNDO_DOC);
    insert_cmd!("redo", edits::redo, [] as [&str; 0], edits::REDO_DOC);
    insert_cmd!(
        "undo-boundary",
        edits::undo_boundary,
        [] as [&str; 0],
        edits::UNDO_BOUNDARY_DOC
    );
    insert_cmd!(
        "set-undo-limit",
        edits::set_undo_limit,
        ["n:Undo limit in bytes: "],
        edits::SET_UNDO_LIMIT_DOC
    );

    // ---------------------------------------------------------------
    // The mark, the region, and the kill ring
    // ---------------------------------------------------------------
    insert_cmd!(
        "set-mark",
        region::set_mark,
        [] as [&str; 0],
        region::SET_MARK_DOC
    );
    insert_cmd!(
        "keyboard-quit",
        region::keyboard_quit,
        [] as [&str; 0],
        region::KEYBOARD_QUIT_DOC
    );
    insert_cmd!(
        "deactivate-mark",
        region::deactivate_mark,
        [] as [&str; 0],
        region::DEACTIVATE_MARK_DOC
    );
    insert_cmd!(
        "exchange-point-and-mark",
        region::exchange_point_and_mark,
        [] as [&str; 0],
        region::EXCHANGE_POINT_AND_MARK_DOC
    );
    insert_cmd!(
        "mark-whole-buffer",
        region::mark_whole_buffer,
        [] as [&str; 0],
        region::MARK_WHOLE_BUFFER_DOC
    );
    insert_cmd!(
        "kill-region",
        region::kill_region,
        [] as [&str; 0],
        region::KILL_REGION_DOC
    );
    insert_cmd!(
        "kill-ring-save",
        region::kill_ring_save,
        [] as [&str; 0],
        region::KILL_RING_SAVE_DOC
    );
    insert_cmd!("yank", region::yank, [] as [&str; 0], region::YANK_DOC);
    insert_cmd!(
        "yank-pop",
        region::yank_pop,
        [] as [&str; 0],
        region::YANK_POP_DOC
    );
    insert_cmd!(
        "set-kill-ring-max",
        region::set_kill_ring_max,
        ["n:Kill ring size: "],
        region::SET_KILL_RING_MAX_DOC
    );
    // ---------------------------------------------------------------
    // Windows
    // ---------------------------------------------------------------
    insert_cmd!(
        "split-window-below",
        windows::split_window_below,
        [] as [&str; 0],
        windows::SPLIT_WINDOW_BELOW_DOC
    );
    insert_cmd!(
        "split-window-right",
        windows::split_window_right,
        [] as [&str; 0],
        windows::SPLIT_WINDOW_RIGHT_DOC
    );
    insert_cmd!(
        "delete-window",
        windows::delete_window,
        [] as [&str; 0],
        windows::DELETE_WINDOW_DOC
    );
    insert_cmd!(
        "delete-other-windows",
        windows::delete_other_windows,
        [] as [&str; 0],
        windows::DELETE_OTHER_WINDOWS_DOC
    );
    insert_cmd!(
        "other-window",
        windows::other_window,
        ["p"],
        windows::OTHER_WINDOW_DOC
    );
    insert_fn!(
        "count-windows",
        windows::count_windows,
        windows::COUNT_WINDOWS_DOC
    );
    insert_fn!(
        "window-buffer",
        windows::window_buffer,
        windows::WINDOW_BUFFER_DOC
    );

    // ---------------------------------------------------------------
    // Faces and the theme
    // ---------------------------------------------------------------
    insert_cmd!(
        "set-face",
        theme::set_face,
        ["s:Face: ", "s:Foreground: ", "s:Background: "],
        theme::SET_FACE_DOC
    );
    insert_fn!("face-style", theme::face_style, theme::FACE_STYLE_DOC);
    insert_fn!("list-faces", theme::list_faces, theme::LIST_FACES_DOC);
    insert_fn!("list-colors", theme::list_colors, theme::LIST_COLORS_DOC);

    // Predicates and accessors rather than things to run from M-x.
    insert_fn!("mark", region::mark, region::MARK_DOC);
    insert_fn!(
        "use-region-p",
        region::use_region_p,
        region::USE_REGION_P_DOC
    );
    insert_fn!(
        "region-beginning",
        region::region_beginning,
        region::REGION_BEGINNING_DOC
    );
    insert_fn!("region-end", region::region_end, region::REGION_END_DOC);
    insert_fn!("kill-new", region::kill_new, region::KILL_NEW_DOC);
    insert_fn!(
        "current-kill",
        region::current_kill,
        region::CURRENT_KILL_DOC
    );
    insert_fn!(
        "kill-ring-length",
        region::kill_ring_length,
        region::KILL_RING_LENGTH_DOC
    );
    insert_cmd!(
        "beginning-of-line",
        edits::beginning_of_line,
        [] as [&str; 0],
        edits::BEGINNING_OF_LINE_DOC
    );
    insert_cmd!(
        "end-of-line",
        edits::end_of_line,
        [] as [&str; 0],
        edits::END_OF_LINE_DOC
    );
    insert_cmd!(
        "forward-word",
        edits::forward_word,
        ["p"],
        edits::FORWARD_WORD_DOC
    );
    insert_cmd!(
        "backward-word",
        edits::backward_word,
        ["p"],
        edits::BACKWARD_WORD_DOC
    );
    insert_cmd!(
        "forward-paragraph",
        edits::forward_paragraph,
        [] as [&str; 0],
        edits::FORWARD_PARAGRAPH_DOC
    );
    insert_cmd!(
        "backward-paragraph",
        edits::backward_paragraph,
        [] as [&str; 0],
        edits::BACKWARD_PARAGRAPH_DOC
    );
    insert_cmd!(
        "beginning-of-buffer",
        edits::beginning_of_buffer,
        [] as [&str; 0],
        edits::BEGINNING_OF_BUFFER_DOC
    );
    insert_cmd!(
        "end-of-buffer",
        edits::end_of_buffer,
        [] as [&str; 0],
        edits::END_OF_BUFFER_DOC
    );
    // The first built-in command that prompts for an argument, so M-x
    // goto-line asks for the number rather than failing on arity.
    insert_cmd!(
        "goto-line",
        edits::goto_line,
        ["nGoto line: "],
        edits::GOTO_LINE_DOC
    );
    insert_cmd!("next-line", edits::next_line, ["p"], edits::NEXT_LINE_DOC);
    insert_cmd!(
        "find-file",
        io::find_file,
        ["fFind file: "],
        io::FIND_FILE_DOC
    );
    insert_fn!("list-dir", io::list_dir, io::LIST_DIR_DOC);
    insert_fn!("match-list", io::match_list, io::MATCH_LIST_DOC);
    insert_fn!(
        "expand-file-name",
        io::expand_file_name,
        io::EXPAND_FILE_NAME_DOC
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
    insert_fn!(
        "with-current-buffer",
        buffers::with_current_buffer,
        buffers::WITH_CURRENT_BUFFER_DOC
    );

    // Point as a number. Plain functions rather than commands: nothing is
    // usefully reached by typing `M-x point', and everything that moves point
    // by a described amount -- a word, a line -- already is a command.
    insert_fn!("point", edits::point, edits::POINT_DOC);
    insert_fn!("point-min", edits::point_min, edits::POINT_MIN_DOC);
    insert_fn!("point-max", edits::point_max, edits::POINT_MAX_DOC);
    insert_fn!("goto-char", edits::goto_char, edits::GOTO_CHAR_DOC);

    // Incremental search. The four entry points are commands so that M-x
    // reaches them; the keys that answer the prompt are installed with
    // `isearch-mode` itself -- see `crate::isearch`.
    insert_cmd!(
        "isearch-forward",
        crate::isearch::isearch_forward,
        [] as [&str; 0],
        crate::isearch::ISEARCH_FORWARD_DOC
    );
    insert_cmd!(
        "isearch-backward",
        crate::isearch::isearch_backward,
        [] as [&str; 0],
        crate::isearch::ISEARCH_BACKWARD_DOC
    );
    insert_cmd!(
        "isearch-forward-regexp",
        crate::isearch::isearch_forward_regexp,
        [] as [&str; 0],
        crate::isearch::ISEARCH_FORWARD_REGEXP_DOC
    );
    insert_cmd!(
        "isearch-backward-regexp",
        crate::isearch::isearch_backward_regexp,
        [] as [&str; 0],
        crate::isearch::ISEARCH_BACKWARD_REGEXP_DOC
    );
}
