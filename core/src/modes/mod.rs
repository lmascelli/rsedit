use crate::{ELispExp, buffer::BufferTrait, input::KeyEvent};
use std::collections::HashMap;

pub mod syntax;
pub use syntax::SyntaxRule;

/// A major mode is a collection of rules that apply to a specific
/// kind of buffers like specific programming language, special text
/// files or special buffers like the minibuffer or the repl buffer.
/// It provides custom keymap, syntax highlighting rules and hook
/// that will be called befor or after some events.
#[derive(Clone, Debug)]
pub struct MajorMode<B: BufferTrait> {
    pub name: String,
    pub keymaps: HashMap<KeyEvent, String>,
    pub syntax_rules: Vec<SyntaxRule>,
    pub hooks: HashMap<String, Vec<ELispExp<B>>>,
}

impl<B: BufferTrait> MajorMode<B> {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            keymaps: HashMap::new(),
            syntax_rules: vec![],
            hooks: HashMap::new(),
        }
    }
}
