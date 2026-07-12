use crate::{ELispExp, buffer::BufferTrait, input::KeyEvent};
use std::collections::HashMap;

/// A SyntaxRule is an association between a regex match and a face
/// that will specify the features of the rendered text. This way
/// a theme system can be implemented where a face is associated to
/// a color.
#[derive(Clone, Debug)]
pub struct SyntaxRule {
    pub pattern: regex::Regex,
    pub face: crate::ui::Face,
}

/// A major mode is a collection of rules that apply to a specific
/// kind of buffers like specific programming language, special text
/// files or special buffers like the minibuffer or the repl buffer.
/// It provides custom keymap, syntax highlighting rules and hook
/// that will be called befor or after some events.
#[derive(Clone, Debug)]
pub struct MajorMode<B: BufferTrait> {
    pub name: String,
    pub keymap: HashMap<KeyEvent, ELispExp<B>>,
    pub syntax_rules: Vec<SyntaxRule>,
    pub hooks: HashMap<String, Vec<ELispExp<B>>>,
}

impl<B: BufferTrait> MajorMode<B> {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            keymap: HashMap::new(),
            syntax_rules: vec![],
            hooks: HashMap::new(),
        }
    }
}
