use crate::{ELispExp, buffer::BufferTrait, input::KeyEvent, ui::Face};
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

pub struct SyntaxSpan {
    pub start: usize,
    pub len: usize,
    pub face: Face,
}

pub fn compute_syntax_spans(text: &str, rules: &[SyntaxRule]) -> Vec<SyntaxSpan> {
    struct RawMatch {
        start: usize,
        len: usize,
        face: Face,
        priority: usize,
    }
    let mut all_matches = Vec::new();

    for (priority, rule) in rules.iter().enumerate() {
        for mat in rule.pattern.find_iter(text) {
            all_matches.push(RawMatch {
                start: mat.start(),
                len: mat.end() - mat.start(),
                face: rule.face.clone(),
                priority,
            });
        }
    }

    all_matches.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.start.cmp(&b.start))
    });
    let mut final_spans: Vec<SyntaxSpan> = Vec::new();

    for m in all_matches {
        let overlaps = final_spans.iter().any(|existing| {
            m.start < existing.start + existing.len && m.start + m.len > existing.start
        });
        if !overlaps {
            final_spans.push(SyntaxSpan {
                start: m.start,
                len: m.len,
                face: m.face,
            });
        }
    }

    final_spans.sort_by(|a, b| a.start.cmp(&b.start));
    final_spans
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
