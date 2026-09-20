use crate::{ELispExp, buffer::BufferTrait, input::Keymap};
use std::collections::HashMap;

pub mod highlighter;
pub mod syntax;
pub use syntax::{
    CommentStyle, Grammar, StringStyle, SyntaxClass, SyntaxRegion, SyntaxRule, SyntaxSpan,
    SyntaxState, SyntaxTable, highlight_line,
};
pub mod sexp;

/// A major mode is a collection of rules that apply to a specific
/// kind of buffers like specific programming language, special text
/// files or special buffers like the minibuffer or the repl buffer.
/// It provides custom keymap, syntax highlighting rules and hook
/// that will be called befor or after some events.
#[derive(Clone, Debug)]
pub struct MajorMode<B: BufferTrait> {
    pub name: String,
    pub keymaps: Keymap<B>,
    /// How this mode colours its language. See [`Grammar`].
    pub grammar: Grammar,
    pub hooks: HashMap<String, Vec<ELispExp<B>>>,
    /// Where completions come from in a buffer in this mode, tried before the
    /// global ones.
    ///
    /// Scoped to the mode rather than held in a variable, for the reason
    /// `hooks` is: a variable would be one list shared by every buffer, and
    /// what can be completed in a Lisp buffer is not what can be completed in
    /// a directory listing. See `crate::primitives::completion`.
    pub completion_functions: Vec<ELispExp<B>>,

    /// Used to look for pairing element of the text like () [] {} "" etc...
    pub syntax_table: Option<SyntaxTable>,
}

impl<B: BufferTrait> MajorMode<B> {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            keymaps: Keymap::new(),
            grammar: Grammar::default(),
            hooks: HashMap::new(),
            completion_functions: Vec::new(),
            syntax_table: None,
        }
    }
}
