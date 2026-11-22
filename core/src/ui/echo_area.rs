//! The echo area: the rows at the bottom of the frame where the editor says
//! things, one for each line of what it says, as many as
//! `max-echo-area-height` lets it take from the windows above.
//!
//! Design: `doc/architettura-editor.typ, <snapshot>`.
use super::width::{cells, cut};

/// Returns the rows the echo area shows for TEXT in a frame WIDTH cells wide:
/// one for each line of TEXT, LIMIT at most and one at least.
///
/// A line ends at a newline, or at a carriage return and a newline, as a
/// command's output does on Windows; the newline that ends TEXT ends its last
/// line and starts none, and an empty TEXT is one empty row. A line longer
/// than the frame is the front end's to cut, as it was when the echo area was
/// a row: it is not wrapped. When TEXT has more lines than LIMIT, the first
/// LIMIT are shown, and the last of them is cut short enough to end with how
/// many were left out, so that what cannot be seen is said.
pub fn echo_rows(text: &str, limit: usize, width: usize) -> Vec<String> {
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if lines.len() > 1 && lines.last() == Some(&"") {
        lines.pop();
    }
    let limit = limit.max(1);
    if lines.len() <= limit {
        return lines.into_iter().map(str::to_owned).collect();
    }
    let marker = left_out(lines.len() - limit);
    let mut rows: Vec<String> = lines[..limit].iter().map(|&line| line.to_owned()).collect();
    if let Some(last) = rows.last_mut() {
        let mut shortened = cut(last, 0, width.saturating_sub(cells(&marker)));
        shortened.push_str(&marker);
        *last = shortened;
    }
    rows
}

/// Returns what ends the last row of the echo area when COUNT lines of the
/// message did not fit in it.
fn left_out(count: usize) -> String {
    match count {
        1 => " [+1 line]".to_string(),
        count => format!(" [+{count} lines]"),
    }
}
