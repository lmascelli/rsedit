//! The block of text two corners describe, and where it sits in a buffer.
//!
//! # What a rectangle is here
//!
//! Point and the mark are two corners; the rectangle is the block between
//! them. Not the *region* between them -- the region from the middle of one
//! line to the middle of a later one is a ragged run that takes in whole
//! lines on the way, and a rectangle is the columns they have in common.
//!
//! That is the whole idea, and everything below is the arithmetic of saying
//! which characters those are when the lines are not all long enough.
//!
//! # Columns are characters
//!
//! Nothing in this editor expands a tab: a tab is drawn in one cell, the way
//! any other character is. So a character column and a screen column are the
//! same number everywhere, and a rectangle cut by character column is the
//! block that was drawn. An editor that expanded tabs would have to choose
//! between the two here, and this one does not have the choice to make.
//!
//! # Short lines
//!
//! A rectangle is a pair of columns applied to a run of lines, and nothing
//! says every line reaches them. A line that stops before the left edge is
//! still one of the rectangle's lines -- leaving it out would change the
//! block's height depending on its contents -- so it takes part with an empty
//! span, and [`Span::padding`] says how many spaces would have to be added to
//! reach the left edge.
//!
//! What to do about that is the operation's business, and the two answers are
//! both right for their own operation. Taking text out -- killing, copying,
//! clearing -- takes what is there and leaves the line's length alone. Putting
//! text in -- opening, yanking, `string-rectangle` -- pads first, because a
//! column that does not exist yet is exactly what those were asked to fill.
use crate::buffer::BufferTrait;

/// A block of text, as a pair of lines and a pair of columns.
///
/// `bottom` is inclusive and `right` is not, which is not an oversight: the
/// lines are a run of *things* and the columns are a span of *positions*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rectangle {
    pub top: usize,
    pub bottom: usize,
    pub left: usize,
    pub right: usize,
}

impl Rectangle {
    /// The block between two corners, given as (line, column) pairs.
    ///
    /// Either corner may be either end: a rectangle dragged up and to the
    /// left is the same rectangle as one dragged down and to the right, and
    /// no caller should have to know which way round the user made it.
    pub fn between(a: (usize, usize), b: (usize, usize)) -> Self {
        Self {
            top: a.0.min(b.0),
            bottom: a.0.max(b.0),
            left: a.1.min(b.1),
            right: a.1.max(b.1),
        }
    }

    /// How many columns wide. Zero is allowed and is useful: a rectangle with
    /// no width is a position on every line, which is what `string-rectangle`
    /// wants when it is being used to prefix a run of lines.
    pub fn width(&self) -> usize {
        self.right - self.left
    }

    /// How many lines tall. Never zero -- a rectangle within one line is one
    /// line tall.
    pub fn height(&self) -> usize {
        self.bottom - self.top + 1
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
    /// Whether this line has any of the rectangle's text on it.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Where RECT sits in TEXT, one entry per line, top first.
///
/// Top first because that is the order the block reads in. Anything that
/// *edits* has to walk them backwards -- an edit to one line moves every
/// offset after it -- which is the caller's business and is why this returns
/// a list rather than doing the walk itself.
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

/// The text RECT covers, one string per line, top first.
///
/// A line that does not reach the rectangle contributes an empty string
/// rather than being left out, so the block keeps the shape it was cut with
/// and can be put back with its lines still lined up.
pub fn text_of<B: BufferTrait>(text: &B, rect: &Rectangle) -> Vec<String> {
    spans(text, rect)
        .into_iter()
        .map(|span| {
            (span.start..span.end)
                .filter_map(|at| text.at(at))
                .collect()
        })
        .collect()
}
