//! Rectangles: the block of columns two corners have in common, and where it
//! sits in a text, line by line. A column is a character, and a tab is one
//! column, as everywhere in the editor. A line too short to reach the block
//! takes part with an empty span and the padding it lacks.
//!
//! Design: `doc/architettura-editor.typ, <rettangoli>`.
use crate::buffer::BufferTrait;

/// A block of text, as a pair of lines and a pair of columns.
///
/// `bottom` is inclusive and `right` is not: the lines are a run of *things*,
/// and the columns a span of *positions*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rectangle {
    pub top: usize,
    pub bottom: usize,
    pub left: usize,
    pub right: usize,
}

impl Rectangle {
    /// Returns the block between two corners, given as (line, column) pairs,
    /// either of which may be either end.
    pub fn between(a: (usize, usize), b: (usize, usize)) -> Self {
        Self {
            top: a.0.min(b.0),
            bottom: a.0.max(b.0),
            left: a.1.min(b.1),
            right: a.1.max(b.1),
        }
    }

    /// Returns the block between offsets A and B of TEXT, either of which may
    /// be either end. With INCLUSIVE, the column at the right edge is part of
    /// it, as a selection that takes the character at both ends wants.
    pub fn between_offsets<B: BufferTrait>(text: &B, a: usize, b: usize, inclusive: bool) -> Self {
        let mut rectangle = Self::between(
            text.cursor_1d_to_2d(a.min(text.len())),
            text.cursor_1d_to_2d(b.min(text.len())),
        );
        rectangle.right = rectangle.right.saturating_add(usize::from(inclusive));
        rectangle
    }

    /// Returns how many columns wide it is. Zero is allowed: a rectangle with
    /// no width is a position on every line, which `string-rectangle` uses to
    /// prefix a run of lines.
    pub fn width(&self) -> usize {
        self.right - self.left
    }
}

/// Where one line's part of a rectangle is, in buffer offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    /// The left edge on this line, clamped to the line's end.
    pub start: usize,
    /// The right edge, clamped likewise. Equal to `start` on a line that
    /// stops before the rectangle begins.
    pub end: usize,
    /// How many spaces this line is short of the left edge. Zero on a line
    /// that reaches it.
    pub padding: usize,
}

impl Span {
    /// Returns whether this line has any of the rectangle's text on it.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Returns where RECT sits in TEXT, one span per line, top first: the order
/// the block reads in. Whatever edits walks them from the bottom, since an edit
/// to one line moves every offset after it.
pub fn spans<B: BufferTrait>(text: &B, rect: &Rectangle) -> Vec<Span> {
    let last_line = text.line_count().saturating_sub(1);
    (rect.top..=rect.bottom.min(last_line))
        .map(|line| {
            let line_start = text.cursor_2d_to_1d(line, 0);
            // `cursor_2d_to_1d` clamps a column past the end of its line to
            // the line's end, which is the whole of what makes a short line
            // work: both edges land on the end and the span is empty.
            let start = text.cursor_2d_to_1d(line, rect.left);
            let end = text.cursor_2d_to_1d(line, rect.right);
            let reached = start - line_start;
            Span {
                line,
                start,
                end,
                padding: rect.left - reached.min(rect.left),
            }
        })
        .collect()
}

/// Returns the text RECT covers, one string per line, top first. A line that
/// does not reach the rectangle gives an empty string, so that the block keeps
/// its shape.
pub fn text_of<B: BufferTrait>(text: &B, rect: &Rectangle) -> Vec<String> {
    spans(text, rect)
        .into_iter()
        .map(|span| text.slice(span.start, span.end))
        .collect()
}
