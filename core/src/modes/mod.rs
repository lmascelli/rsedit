use crate::{ELispExp, buffer::BufferTrait, input::Keymap};
use std::collections::HashMap;

pub mod highlighter;
pub mod syntax;
pub use syntax::{Grammar, SyntaxRegion, SyntaxRule, SyntaxSpan, SyntaxState, highlight_line};

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
}

impl<B: BufferTrait> MajorMode<B> {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            keymaps: Keymap::new(),
            grammar: Grammar::default(),
            hooks: HashMap::new(),
        }
    }
}
