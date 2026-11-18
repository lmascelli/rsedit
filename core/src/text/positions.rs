//! A position as a line and a column, with the column counted in characters,
//! in UTF-16 code units or in UTF-8 bytes, and back: for a program that
//! measures text in one of those encodings and has to agree with a text
//! counted in characters. The same measure on a string says where its end
//! falls when the string is put in a text.
//!
//! Design: `doc/architettura-editor.typ, <posizioni-unita>`.
use crate::buffer::BufferTrait;

/// What a column counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Characters, as a position does.
    Char,
    /// UTF-16 code units: two for a character outside the Basic Multilingual
    /// Plane, one for any other.
    Utf16,
    /// UTF-8 bytes: from one to four for a character.
    Utf8,
}

impl Unit {
    /// Returns how many of this unit C takes.
    pub fn width(self, c: char) -> usize {
        match self {
            Unit::Char => 1,
            Unit::Utf16 => c.len_utf16(),
            Unit::Utf8 => c.len_utf8(),
        }
    }

    /// Returns how many of this unit CHARS take together.
    fn measure(self, chars: impl Iterator<Item = char>) -> usize {
        match self {
            Unit::Char => chars.count(),
            unit => chars.map(|c| unit.width(c)).sum(),
        }
    }

    /// Returns the name Lisp gives the unit.
    pub fn name(self) -> &'static str {
        match self {
            Unit::Char => "char",
            Unit::Utf16 => "utf-16",
            Unit::Utf8 => "utf-8",
        }
    }
}

/// Returns the line of POS in TEXT, counted from 0, and its column, counted
/// in UNIT from the start of the line. A position past the end is the end.
///
/// Costs the length of the line before POS, not the text: the line comes
/// from the text's own index.
pub fn line_column<B: BufferTrait>(text: &B, pos: usize, unit: Unit) -> (usize, usize) {
    let pos = pos.min(text.len());
    let (line, column) = text.cursor_1d_to_2d(pos);
    if unit == Unit::Char {
        return (line, column);
    }
    (
        line,
        unit.measure(text.chars_from(pos - column).take(column)),
    )
}

/// Returns how many newlines TEXT holds, and the column of its end, counted
/// in UNIT from its last newline or, when it has none, from its start.
///
/// It is the line and column [`line_column`] gives for the end of a text that
/// is TEXT alone, and costs one pass over TEXT.
pub fn string_line_column(text: &str, unit: Unit) -> (usize, usize) {
    let lines = text.bytes().filter(|&b| b == b'\n').count();
    let last_line = text.rfind('\n').map_or(text, |at| &text[at + 1..]);
    (lines, unit.measure(last_line.chars()))
}

/// Returns the position in TEXT at LINE, counted from 0, and COLUMN, counted
/// in UNIT from the start of the line.
///
/// A column past the end of its line is the line's end, and a line past the
/// last is the end of the text. A column that falls inside a character --
/// between the two halves of a surrogate pair, among the bytes of one
/// character -- is the position before that character.
pub fn position<B: BufferTrait>(text: &B, line: usize, column: usize, unit: Unit) -> usize {
    if line >= text.line_count() {
        return text.len();
    }
    if unit == Unit::Char {
        return text.cursor_2d_to_1d(line, column);
    }
    let start = text.cursor_2d_to_1d(line, 0);
    let mut at = start;
    let mut counted = 0;
    for c in text.chars_from(start) {
        let next = counted + unit.width(c);
        if c == '\n' || next > column {
            break;
        }
        counted = next;
        at += 1;
    }
    at
}
