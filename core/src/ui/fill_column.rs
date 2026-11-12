//! A fill-column guide drawn into the composed rows, shared by both hosts.
//! It replaces only an empty cell; buffer text and cursor mapping stay intact.
use super::width::{cells, char_cells};
use super::{Face, Highlight, Rect};
use std::collections::HashMap;

pub(crate) const SHOW_GUIDE: &str = "auto-fill-column-show-guide";
pub(crate) const GUIDE_CHARACTER: &str = "auto-fill-column-character";
pub(crate) const DEFAULT_CHARACTER: char = '│';

/// Effective columns of buffers with a guide, read before composition locks.
#[derive(Clone, Debug)]
pub struct FillColumnGuide {
    pub columns: HashMap<String, usize>,
    pub character: char,
}

impl FillColumnGuide {
    pub(crate) fn draw(
        &self,
        buffer: &str,
        rect: &Rect,
        scroll_x: usize,
        lines: &mut Vec<String>,
        highlights: &mut Vec<Highlight>,
    ) {
        let Some(column) = self
            .columns
            .get(buffer)
            .and_then(|c| c.checked_sub(scroll_x))
        else {
            return;
        };
        if column >= rect.width {
            return;
        }
        // Include the empty rows below EOF, but never the status line.
        lines.resize(rect.height, String::new());
        for (row, text) in lines.iter_mut().enumerate() {
            let mut cell = 0;
            let mut blank = None;
            for (byte, ch) in text.char_indices() {
                let width = char_cells(ch);
                if cell == column
                    && ch == ' '
                    && text[byte + 1..]
                        .chars()
                        .next()
                        .is_none_or(|c| char_cells(c) > 0)
                {
                    blank = Some(byte);
                    break;
                }
                if cell + width > column {
                    break;
                }
                cell += width;
            }
            if let Some(byte) = blank {
                text.replace_range(byte..byte + 1, &self.character.to_string());
            } else {
                let used = cells(text);
                if used > column {
                    continue; // Text, a wide character, or a combining sequence.
                }
                text.extend(std::iter::repeat_n(' ', column - used));
                text.push(self.character);
            }
            highlights.push(Highlight {
                row,
                start_col: column,
                end_col: column + 1,
                face: Face::FILL_COLUMN_INDICATOR,
            });
        }
    }
}
