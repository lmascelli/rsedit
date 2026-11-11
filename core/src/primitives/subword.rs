//! Subword boundaries shared by word motion and killing, with literal separators.
//! Design: `doc/architettura-editor.typ, <movimento>`.
use super::*;
use crate::buffer::Buffer;
use std::sync::Arc;

const MODE: &str = "subword-mode";
const SEPARATORS: &str = "subword-separators";

pub(super) fn separators<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> String {
    match env.get_variable(SEPARATORS) {
        Some(ELispExp::String(text)) => text.to_string(),
        _ => "_".into(),
    }
}

fn local_separators<'a, B: BufferTrait>(buf: &'a Buffer<B>, fallback: &'a str) -> &'a str {
    match buf.data::<ELispExp<B>>("lisp:subword-separators") {
        Some(ELispExp::String(text)) => text,
        _ => fallback,
    }
}

fn constituent(c: char, separators: &str) -> bool {
    (super::movement::is_word_char(c) || c == '-') && !separators.contains(c)
}

fn boundary<B: BufferTrait>(text: &B, at: usize) -> bool {
    let (Some(left), Some(right)) = (at.checked_sub(1).and_then(|i| text.at(i)), text.at(at))
    else {
        return false;
    };
    (left.is_lowercase() && right.is_uppercase())
        || (left.is_alphabetic() && right.is_numeric())
        || (left.is_numeric() && right.is_alphabetic())
        || (left.is_uppercase()
            && right.is_uppercase()
            && text.at(at + 1).is_some_and(char::is_lowercase))
}

/// Returns a subword destination when the mode is on, an ordinary word otherwise.
pub(crate) fn forward<B: BufferTrait>(
    buf: &Buffer<B>,
    from: usize,
    count: usize,
    separators: &str,
) -> usize {
    if !buf.minor_mode_enabled(MODE) {
        return super::movement::word_forward(&buf.text, from, count);
    }
    let separators = local_separators(buf, separators);
    let mut pos = from.min(buf.text.len());
    for _ in 0..count {
        if pos >= buf.text.len() {
            break;
        }
        while pos < buf.text.len() && !buf.text.at(pos).is_some_and(|c| constituent(c, separators))
        {
            pos += 1;
        }
        if pos < buf.text.len() {
            pos += 1;
        }
        while pos < buf.text.len()
            && buf.text.at(pos).is_some_and(|c| constituent(c, separators))
            && !boundary(&buf.text, pos)
        {
            pos += 1;
        }
    }
    pos
}

/// Returns a subword destination backward, with the same boundaries as forward.
pub(crate) fn backward<B: BufferTrait>(
    buf: &Buffer<B>,
    from: usize,
    count: usize,
    separators: &str,
) -> usize {
    if !buf.minor_mode_enabled(MODE) {
        return super::movement::word_backward(&buf.text, from, count);
    }
    let separators = local_separators(buf, separators);
    let mut pos = from.min(buf.text.len());
    for _ in 0..count {
        if pos == 0 {
            break;
        }
        while pos > 0
            && !buf
                .text
                .at(pos - 1)
                .is_some_and(|c| constituent(c, separators))
        {
            pos -= 1;
        }
        if pos > 0 {
            pos -= 1;
        }
        while pos > 0
            && buf
                .text
                .at(pos - 1)
                .is_some_and(|c| constituent(c, separators))
            && !boundary(&buf.text, pos)
        {
            pos -= 1;
        }
    }
    pos
}
