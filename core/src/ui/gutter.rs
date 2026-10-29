//! The gutter: the line numbers left of a window's text, how many columns
//! they take, and what each row of them says.
//!
//! Design: `doc/architettura-editor.typ, <margine>`.
use crate::ui::Face;

/// One row of a window's gutter.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct GutterCell {
    /// Exactly [`RenderableWindowView::gutter_width`] characters, padded: a
    /// renderer prints it without measuring.
    ///
    /// [`RenderableWindowView::gutter_width`]:
    /// crate::ui::RenderableWindowView::gutter_width
    pub text: String,
    pub face: Face,
}

/// What a window's gutter shows, as `display-line-numbers` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineNumbers {
    /// No gutter at all: the text starts at the window's left edge.
    #[default]
    Off,
    /// Each line's own number, counted from one.
    Absolute,
    /// How far each line is from point's, with point's own line showing its
    /// own number rather than a nought.
    Relative,
}

/// How the gutters are configured, read from Lisp before composition takes
/// any lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GutterSpec {
    pub numbers: LineNumbers,
    /// The fewest columns the numbers take, without the space after them: a
    /// minimum, so that the text does not shift as a file grows; more when a
    /// number needs them.
    pub min_digits: usize,
}

/// The narrowest a window's text may be left by a gutter: a window narrower
/// than this and the gutter draws no numbers.
pub const MIN_TEXT_WIDTH_FOR_GUTTER: usize = 8;

/// Returns how many digits N has in base ten.
fn decimal_width(n: usize) -> usize {
    let mut width = 1;
    let mut rest = n / 10;
    while rest > 0 {
        width += 1;
        rest /= 10;
    }
    width
}

/// Returns how many columns the gutter takes in a window WIDTH columns wide
/// showing LINE_COUNT lines, as SPEC says: zero when SPEC asks for none, or
/// when the window cannot spare them.
pub fn gutter_columns(spec: GutterSpec, line_count: usize, width: usize) -> usize {
    if spec.numbers == LineNumbers::Off {
        return 0;
    }
    // An empty buffer still has a line one.
    let digits = decimal_width(line_count.max(1)).max(spec.min_digits);
    // One more for the space between the numbers and the text.
    let wanted = digits + 1;
    if width < wanted + MIN_TEXT_WIDTH_FOR_GUTTER {
        return 0;
    }
    wanted
}

/// Returns the gutter, WIDTH columns wide, of ROWS rows of a window showing
/// LINE_COUNT lines from FIRST_LINE, with point on POINT_LINE: a cell for
/// every row, blank past the end of the buffer, so that cells and rows pair
/// by index.
pub fn gutter_cells(
    spec: GutterSpec,
    width: usize,
    first_line: usize,
    rows: usize,
    line_count: usize,
    point_line: usize,
) -> Vec<GutterCell> {
    if width == 0 {
        return Vec::new();
    }
    (0..rows)
        .map(|row| {
            let line = first_line + row;
            if line >= line_count {
                return GutterCell {
                    text: " ".repeat(width),
                    face: Face::LINE_NUMBER,
                };
            }
            let current = line == point_line;
            let shown = match spec.numbers {
                LineNumbers::Relative if !current => line.abs_diff(point_line),
                // Counted from one, as a person reads them.
                _ => line + 1,
            };
            // Right-aligned, with the last column for the space; a number too
            // wide loses its left end rather than shift the row.
            let mut text = format!("{:>pad$} ", shown, pad = width - 1);
            if text.chars().count() > width {
                text = text.chars().skip(text.chars().count() - width).collect();
            }
            GutterCell {
                text,
                face: if current {
                    Face::LINE_NUMBER_CURRENT
                } else {
                    Face::LINE_NUMBER
                },
            }
        })
        .collect()
}
