//! What a window's rows say, and how a column of the buffer maps to a column
//! of the screen and back, with virtual text drawn between them.
//!
//! Design: `doc/architettura-editor.typ, <righe>`.
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

/// One row of a window, composed: the text it shows and the pieces it is
/// made of, buffer text or virtual text.
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
    // TODO(TUI-3): a Tab is one column here.
    // TODO(TUI-4): so is a wide character.
    /// Returns a row that is exactly its line, with nothing added.
    fn plain(line: usize, text: String) -> Self {
        let width = text.chars().count();
        Self {
            line,
            text,
            pieces: vec![Piece::Text { from: 0, width }],
            faces: Vec::new(),
        }
    }

    /// Returns the screen column buffer column COLUMN is drawn at.
    ///
    /// Virtual text anchored at a column is drawn before it, so the column
    /// maps past the virtual text, onto its own character: point is never
    /// drawn inside virtual text. A column past the end of the line maps past
    /// the end of the row, one screen column per buffer column.
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
        // Past the end of the line, one screen column per buffer column, and
        // not clamped: a selection runs one column past a line's end to show
        // the newline as selected.
        screen + (column - text_width.min(column))
    }

    /// Returns the buffer column screen column SCREEN is on.
    ///
    /// A column inside virtual text answers the column the text is anchored
    /// at: a click on virtual text puts point on the character it is in
    /// front of.
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

    /// Returns the faces the virtual text on this row asked for, as
    /// highlights at ROW.
    fn virtual_highlights(&self, row: usize) -> impl Iterator<Item = Highlight> + '_ {
        self.faces.iter().map(move |(start, end, face)| Highlight {
            row,
            start_col: *start,
            end_col: *end,
            face: *face,
        })
    }
}

/// The rows a window is showing, composed from its buffer's lines.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    rows: Vec<Row>,
    /// The first buffer line the rows cover, so a line can be found without
    /// searching.
    first_line: usize,
}

impl Layout {
    /// Composes the rows of the buffer lines from FIRST_LINE, HEIGHT of them.
    ///
    /// VIRTUAL_TEXT is asked once per row, not once per character.
    pub fn compose<B: BufferTrait>(
        text: &B,
        virtual_text: &VirtualTextTable,
        first_line: usize,
        height: usize,
    ) -> Self {
        let lines = text.get_lines(first_line, first_line + height);
        // The ordinary case: with nothing to add, every row is its line.
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

    /// Returns the row showing buffer line LINE, with its index, if it is on
    /// screen.
    pub fn row_of_line(&self, line: usize) -> Option<(usize, &Row)> {
        let index = line.checked_sub(self.first_line)?;
        self.rows.get(index).map(|row| (index, row))
    }

    /// Returns the row at index ROW of the window.
    pub fn row(&self, row: usize) -> Option<&Row> {
        self.rows.get(row)
    }

    /// Returns every row's text, scrolled sideways by SCROLL_X and cut to
    /// WIDTH: what the renderer draws.
    pub fn visible_text(&self, scroll_x: usize, width: usize) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| row.text.chars().skip(scroll_x).take(width).collect())
            .collect()
    }

    /// Returns the highlights the virtual text asks for, clipped like any
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

/// Returns the row of LINE, whose TEXT starts at offset LINE_START, with the
/// virtual text that sits inside it.
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

/// A highlight in buffer coordinates, before it is placed on a row: a type
/// apart from [`Highlight`], the same four numbers in screen coordinates, so
/// that the compiler tells the two apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BufferHighlight {
    pub line: usize,
    /// Half-open buffer columns.
    pub start: usize,
    pub end: usize,
    pub face: Face,
}

/// Returns HIGHLIGHTS placed on the rows of LAYOUT, scrolled by SCROLL_X and
/// cut to WIDTH, without those left empty. The one place colouring, overlays
/// and the region are mapped onto the screen.
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

/// Returns HIGHLIGHT shifted by the sideways scroll and cut to WIDTH, or
/// `None` when nothing of it is left.
fn clip(highlight: Highlight, scroll_x: usize, width: usize) -> Option<Highlight> {
    let start_col = highlight.start_col.saturating_sub(scroll_x);
    let end_col = highlight.end_col.saturating_sub(scroll_x).min(width);
    (start_col < end_col).then_some(Highlight {
        start_col,
        end_col,
        ..highlight
    })
}
