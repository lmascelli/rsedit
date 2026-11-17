//! The findings a frame draws over a buffer: each span in the face of its
//! severity, a sign in the gutter on the line where one starts, a count for
//! the status line, and the message of the one at point. They come in as
//! faces, signs and text, read before the frame's locks; only where each one
//! is now is read under them, from the buffer's markers.
//!
//! Design: `doc/architettura-editor.typ, <segnalazioni-schermo>`.
use std::collections::HashMap;

use crate::buffer::{Buffer, BufferTrait};
use crate::managers::Buffers;
use crate::text::positions::{self, Unit};
use crate::ui::Face;
use crate::ui::layout::BufferHighlight;
use crate::ui::windows::{Rect, Window};

/// How many columns the signs take at the left of a gutter: the sign, and a
/// space before the numbers or the text.
pub const SIGN_COLUMNS: usize = 2;

/// One finding, as a frame draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct FindingMark {
    /// The ids of the markers at its start and its end in the buffer, when
    /// the buffer has them.
    pub markers: Option<(u64, u64)>,
    /// Where its producer said it starts and ends, a line from 0 and a
    /// column: where it is drawn when the buffer has not both markers.
    pub start: (usize, usize),
    pub end: (usize, usize),
    /// What the columns of `start` and `end` count.
    pub unit: Unit,
    /// How much it matters, 0 the most: on a line, and at point, the one
    /// that matters most is shown.
    pub rank: u8,
    pub face: Face,
    /// One cell wide.
    pub sign: char,
    pub sign_face: Face,
    /// What the echo area says while point is on it: one line.
    pub message: String,
}

/// The findings drawn over one buffer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BufferFindings {
    /// In the order they are drawn, each over the one before: those that
    /// matter least first.
    pub marks: Vec<FindingMark>,
    /// What `%!` shows in the status line of a window showing the buffer.
    pub count: String,
}

/// The findings a frame draws, by the name of the buffer they are over.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FindingMarks {
    pub buffers: HashMap<String, BufferFindings>,
}

impl FindingMarks {
    /// Returns the findings drawn over BUFFER, if it has any.
    pub fn of(&self, buffer: &str) -> Option<&BufferFindings> {
        self.buffers
            .get(buffer)
            .filter(|found| !found.marks.is_empty())
    }
}

/// Returns the offset of LINE, from 0, and COLUMN, counted in UNIT, in
/// TEXT, both kept within the text.
fn offset_of<B: BufferTrait>(text: &B, (line, column): (usize, usize), unit: Unit) -> usize {
    let line = line.min(text.line_count().saturating_sub(1));
    if unit != Unit::Char {
        return positions::position(text, line, column, unit);
    }
    let length = text
        .get_lines(line, line + 1)
        .first()
        .map(|text| text.chars().count())
        .unwrap_or(0);
    text.cursor_2d_to_1d(line, column.min(length))
}

/// Returns where each of MARKS is in BUF now, as offsets, the end not before
/// the start: at its markers when BUF has both, where its producer said
/// otherwise.
fn spans<'a, B: BufferTrait>(
    buf: &Buffer<B>,
    marks: &'a [FindingMark],
) -> impl Iterator<Item = (usize, usize, &'a FindingMark)> {
    marks.iter().map(|mark| {
        let placed = mark
            .markers
            .and_then(|(start, end)| Some((buf.markers.get(start)?, buf.markers.get(end)?)));
        let (start, end) = placed.unwrap_or_else(|| {
            (
                offset_of(&buf.text, mark.start, mark.unit),
                offset_of(&buf.text, mark.end, mark.unit),
            )
        });
        let start = start.min(buf.text.len());
        (start, end.min(buf.text.len()).max(start), mark)
    })
}

/// Returns the findings MARKS has for WIN's buffer on the lines WIN shows in
/// RECT, in buffer columns, those that matter most last. A finding with no
/// span takes the cell at its start, the one after the line when it is at
/// the line's end.
pub fn finding_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
    marks: Option<&FindingMarks>,
) -> Vec<BufferHighlight> {
    let Some(found) = marks.and_then(|marks| marks.of(&win.buffer_name)) else {
        return Vec::new();
    };
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Vec::new();
    };
    let buf = buf
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let first = win.scroll_y;
    let last = (win.scroll_y + rect.height).min(buf.text.line_count());
    if first >= last {
        return Vec::new();
    }
    let mut highlights = Vec::new();
    for (start, end, mark) in spans(&buf, &found.marks) {
        if start == end {
            let (line, column) = buf.text.cursor_1d_to_2d(start);
            if (first..last).contains(&line) {
                highlights.push(BufferHighlight {
                    line,
                    start: column,
                    end: column + 1,
                    face: mark.face,
                });
            }
            continue;
        }
        highlights.extend(super::compose::span_highlights(
            &buf.text,
            (start, end),
            (first, last),
            mark.face,
        ));
    }
    highlights
}

/// Returns the sign of each line from FIRST to LAST, the last excluded, on
/// which one of FOUND starts in BUF: that of the one that matters most.
pub(crate) fn line_signs<B: BufferTrait>(
    buf: &Buffer<B>,
    found: &BufferFindings,
    (first, last): (usize, usize),
) -> HashMap<usize, (char, Face)> {
    let mut signs: HashMap<usize, (u8, char, Face)> = HashMap::new();
    for (start, _, mark) in spans(buf, &found.marks) {
        let (line, _) = buf.text.cursor_1d_to_2d(start);
        if !(first..last).contains(&line) {
            continue;
        }
        let sign = (mark.rank, mark.sign, mark.sign_face);
        signs
            .entry(line)
            .and_modify(|kept| {
                if sign.0 < kept.0 {
                    *kept = sign;
                }
            })
            .or_insert(sign);
    }
    signs
        .into_iter()
        .map(|(line, (_, sign, face))| (line, (sign, face)))
        .collect()
}

/// Returns what the echo area says with point where it is in BUFFER: the
/// message of the finding of MARKS that matters most among those point is
/// on, the first given of them when several matter as much, and how many
/// more there are. Point is on a finding from its start to just before its
/// end, and on one with no span at its start.
pub fn finding_message_at<B: BufferTrait>(
    buffers: &Buffers<B>,
    buffer: &str,
    marks: Option<&FindingMarks>,
) -> Option<String> {
    let found = marks?.of(buffer)?;
    let buf = buffers.handle(buffer)?;
    let buf = buf
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let point = buf.text.cursor_pos_1d();
    let under: Vec<&FindingMark> = spans(&buf, &found.marks)
        .filter(|&(start, end, _)| start <= point && point < end.max(start + 1))
        .map(|(_, _, mark)| mark)
        .collect();
    let shown = under.iter().min_by_key(|mark| mark.rank)?;
    Some(match under.len() - 1 {
        0 => shown.message.clone(),
        more => format!("{} (and {more} more)", shown.message),
    })
}
