//! Traversing visual rows without laying out the whole buffer. A viewport
//! keeps a logical line and a continuation within it; motion composes only
//! the lines it crosses, using the same rows as drawing and hit testing.
use super::layout::{Layout, Row};
use crate::buffer::{BufferTrait, VirtualTextTable};
use std::{cell::RefCell, sync::Arc};

pub const MODE: &str = "visual-line-mode";

/// A row's address, ordered as it appears in the buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct VisualPoint {
    pub line: usize,
    pub row: usize,
}

pub(crate) struct VisualLines<'a, B: BufferTrait> {
    text: &'a B,
    virtual_text: &'a VirtualTextTable,
    width: usize,
    tab_width: usize,
    // At most one logical line, shared by all the questions in a traversal.
    // Walking a large count does not retain the lines already crossed.
    cached: RefCell<Option<(usize, Arc<Vec<Row>>)>>,
}

impl<'a, B: BufferTrait> VisualLines<'a, B> {
    pub fn new(
        text: &'a B,
        virtual_text: &'a VirtualTextTable,
        width: usize,
        tab_width: usize,
    ) -> Self {
        Self {
            text,
            virtual_text,
            width,
            tab_width,
            cached: RefCell::new(None),
        }
    }

    pub fn line(&self, line: usize) -> Arc<Vec<Row>> {
        if let Some((number, rows)) = &*self.cached.borrow()
            && *number == line
        {
            return rows.clone();
        }
        let rows = Arc::new(
            Layout::compose(self.text, self.virtual_text, line, 1, self.tab_width)
                .rows()
                .first()
                .cloned()
                .map(|row| row.wrap(self.width))
                .unwrap_or_default(),
        );
        *self.cached.borrow_mut() = Some((line, rows.clone()));
        rows
    }

    pub fn normalize(&self, at: VisualPoint) -> VisualPoint {
        let line = at.line.min(self.text.line_count().saturating_sub(1));
        VisualPoint {
            line,
            row: at.row.min(self.line(line).len().saturating_sub(1)),
        }
    }

    pub fn position(&self, line: usize, column: usize) -> (VisualPoint, usize) {
        let rows = self.line(line);
        let index = rows
            .iter()
            .position(|row| row.contains(column))
            .unwrap_or_else(|| rows.len().saturating_sub(1));
        let cell = rows.get(index).map_or(0, |row| row.to_screen(column));
        (VisualPoint { line, row: index }, cell)
    }

    pub fn end(&self) -> VisualPoint {
        let line = self.text.line_count().saturating_sub(1);
        VisualPoint {
            line,
            row: self.line(line).len().saturating_sub(1),
        }
    }

    pub fn advance(&self, at: VisualPoint, delta: isize) -> VisualPoint {
        let mut at = self.normalize(at);
        let mut remaining = delta.unsigned_abs();
        while remaining > 0 {
            let last = self.line(at.line).len().saturating_sub(1);
            if delta >= 0 {
                let available = last.saturating_sub(at.row);
                if remaining <= available {
                    at.row += remaining;
                    break;
                }
                if at.line + 1 >= self.text.line_count() {
                    at.row = last;
                    break;
                }
                remaining -= available + 1;
                at.line += 1;
                at.row = 0;
            } else {
                if remaining <= at.row {
                    at.row -= remaining;
                    break;
                }
                if at.line == 0 {
                    at.row = 0;
                    break;
                }
                remaining -= at.row + 1;
                at.line -= 1;
                at.row = self.line(at.line).len().saturating_sub(1);
            }
        }
        at
    }

    pub fn compose(&self, at: VisualPoint, height: usize) -> Layout {
        if height == 0 {
            return Layout::default();
        }
        let at = self.normalize(at);
        let mut rows = Vec::new();
        for line in at.line..self.text.line_count() {
            if rows.len() >= height {
                break;
            }
            let skip = if line == at.line { at.row } else { 0 };
            let remaining = height - rows.len();
            rows.extend(self.line(line).iter().skip(skip).take(remaining).cloned());
        }
        Layout::from_rows(rows)
    }
}
