//! How many cells of a terminal, or of the graphical grid, a character takes:
//! two for an ideogram or an emoji, none for a combining accent, one for the
//! rest. A Tab is the layout's to expand, since its width depends on where it
//! stands; anywhere else it is a control character, drawn as a picture one
//! cell wide.
//!
//! The one measure every front end and the layout share, so that the text,
//! the cursor, the highlights and a click fall on the same cells.
//!
//! Design: `doc/architettura-editor.typ, <righe>`.
use unicode_width::UnicodeWidthChar;

/// Returns how many cells C takes.
///
/// A control character takes one: a front end draws it as a picture of
/// itself, so that nothing in a file is obeyed by the terminal.
pub fn char_cells(c: char) -> usize {
    if c.is_control() {
        return 1;
    }
    c.width().unwrap_or(1)
}

/// Returns how many cells TEXT takes.
pub fn cells(text: &str) -> usize {
    text.chars().map(char_cells).sum()
}

/// Returns the part of TEXT that falls on cells FROM to FROM + WIDTH.
///
/// A wide character cut by either edge becomes a space for each of its cells
/// inside, so that what comes after it stays where it was counted; a
/// combining character goes with the one it combines with.
pub fn cut(text: &str, from: usize, width: usize) -> String {
    let end = from + width;
    let mut out = String::new();
    let mut at = 0;
    let mut kept = false;
    for c in text.chars() {
        let cells = char_cells(c);
        if cells == 0 {
            if kept {
                out.push(c);
            }
            continue;
        }
        let (start, stop) = (at, at + cells);
        at = stop;
        if stop <= from {
            kept = false;
            continue;
        }
        if start >= end {
            break;
        }
        if start >= from && stop <= end {
            out.push(c);
            kept = true;
        } else {
            out.extend(std::iter::repeat_n(' ', stop.min(end) - start.max(from)));
            kept = false;
        }
    }
    out
}

/// Returns TEXT cut to WIDTH cells and padded with spaces to exactly WIDTH.
pub fn fit(text: &str, width: usize) -> String {
    let mut out = cut(text, 0, width);
    let used = cells(&out);
    out.extend(std::iter::repeat_n(' ', width.saturating_sub(used)));
    out
}
