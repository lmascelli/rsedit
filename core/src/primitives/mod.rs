use crate::{
    BufferTrait, ELispExp, EditorState,
    input::{KeyCode, KeyEvent, KeyModifiers, Keymap, OnUnbound, TransientKeymap},
        modes::{CommentStyle, MajorMode, SyntaxClass, SyntaxRegion, SyntaxRule, SyntaxTable},
    ui::Face,
};

use risp::{Env, EvalError, LispContext, LispPrimitive, exact_arity};


/// Parse a whole binding: one or more keys, separated by spaces.
///
/// A part that fails to parse fails the whole sequence rather than being
/// dropped. Silently binding `C-x` when `"C-x C-"` was written would define a
/// prefix that swallows the real binding underneath it.
pub(crate) fn parse_key_sequence(seq: &str) -> Option<Vec<KeyEvent>> {
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

/// Parse one key: optional `C-`, `M-`, `C-M-` or `S-` modifiers, then a key
/// name.
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
    } else if let Some(rest) = seq.strip_prefix("S-") {
        // For keys that are not characters -- `S-<tab>`. A shifted character
        // arrives as its own character, so `S-a` parses but never fires.
        modifiers.shift = true;
        chars = rest.chars().peekable();
    }

    let key_code = match chars.collect::<String>().as_str() {
        "<ret>" | "<Return>" => KeyCode::Enter,
        "<esc>" | "<Escape>" => KeyCode::Esc,
        "tab" | "<tab>" | "<Tab>" => KeyCode::Tab,
        // What terminals call Shift-Tab, accepted under that name too.
        "<backtab>" => {
            modifiers.shift = true;
            KeyCode::Tab
        }
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

pub(crate) mod args;
mod ask;
mod buffers;
mod commands;
mod comments;
mod completion;
pub(crate) mod edits;
mod general;
mod help;
pub(crate) mod io;
mod isearch;
mod macros;
mod match_data;
mod minibuffer;
mod modes;
pub(crate) mod mouse;
mod overlays;
mod rectangle;
mod region;
mod replace;
pub(crate) mod results;
mod scan;
mod shell;
mod theme;
mod ui;
mod verbs;
mod virtual_text;
mod windows;
mod workers;

/// What a primitives module registers into.
///
/// A value, where this used to be two `macro_rules!` written *inside*
/// `install_primitives`. That is why all 277 registrations lived in that one
/// function: a macro declared in a function body can only be called from that
/// body, so every module's registrations had to be written there, two files
/// away from the code they name. A value can be handed to twenty-five modules,
/// and each one registers its own.
pub(crate) struct Registry<'a, B: BufferTrait> {
    state: &'a EditorState<B>,
    env: &'a std::sync::Arc<Env<EditorState<B>>>,
}

impl<B: BufferTrait> Registry<'_, B> {
    /// Bind NAME to a primitive Lisp can call.
    ///
    /// DOC must begin with the primitive's own call -- `(name ARGS): what it
    /// does` -- because a primitive has no parameter list anything can read,
    /// so that first line *is* the signature the help commands show.
    /// `tests::doc_faithfulness_tests` checks that over every primitive there
    /// is, along with the docstring simply being there.
    pub(crate) fn function(
        &self,
        name: &str,
        pointer: LispPrimitive<EditorState<B>>,
        doc: &'static str,
    ) {
        self.env
            .set_function(name.into(), ELispExp::primitive(pointer, Some(doc.into())));
    }

    /// Bind NAME, and register it as a command `M-x` can reach.
    ///
    /// One call rather than two so that a built-in command's argument spec
    /// sits next to its implementation and its docstring, rather than in a
    /// `.lisp` file that could drift or fail to load. User-defined commands
    /// take the other route, `defcommand`, which expands to a `defun` plus a
    /// `register-command` call; both end up in the same registry, and neither
    /// knows about the other.
    ///
    /// SPECS are parsed here, at boot, so a malformed one is a startup panic
    /// rather than a surprise the first time somebody runs the command.
    pub(crate) fn command(
        &self,
        name: &str,
        pointer: LispPrimitive<EditorState<B>>,
        specs: &[&str],
        doc: &'static str,
    ) {
        self.function(name, pointer, doc);
        self.state.register_command(
            name,
            specs
                .iter()
                .map(|code| {
                    crate::commands::ArgSpec::parse(code)
                        .unwrap_or_else(|why| panic!("built-in command `{name}`: {why}"))
                })
                .collect(),
        );
    }
}

/// Install every primitive the editor provides.
///
/// Each module registers its own, through [`Registry`]; this is the list of
/// modules and nothing else. Alphabetical, because the order genuinely does
/// not matter -- each registration is an insertion into a map under a name no
/// other registration uses -- so alphabetical is the order that makes a
/// missing module visible.
pub fn install_primitives<B: BufferTrait>(
    state: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
) {
    let into = Registry { state, env };
    ask::install(&into);
    buffers::install(&into);
    commands::install(&into);
    comments::install(&into);
    completion::install(&into);
    edits::install(&into);
    general::install(&into);
    help::install(&into);
    io::install(&into);
    isearch::install(&into);
    macros::install(&into);
    match_data::install(&into);
    minibuffer::install(&into);
    modes::install(&into);
    mouse::install(&into);
    overlays::install(&into);
    rectangle::install(&into);
    region::install(&into);
    replace::install(&into);
    results::install(&into);
    scan::install(&into);
    shell::install(&into);
    theme::install(&into);
    ui::install(&into);
    verbs::install(&into);
    virtual_text::install(&into);
    windows::install(&into);
    workers::install(&into);
}
