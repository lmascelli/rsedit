//! What a major mode is, and how its text is read: the grammar that colours
//! it, the syntax table that says what its characters mean, and the scanner
//! that reads the table to find its expressions.
//!
//! Design: `doc/architettura-editor.typ, <modi>`.
use crate::{ELispExp, buffer::BufferTrait, input::Keymap};
use std::collections::HashMap;

pub mod grammar;
pub mod sexp;
pub mod syntax_table;
pub use grammar::{Grammar, SyntaxRegion, SyntaxRule, SyntaxSpan, SyntaxState, highlight_line};
pub use syntax_table::{CommentStyle, StringStyle, SyntaxClass, SyntaxTable};

/// A mode: the keymap, grammar, hooks, completion sources and syntax table
/// of the buffers in it, under the mode's name. A major mode is what a buffer
/// is in, one at a time; a minor mode, one with a lighter, is turned on and
/// off in a buffer on top of it, and lends it its keymap and its hooks.
#[derive(Clone, Debug)]
pub struct MajorMode<B: BufferTrait> {
    /// The name the mode is registered and asked for under.
    pub name: String,
    /// The mode's bindings, consulted before the global ones.
    pub keymaps: Keymap<B>,
    /// How this mode colours its language. See [`Grammar`].
    pub grammar: Grammar,
    /// The functions of each of the mode's hooks, by the hook's name.
    pub hooks: HashMap<String, Vec<ELispExp<B>>>,
    /// Where completions come from in a buffer in this mode, tried before the
    /// global ones. See `crate::primitives::completion`.
    pub completion_functions: Vec<ELispExp<B>>,
    /// What the scanner reads to find this mode's expressions, or `None` for
    /// the default table.
    pub syntax_table: Option<SyntaxTable>,
    /// For a minor mode, what the status line shows while it is on, which may
    /// be empty; `None` for a major mode.
    pub lighter: Option<String>,
}

impl<B: BufferTrait> MajorMode<B> {
    /// Returns a mode called NAME with nothing of its own: no bindings,
    /// grammar, hooks or completion sources, and the default syntax table.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            keymaps: Keymap::new(),
            grammar: Grammar::default(),
            hooks: HashMap::new(),
            completion_functions: Vec::new(),
            syntax_table: None,
            lighter: None,
        }
    }

    /// Returns whether this is a minor mode.
    pub fn is_minor(&self) -> bool {
        self.lighter.is_some()
    }
}
