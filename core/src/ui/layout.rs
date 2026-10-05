//! What a window's rows actually say, and how a screen cell relates to a
//! buffer position.
//!
//! # The identity this replaces
//!
//! Everything in the editor used to rest on one equality:
//!
//! > buffer column == screen column minus `scroll_x`
//!
//! It held because a row on screen was a line of the buffer and nothing else.
//! Five things depended on it: the strings a window shows, where the cursor is
//! drawn, how far the window scrolls sideways to keep the cursor visible,
//! where a highlight's columns are, and which buffer position a click is on.
//!
//! [`crate::buffer::virtual_text`] breaks it. A row that shows an inlay hint
//! is longer than the line it came from, and every one of those five has to
//! ask rather than assume. This module is the thing they ask.
//!
//! # Rows, not lines
//!
//! A [`Row`] is one row of the screen, and it records which buffer line it
//! came from. Today that is always one row per line -- nothing produces a row
//! that is not a line -- but the five callers are written against rows, so a
//! decoration that occupies a row of its own, or a long line wrapped across
//! two, is a new kind of producer rather than a fifth coordinate system
//! threaded through the editor again.
//!
//! What is deliberately *not* here is an enum of row kinds with variants
//! nothing creates. [`Row::line`] is a buffer line number because that is
//! what every caller needs today; when something does produce a row belonging
//! to no line, it becomes the thing that says so, and the change is to one
//! type and the handful of places that read it.
//!
//! # Why a list of pieces
//!
//! A row is a few runs: some of the buffer, some not. Mapping either way is a
//! walk over them, and a row with no virtual text on it has exactly one piece
//! -- which is every row of every window in the ordinary case, so the common
//! path is a single comparison and an addition.
//!
//! The alternative, a buffer column per screen cell, is simpler to read and
//! allocates per cell per row per window per frame for a mapping that is
//! almost always the identity.
use crate::buffer::{BufferTrait, VirtualTextTable};
use crate::ui::{Face, Highlight};

/// A run of screen columns, and where they came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Piece {
    /// `width` columns of the buffer, starting at buffer column `from`.
    Text { from: usize, width: usize },
    /// `width` columns that are in no buffer.
    Virtual { width: usize },
}

impl Piece {
    fn width(&self) -> usize {
        match self {
            Piece::Text { width, .. } | Piece::Virtual { width } => *width,
        }
    }
}

/// One row of a window, composed.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Which line of the buffer this row shows.
    pub line: usize,
    /// Everything on the row, in screen order, before any sideways scrolling.
    pub text: String,
    pieces: Vec<Piece>,
    /// Faces the virtual text asked for, as screen column spans on this row.
    /// Empty for a row with no virtual text, which is the ordinary case.
    faces: Vec<(usize, usize, Face)>,
}

impl Row {
    /// A row that is exactly its line, with nothing added.
    fn plain(line: usize, text: String) -> Self {
        let width = text.chars().count();
        Self {
            line,
            text,
            pieces: vec![Piece::Text { from: 0, width }],
            faces: Vec::new(),
        }
    }

    /// How many screen columns the row occupies.
    pub fn width(&self) -> usize {
        self.pieces.iter().map(Piece::width).sum()
    }

    /// Where buffer column COLUMN is drawn.
    ///
    /// Virtual text anchored at a column comes *before* it, so a position with
    /// a hint in front of it maps past the hint and onto the character itself.
    /// That is what keeps the cursor on real text: a hint is not somewhere
    /// point can be, and point is never drawn inside one.
    ///
    /// A column past the end of the line maps past the end of the row, which
    /// is what the cursor sitting on a line's newline needs.
    pub fn to_screen(&self, column: usize) -> usize {
        let mut screen = 0;
        let mut text_width = 0;
        for piece in &self.pieces {
            match piece {
                Piece::Text { from, width } => {
                    if column < from + width {
                        return screen + (column - from);
                    }
                    screen += width;
                    text_width = from + width;
                }
                Piece::Virtual { width } => screen += width,
            }
        }
        // Past the end of the line, one screen column per buffer column.
        //
        // Not clamped to the row, which is what this did first and what two
        // tests caught: a span that runs one column past a line's last
        // character is how a multi-line selection shows its line endings as
        // selected, and clamping turned every one of those back into a span
        // that stopped at the text.
        screen + (column - text_width.min(column))
    }

    /// Which buffer column screen column SCREEN is on.
    ///
    /// A column inside virtual text answers the buffer column the text is
    /// anchored at -- clicking a hint puts point on the character it is in
    /// front of, because there is nowhere else for point to go.
    pub fn to_buffer(&self, screen: usize) -> usize {
        let mut walked = 0;
        let mut last_buffer = 0;
        for piece in &self.pieces {
            match piece {
                Piece::Text { from, width } => {
                    if screen < walked + width {
                        return from + (screen - walked);
                    }
                    walked += width;
                    last_buffer = from + width;
                }
                Piece::Virtual { width } => {
                    if screen < walked + width {
                        // The anchor: the column the next run of text starts
                        // at, which is the one this virtual text sits before.
                        return last_buffer;
                    }
                    walked += width;
                }
            }
        }
        // Past the end of the row, one buffer column per screen column, so
        // that this stays the inverse of `to_screen` outside the text as well
        // as inside it.
        last_buffer + (screen - walked.min(screen))
    }

    /// The faces the virtual text on this row asked for, as highlights at ROW.
    fn virtual_highlights(&self, row: usize) -> impl Iterator<Item = Highlight> + '_ {
        self.faces.iter().map(move |(start, end, face)| Highlight {
            row,
            start_col: *start,
            end_col: *end,
            face: *face,
        })
    }
}

/// The rows a window is showing.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    rows: Vec<Row>,
    /// The first buffer line the rows cover, so a line can be found without
    /// searching.
    first_line: usize,
}

impl Layout {
    /// Compose the rows for the buffer lines `[first_line, first_line + height)`.
    ///
    /// `virtual_text` is read once per row rather than searched per character:
    /// a frame does this for every visible row of every window, and a search
    /// per column would be the kind of cost that only shows up on somebody
    /// else's machine.
    pub fn compose<B: BufferTrait>(
        text: &B,
        virtual_text: &VirtualTextTable,
        first_line: usize,
        height: usize,
    ) -> Self {
        let lines = text.get_lines(first_line, first_line + height);
        // The ordinary case, and worth not paying for: with nothing to add,
        // every row is its line.
        if virtual_text.is_empty() {
            return Self {
                rows: lines
                    .into_iter()
                    .enumerate()
                    .map(|(offset, line)| Row::plain(first_line + offset, line))
                    .collect(),
                first_line,
            };
        }
        let rows = lines
            .into_iter()
            .enumerate()
            .map(|(offset, line)| {
                let line_number = first_line + offset;
                let line_start = text.cursor_2d_to_1d(line_number, 0);
                compose_row(line_number, &line, line_start, virtual_text)
            })
            .collect();
        Self { rows, first_line }
    }

    /// The row showing buffer line LINE, if it is on screen.
    pub fn row_of_line(&self, line: usize) -> Option<(usize, &Row)> {
        let index = line.checked_sub(self.first_line)?;
        self.rows.get(index).map(|row| (index, row))
    }

    /// The row at index ROW of the window.
    pub fn row(&self, row: usize) -> Option<&Row> {
        self.rows.get(row)
    }

    /// Every row's text, scrolled sideways by SCROLL_X and cut to WIDTH --
    /// what the renderer draws.
    pub fn visible_text(&self, scroll_x: usize, width: usize) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| row.text.chars().skip(scroll_x).take(width).collect())
            .collect()
    }

    /// The highlights the virtual text itself asks for, clipped like any
    /// other.
    pub fn virtual_highlights(&self, scroll_x: usize, width: usize) -> Vec<Highlight> {
        self.rows
            .iter()
            .enumerate()
            .flat_map(|(index, row)| row.virtual_highlights(index))
            .filter_map(|highlight| clip(highlight, scroll_x, width))
            .collect()
    }
}

/// Compose one row out of its line and whatever sits inside it.
fn compose_row(line: usize, text: &str, line_start: usize, virtual_text: &VirtualTextTable) -> Row {
    let line_width = text.chars().count();
    // The newline's own offset is in range, so virtual text at the end of a
    // line is placed by anchoring it there.
    let inside: Vec<_> = virtual_text
        .between(line_start, line_start + line_width + 1)
        .collect();
    if inside.is_empty() {
        return Row::plain(line, text.to_string());
    }
    let mut composed = String::new();
    let mut pieces = Vec::new();
    let mut faces = Vec::new();
    let mut column = 0;
    let mut screen = 0;
    for entry in inside {
        let at = (entry.at - line_start).min(line_width);
        if at > column {
            let run: String = text.chars().skip(column).take(at - column).collect();
            composed.push_str(&run);
            pieces.push(Piece::Text {
                from: column,
                width: at - column,
            });
            screen += at - column;
            column = at;
        }
        let width = entry.text.chars().count();
        if width == 0 {
            continue;
        }
        composed.push_str(&entry.text);
        pieces.push(Piece::Virtual { width });
        faces.push((screen, screen + width, entry.face));
        screen += width;
    }
    if column < line_width {
        let run: String = text.chars().skip(column).collect();
        composed.push_str(&run);
        pieces.push(Piece::Text {
            from: column,
            width: line_width - column,
        });
    }
    Row {
        line,
        text: composed,
        pieces,
        faces,
    }
}

/// A highlight in buffer coordinates, before it is placed on a row.
///
/// A distinct type from [`Highlight`] so that the compiler catches the two
/// being mixed up: they are the same four numbers in two coordinate systems,
/// and getting them the wrong way round draws a colour on the wrong words
/// rather than failing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BufferHighlight {
    pub line: usize,
    /// Half-open buffer columns.
    pub start: usize,
    pub end: usize,
    pub face: Face,
}

/// Place HIGHLIGHTS on the rows they belong to, scrolled and clipped.
///
/// One function for all three producers -- syntax, overlays and the region.
/// Each of them used to subtract `scroll_x`, clamp to the width and drop the
/// empties itself, which was three copies of a clipping rule and three places
/// for a mapping to be forgotten.
pub fn place(
    layout: &Layout,
    highlights: impl IntoIterator<Item = BufferHighlight>,
    scroll_x: usize,
    width: usize,
) -> Vec<Highlight> {
    highlights
        .into_iter()
        .filter_map(|highlight| {
            let (row, line) = layout.row_of_line(highlight.line)?;
            clip(
                Highlight {
                    row,
                    start_col: line.to_screen(highlight.start),
                    end_col: line.to_screen(highlight.end),
                    face: highlight.face,
                },
                scroll_x,
                width,
            )
        })
        .collect()
}

/// Shift a highlight by the sideways scroll and cut it to the window, or drop
/// it when nothing of it is left.
fn clip(highlight: Highlight, scroll_x: usize, width: usize) -> Option<Highlight> {
    let start_col = highlight.start_col.saturating_sub(scroll_x);
    let end_col = highlight.end_col.saturating_sub(scroll_x).min(width);
    (start_col < end_col).then_some(Highlight {
        start_col,
        end_col,
        ..highlight
    })
}
