//! What a window's rows say, and how a column of the buffer maps to a cell of
//! the screen and back: a Tab drawn to the next stop, a wide character across
//! two cells, and virtual text between them.
//!
//! Design: `doc/architettura-editor.typ, <righe>`.
use crate::buffer::{BufferTrait, VirtualTextTable};
use crate::ui::width::{char_cells, cut};
use crate::ui::{Face, Highlight};
use std::collections::HashMap;
use std::sync::Arc;

/// How many cells a Tab reaches to, mode by mode: a mode's own `tab-width`
/// property, or the variable, read once a frame before any lock is taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabWidths {
    /// What a mode with none of its own has.
    pub default: usize,
    /// The modes that have their own.
    pub by_mode: HashMap<String, usize>,
}

impl Default for TabWidths {
    fn default() -> Self {
        Self {
            default: DEFAULT_TAB_WIDTH,
            by_mode: HashMap::new(),
        }
    }
}

impl TabWidths {
    /// Returns the width of a Tab in MODE.
    pub fn of(&self, mode: &str) -> usize {
        self.by_mode.get(mode).copied().unwrap_or(self.default)
    }
}

/// How many cells a Tab reaches to when nothing says otherwise: what the
/// `indent` module gives `tab-width`.
pub const DEFAULT_TAB_WIDTH: usize = 4;

/// The widest a Tab is drawn, however wide it is set: wider has no use, and
/// a mistaken million would make one Tab a million spaces a frame.
pub const MAX_TAB_WIDTH: usize = 64;

/// One row of a window, composed: the text it shows, with Tabs drawn as the
/// spaces they reach across, and where each column of the line is on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Which line of the buffer this row shows.
    pub line: usize,
    /// Everything on the row, in screen order, before any sideways scrolling.
    pub text: String,
    /// Where each column of the line is drawn: the cell it begins at and how
    /// many it takes. `None` when every column is the one cell of the same
    /// number, which is most rows, and costs nothing to keep.
    columns: Option<Arc<Vec<(usize, usize)>>>,
    /// How many columns the line has.
    length: usize,
    /// How many cells the row takes, virtual text included.
    cells: usize,
    /// Faces the virtual text asked for, as cell spans on this row. Empty for
    /// a row with no virtual text, which is the ordinary case.
    faces: Vec<(usize, usize, Face)>,
    /// The cells of the composed logical line this visual row shows.
    from: usize,
    to: usize,
    /// Its index within the logical line, and whether it is the final row.
    pub continuation: usize,
    last: bool,
    wrapped: bool,
}

impl Row {
    /// Returns a row that is exactly its line, a column to a cell.
    fn plain(line: usize, text: String) -> Self {
        let length = text.chars().count();
        Self {
            line,
            text,
            columns: None,
            length,
            cells: length,
            faces: Vec::new(),
            from: 0,
            to: length,
            continuation: 0,
            last: true,
            wrapped: false,
        }
    }

    /// Returns the cell buffer column COLUMN is drawn at: the first of a Tab's
    /// or a wide character's.
    ///
    /// Virtual text anchored at a column is drawn before it, so the column
    /// maps past the virtual text, onto its own character: point is never
    /// drawn inside virtual text. A column past the end of the line maps past
    /// the end of the row, one cell per column.
    pub fn to_screen(&self, column: usize) -> usize {
        self.line_cell(column).saturating_sub(self.from)
    }

    fn line_cell(&self, column: usize) -> usize {
        let Some(columns) = &self.columns else {
            return column;
        };
        match columns.get(column) {
            Some((start, _)) => *start,
            // Past the end of the line, one cell per column, and not clamped:
            // a selection runs one column past a line's end to show the
            // newline as selected.
            None => self.cells + (column - self.length),
        }
    }

    /// Returns how many cells buffer column COLUMN takes: one past the end of
    /// the line, where point can be.
    pub fn cells_at(&self, column: usize) -> usize {
        self.columns
            .as_ref()
            .and_then(|columns| columns.get(column))
            .map_or(1, |(_, cells)| (*cells).max(1))
    }

    /// Returns the buffer column cell SCREEN is on.
    ///
    /// Every cell of a Tab or a wide character answers its column. A cell
    /// inside virtual text answers the column the text is anchored at: a click
    /// on virtual text puts point on the character it is in front of.
    pub fn to_buffer(&self, screen: usize) -> usize {
        let column = self.buffer_at(self.from.saturating_add(screen));
        if !self.wrapped {
            column
        } else if self.last {
            column.min(self.length)
        } else {
            column.min(self.last_column())
        }
    }

    fn buffer_at(&self, screen: usize) -> usize {
        let Some(columns) = &self.columns else {
            return screen;
        };
        // The first column that ends past SCREEN: the one SCREEN is on, or,
        // when SCREEN is in virtual text, the one the text is in front of.
        let at = columns.partition_point(|(start, cells)| start + cells <= screen);
        if at < columns.len() {
            return at;
        }
        if screen < self.cells {
            // Virtual text at the end of the line, anchored there.
            return self.length;
        }
        // Past the end of the row, one column per cell, so that this stays
        // the inverse of `to_screen` outside the text as well as inside it.
        self.length + (screen - self.cells)
    }

    /// Returns the first and last positions point can occupy on this row.
    pub fn first_column(&self) -> usize {
        self.buffer_at(self.from).min(self.length)
    }

    pub fn last_column(&self) -> usize {
        if self.last {
            self.length
        } else {
            self.buffer_at(self.to.saturating_sub(1)).min(self.length)
        }
    }

    /// Returns whether COLUMN is displayed on this row, rather than on one
    /// of the other rows of the same logical line.
    pub fn contains(&self, column: usize) -> bool {
        let cell = self.line_cell(column);
        cell >= self.from && (self.last || cell < self.to)
    }

    /// Maps a cell on this row back to the unwrapped composed line. Mouse
    /// handling uses this before doing its existing cell-to-buffer mapping.
    pub fn line_screen(&self, screen: usize) -> usize {
        self.line_cell(self.to_buffer(screen))
    }

    /// Splits a composed line at whitespace when possible. The column map is
    /// shared by its continuations, so a long line does not copy it per row.
    pub(crate) fn wrap(self, width: usize) -> Vec<Self> {
        let width = width.max(1);
        let chars: Vec<_> = self.text.char_indices().collect();
        let mut ranges = Vec::new();
        let (mut at, mut cell) = (0, 0);
        while at < chars.len() {
            let (start, from) = (at, cell);
            let mut word_break = None;
            while at < chars.len() {
                let c = chars[at].1;
                let size = char_cells(c);
                if cell + size > from + width && cell > from {
                    break;
                }
                cell += size;
                at += 1;
                if c.is_whitespace() {
                    word_break = Some((at, cell));
                }
            }
            if at < chars.len()
                && !chars[at].1.is_whitespace()
                && let Some((next, end)) = word_break
            {
                at = next;
                cell = end;
            }
            let byte_start = chars[start].0;
            let byte_end = chars.get(at).map_or(self.text.len(), |(byte, _)| *byte);
            ranges.push((byte_start, byte_end, from, cell));
        }
        // Point at an exact-width end of line needs a cell of its own.
        if ranges.is_empty() || ranges.last().is_some_and(|r| r.3 - r.2 >= width) {
            ranges.push((self.text.len(), self.text.len(), self.cells, self.cells));
        }
        let last = ranges.len() - 1;
        ranges
            .into_iter()
            .enumerate()
            .map(|(index, (a, b, from, to))| Self {
                text: self.text[a..b].to_string(),
                columns: self.columns.clone(),
                faces: self
                    .faces
                    .iter()
                    .filter_map(|(start, end, face)| {
                        let a = (*start).max(from);
                        let b = (*end).min(to);
                        (a < b).then(|| (a - from, b - from, *face))
                    })
                    .collect(),
                from,
                to,
                continuation: index,
                last: index == last,
                wrapped: true,
                line: self.line,
                length: self.length,
                cells: self.cells,
            })
            .collect()
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    rows: Vec<Row>,
}

impl Layout {
    /// Composes the rows of the buffer lines from FIRST_LINE, HEIGHT of them,
    /// with a Tab reaching to the next multiple of TAB_WIDTH.
    ///
    /// VIRTUAL_TEXT is asked once per row, not once per character.
    pub fn compose<B: BufferTrait>(
        text: &B,
        virtual_text: &VirtualTextTable,
        first_line: usize,
        height: usize,
        tab_width: usize,
    ) -> Self {
        let tab_width = tab_width.clamp(1, MAX_TAB_WIDTH);
        let lines = text.get_lines(first_line, first_line + height);
        let rows = lines
            .into_iter()
            .enumerate()
            .map(|(offset, line)| {
                let line_number = first_line + offset;
                // Asked for its offset only when there is virtual text to
                // place: the ordinary case reads nothing more of the buffer.
                let line_start = if virtual_text.is_empty() {
                    0
                } else {
                    text.cursor_2d_to_1d(line_number, 0)
                };
                compose_row(line_number, &line, line_start, virtual_text, tab_width)
            })
            .collect();
        Self { rows }
    }

    /// Returns the row showing buffer line LINE, with its index, if it is on
    /// screen.
    pub fn row_of_line(&self, line: usize) -> Option<(usize, &Row)> {
        let index = self.rows.partition_point(|row| row.line < line);
        self.rows
            .get(index)
            .filter(|row| row.line == line)
            .map(|row| (index, row))
    }

    /// Returns the visual row that displays a particular buffer position.
    pub fn row_of_position(&self, line: usize, column: usize) -> Option<(usize, &Row)> {
        self.rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.line == line && row.contains(column))
    }

    pub(crate) fn from_rows(rows: Vec<Row>) -> Self {
        Self { rows }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn is_wrapped(&self) -> bool {
        self.rows.first().is_some_and(|row| row.wrapped)
    }

    /// Returns the row at index ROW of the window.
    pub fn row(&self, row: usize) -> Option<&Row> {
        self.rows.get(row)
    }

    /// Returns every row's text, scrolled sideways by SCROLL_X cells and cut
    /// to WIDTH: what the renderer draws.
    pub fn visible_text(&self, scroll_x: usize, width: usize) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| cut(&row.text, scroll_x, width))
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

/// Returns whether every character of TEXT is drawn as one cell where it
/// stands: no Tab, nothing wide, nothing that combines.
fn one_cell_each(text: &str) -> bool {
    text.bytes().all(|b| (0x20..0x7f).contains(&b))
        || text.chars().all(|c| c != '\t' && char_cells(c) == 1)
}

/// The text of a row as it is built, and how many cells it has reached.
struct Composer {
    text: String,
    cells: usize,
    tab_width: usize,
}

impl Composer {
    /// Adds C: a Tab as the spaces to the next stop, anything else as itself.
    fn push(&mut self, c: char) {
        if c == '\t' {
            let reach = self.tab_width - self.cells % self.tab_width;
            self.text.extend(std::iter::repeat_n(' ', reach));
            self.cells += reach;
        } else {
            self.text.push(c);
            self.cells += char_cells(c);
        }
    }

    /// Adds TEXT, virtual, and the face it asked for over the cells it took.
    fn push_virtual(&mut self, text: &str, face: Face, faces: &mut Vec<(usize, usize, Face)>) {
        let start = self.cells;
        text.chars().for_each(|c| self.push(c));
        if self.cells > start {
            faces.push((start, self.cells, face));
        }
    }
}

/// Returns the row of LINE, whose TEXT starts at offset LINE_START, with the
/// virtual text that sits inside it, a Tab reaching to the next multiple of
/// TAB_WIDTH.
fn compose_row(
    line: usize,
    text: &str,
    line_start: usize,
    virtual_text: &VirtualTextTable,
    tab_width: usize,
) -> Row {
    let length = text.chars().count();
    // The newline's own offset is in range, so virtual text at the end of a
    // line is placed by anchoring it there.
    let inside: Vec<_> = if virtual_text.is_empty() {
        Vec::new()
    } else {
        virtual_text
            .between(line_start, line_start + length + 1)
            .collect()
    };
    if inside.is_empty() && one_cell_each(text) {
        return Row::plain(line, text.to_string());
    }
    let mut row = Composer {
        text: String::with_capacity(text.len()),
        cells: 0,
        tab_width,
    };
    let mut columns = Vec::with_capacity(length);
    let mut faces = Vec::new();
    let mut pending = inside.into_iter().peekable();
    for (column, c) in text.chars().enumerate() {
        // What is anchored at this column is drawn before its character.
        while let Some(entry) = pending.next_if(|entry| entry.at - line_start <= column) {
            row.push_virtual(&entry.text, entry.face, &mut faces);
        }
        let start = row.cells;
        row.push(c);
        columns.push((start, row.cells - start));
    }
    // What is anchored at the end of the line, or past it.
    for entry in pending {
        row.push_virtual(&entry.text, entry.face, &mut faces);
    }
    Row {
        line,
        text: row.text,
        columns: Some(Arc::new(columns)),
        length,
        cells: row.cells,
        faces,
        from: 0,
        to: row.cells,
        continuation: 0,
        last: true,
        wrapped: false,
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
        .flat_map(|highlight| {
            layout
                .rows
                .iter()
                .enumerate()
                .filter(move |(_, row)| row.line == highlight.line)
                .filter_map(move |(row, line)| {
                    clip(
                        Highlight {
                            row,
                            start_col: line.to_screen(highlight.start),
                            end_col: line.to_screen(highlight.end),
                            face: highlight.face,
                        },
                        scroll_x,
                        if line.last {
                            width
                        } else {
                            width.min(line.to - line.from)
                        },
                    )
                })
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
