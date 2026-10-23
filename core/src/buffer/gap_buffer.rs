//! [`GapBuffer`]: the text as one array with a gap at the cursor, and an index
//! of where its newlines are.
//!
//! Design: `doc/architettura-editor.typ, <gap-buffer>`.
use crate::buffer::BufferTrait;
use std::collections::VecDeque;

/// A text kept as one array of characters with a gap at the cursor: typing
/// fills the gap, deleting widens it, and moving the cursor moves it. An index
/// of the newlines answers questions about lines without a scan.
///
/// # Examples
///
/// ```
/// use rsedit_core::{BufferTrait, GapBuffer};
///
/// let text = GapBuffer::from("a\nb\nc");
/// assert_eq!(text.get_lines(1, 3), ["b", "c"]);
/// assert_eq!(text.cursor_1d_to_2d(4), (2, 0));
/// ```
pub struct GapBuffer {
    data: Vec<char>,
    gap_start: usize,
    gap_end: usize,

    // ---- line index -------------------------------------------------------
    //
    // Physical positions of every '\n', split around the gap and ascending
    // within each half. Physical rather than logical positions on purpose:
    // an edit only ever happens *at* the gap, and a physical position on
    // either side of the gap does not move when the gap grows or shrinks. So
    // typing a character updates the index in O(1) instead of shifting every
    // line start after the cursor.
    //
    // `before` is a Vec because it is only ever pushed and popped at its end.
    // `after` is a VecDeque because moving the gap migrates entries at its
    // front, which a Vec could only do by shifting the whole thing.
    newlines_before: Vec<usize>,
    newlines_after: VecDeque<usize>,
}

impl Default for GapBuffer {
    fn default() -> Self {
        Self {
            data: vec![char::default(); GapBuffer::GAPBUFFER_BASE_LEN],
            gap_start: 0,
            gap_end: GapBuffer::GAPBUFFER_BASE_LEN,
            newlines_before: Vec::new(),
            newlines_after: VecDeque::new(),
        }
    }
}

// TODO(BUF-3): `BufferTrait` requires `Clone`, and nothing clones a buffer
// today: the first caller that does panics here.
impl Clone for GapBuffer {
    fn clone(&self) -> Self {
        todo!("It is required to implement the Clone trait for GapBuffer");
    }
}

impl ToString for GapBuffer {
    fn to_string(&self) -> String {
        let ret = self.data[0..self.gap_start]
            .iter()
            .chain(self.data[self.gap_end..self.data.len()].iter())
            .collect::<String>();
        ret
    }
}

impl<'input> From<&'input str> for GapBuffer {
    fn from(text: &'input str) -> Self {
        let mut ret = Self::default();
        for c in text.chars() {
            ret.insert(c);
        }
        ret.move_gap(0);
        ret
    }
}

impl BufferTrait for GapBuffer {
    fn len(&self) -> usize {
        self.gap_start + self.data.len() - self.gap_end
    }

    fn chars_from(&self, pos: usize) -> impl Iterator<Item = char> + '_ {
        // The two halves of the storage on either side of the gap, as slices.
        // `at` branches on which side `pos` falls once per character; iterating
        // the halves decides it once for the whole walk.
        let front_start = pos.min(self.gap_start);
        let back_start = self
            .gap_end
            .saturating_add(pos.saturating_sub(self.gap_start))
            .min(self.data.len());
        self.data[front_start..self.gap_start]
            .iter()
            .chain(self.data[back_start..].iter())
            .copied()
    }

    fn at(&self, pos: usize) -> Option<char> {
        // `<`, not `<=`: at `pos == gap_start` the character is at
        // `data[gap_end]`, past the gap. `data[pos]` is inside the gap, and
        // holds whatever an earlier `copy_within` left there.
        if pos >= self.len() {
            None
        } else if pos < self.gap_start {
            Some(self.data[pos])
        } else {
            Some(self.data[pos + self.gap_end - self.gap_start])
        }
    }

    fn cursor_pos(&self) -> (usize, usize) {
        // TODO(BUF-2): counts from the start of the text at every call; the
        // line index could answer it.
        let mut line = 0;
        let mut col = 0;

        for &c in &self.data[..self.gap_start] {
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }

        (line, col)
    }

    fn cursor_pos_1d(&self) -> usize {
        self.gap_start
    }

    fn cursor_1d_to_2d(&self, pos: usize) -> (usize, usize) {
        // Binary search for how many newlines lie strictly before `pos`; that
        // count is the line number, and the column is the distance from the
        // line's start.
        let mut low = 0;
        let mut high = self.newline_count();
        while low < high {
            let mid = (low + high) / 2;
            if self.newline_logical(mid) < pos {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        let line = low;
        let col = pos - self.line_start(line).unwrap_or(0);
        (line, col)
    }

    fn cursor_2d_to_1d(&self, line: usize, col: usize) -> usize {
        match self.line_start(line) {
            Some(start) => (start + col).min(self.line_end(line)),
            None => self.len(),
        }
    }

    fn cursor_move_forward(&mut self) -> bool {
        if self.gap_start < self.len() {
            self.move_gap(self.gap_start + 1);
            true
        } else {
            false
        }
    }

    fn cursor_move_backward(&mut self) -> bool {
        if self.gap_start > 0 {
            self.move_gap(self.gap_start - 1);
            true
        } else {
            false
        }
    }

    fn cursor_move(&mut self, line: usize, col: usize) {
        let target = self.cursor_2d_to_1d(line, col);
        self.move_gap(target);
    }

    fn find_backward(&mut self, c: char) -> Option<usize> {
        let mut current_pos = self.cursor_pos_1d();
        while current_pos > 0 {
            if let Some(tc) = self.at(current_pos - 1) {
                if c == tc {
                    return Some(current_pos - 1);
                } else {
                    current_pos -= 1;
                }
            } else {
                unreachable!()
            }
        }
        None
    }

    fn find_forward(&mut self, c: char) -> Option<usize> {
        // TODO(BUF-3): `self.len() - 1` underflows on an empty text.
        let mut current_pos = self.cursor_pos_1d();
        while current_pos < self.len() - 1 {
            if let Some(tc) = self.at(current_pos + 1) {
                if c == tc {
                    return Some(current_pos + 1);
                } else {
                    current_pos += 1;
                }
            } else {
                unreachable!()
            }
        }
        None
    }

    fn insert(&mut self, c: char) {
        if self.gap_start == self.gap_end {
            self.gap_grow();
        }
        self.data[self.gap_start] = c;
        // The new character lands at the gap, so every newline already
        // recorded keeps its physical position: those before the gap are
        // below `gap_start`, those after it are at or above `gap_end`. A
        // newline typed here is simply the new largest of the "before" half.
        if c == '\n' {
            self.newlines_before.push(self.gap_start);
        }
        self.gap_start += 1;
    }

    fn delete(&mut self) {
        if self.gap_start > 0 {
            self.gap_start -= 1;
            // The character just swallowed by the gap sat at `gap_start`; if
            // it was a newline it was the last entry of the "before" half.
            if self.data[self.gap_start] == '\n' {
                self.newlines_before.pop();
            }
        }
    }

    fn clear(&mut self) {
        // A fresh buffer, assigned whole: one allocation rather than a gap
        // move per character, and every field reset with it -- the line index
        // included, and whatever is added to `GapBuffer` later.
        *self = Self::default();
    }

    fn line_count(&self) -> usize {
        self.newline_count() + 1
    }

    fn get_lines(&self, start_line: usize, end_line: usize) -> Vec<String> {
        let capacity = end_line.saturating_sub(start_line);
        let mut lines = Vec::with_capacity(capacity);

        // Straight to the first line wanted, through the index: reading a
        // viewport costs the viewport, not the text before it.
        if let Some(mut pos) = self.line_start(start_line) {
            let len = self.len();
            let mut line = start_line;
            let mut current = String::new();

            while pos < len && line < end_line {
                let c = self.logical_char(pos);
                pos += 1;
                if c == '\n' {
                    lines.push(std::mem::take(&mut current));
                    line += 1;
                } else {
                    current.push(c);
                }
            }

            // The final line, when the text does not end in a newline.
            if line < end_line {
                lines.push(current);
            }
        }

        // Lines past the end of the text, empty.
        while lines.len() < capacity {
            lines.push(String::new());
        }

        lines
    }
}

impl GapBuffer {
    /// How many characters a new buffer has room for before the gap grows.
    const GAPBUFFER_BASE_LEN: usize = 1024;

    // ---- line index: reading it ------------------------------------------

    /// Returns the width of the gap: how far a physical position after the
    /// gap sits ahead of its logical one.
    pub(crate) fn gap_width(&self) -> usize {
        self.gap_end - self.gap_start
    }

    /// Returns the logical (text) position of a physical `data` index.
    fn logical_of(&self, physical: usize) -> usize {
        if physical < self.gap_start {
            physical
        } else {
            physical - self.gap_width()
        }
    }

    /// Returns the character at logical position I, skipping over the gap.
    fn logical_char(&self, i: usize) -> char {
        let physical = if i < self.gap_start {
            i
        } else {
            i + self.gap_width()
        };
        self.data[physical]
    }

    /// Returns how many newlines the text holds.
    fn newline_count(&self) -> usize {
        self.newlines_before.len() + self.newlines_after.len()
    }

    /// Returns the logical position of the K-th newline in the text, from 0.
    fn newline_logical(&self, k: usize) -> usize {
        let physical = if k < self.newlines_before.len() {
            self.newlines_before[k]
        } else {
            self.newlines_after[k - self.newlines_before.len()]
        };
        self.logical_of(physical)
    }

    /// Returns the logical position where LINE begins, in O(1), or `None`
    /// when the text has no such line. Line 0 begins at 0; every later line
    /// one past the newline that ended the line before.
    fn line_start(&self, line: usize) -> Option<usize> {
        if line == 0 {
            return Some(0);
        }
        if line > self.newline_count() {
            return None;
        }
        Some(self.newline_logical(line - 1) + 1)
    }

    /// Returns the logical position one past the last character of LINE, not
    /// counting its newline.
    fn line_end(&self, line: usize) -> usize {
        if line < self.newline_count() {
            self.newline_logical(line)
        } else {
            self.len()
        }
    }

    // ---- line index: keeping it correct ----------------------------------

    /// Checks the whole index against a scan of the text, and panics where
    /// the two disagree. Only for tests: nothing in normal operation needs
    /// it, which is the property worth protecting.
    #[cfg(test)]
    pub(crate) fn assert_index_valid(&self) {
        let scanned: Vec<usize> = (0..self.len())
            .filter(|&i| self.logical_char(i) == '\n')
            .collect();
        let indexed: Vec<usize> = (0..self.newline_count())
            .map(|k| self.newline_logical(k))
            .collect();
        assert_eq!(
            indexed, scanned,
            "the line index disagrees with the text it indexes"
        );
        assert!(
            self.newlines_before.iter().all(|&p| p < self.gap_start),
            "a newline recorded before the gap is not before the gap"
        );
        assert!(
            self.newlines_after.iter().all(|&p| p >= self.gap_end),
            "a newline recorded after the gap is not after the gap"
        );
    }

    /// Moves the gap, and with it the cursor, to NEW_CURSOR_POS. A position
    /// past the end of the text is ignored.
    pub fn move_gap(&mut self, new_cursor_pos: usize) {
        if new_cursor_pos > self.data.len() - self.gap_end + self.gap_start {
            return;
        }
        // Moving the gap is the one operation that relocates text, so it is
        // also the one that has to migrate index entries between the halves.
        // Only the newlines inside the moved span are touched; the cost is
        // proportional to the distance moved, which `copy_within` already
        // pays anyway.
        let width = self.gap_width();
        if new_cursor_pos < self.gap_start {
            let range = self.gap_start - new_cursor_pos;
            // Text moves right across the gap: its physical positions gain
            // the gap width. Popping largest-first and pushing to the front
            // leaves `newlines_after` ascending.
            while let Some(&physical) = self.newlines_before.last() {
                if physical < new_cursor_pos {
                    break;
                }
                self.newlines_before.pop();
                self.newlines_after.push_front(physical + width);
            }
            self.data
                .copy_within(new_cursor_pos..self.gap_start, self.gap_end - range);
            self.gap_start -= range;
            self.gap_end -= range;
        } else if new_cursor_pos > self.gap_start {
            let range = new_cursor_pos - self.gap_start;
            // Text moves left across the gap: its physical positions lose the
            // gap width, and the entries leave `newlines_after` in order.
            while let Some(&physical) = self.newlines_after.front() {
                if physical >= self.gap_end + range {
                    break;
                }
                self.newlines_after.pop_front();
                self.newlines_before.push(physical - width);
            }
            self.data
                .copy_within(self.gap_end..self.gap_end + range, self.gap_start);
            self.gap_start += range;
            self.gap_end += range;
        }
    }

    /// Doubles the storage, and widens the gap by what it adds.
    pub fn gap_grow(&mut self) {
        let old_data_len = self.data.len();
        let new_data_len = 2 * old_data_len;
        let mut new_data = vec![char::default(); new_data_len];
        new_data[0..self.gap_start].copy_from_slice(&self.data[0..self.gap_start]);
        new_data[self.gap_end + old_data_len..new_data_len]
            .copy_from_slice(&self.data[self.gap_end..self.data.len()]);
        _ = core::mem::replace(&mut self.data, new_data);
        // Everything after the gap slid to the far end of the larger buffer,
        // so the physical positions recorded for it slid by the same amount.
        // O(lines after the cursor), but amortised over a doubling that
        // already copies the whole buffer.
        for physical in self.newlines_after.iter_mut() {
            *physical += old_data_len;
        }
        self.gap_end += old_data_len;
    }
}
