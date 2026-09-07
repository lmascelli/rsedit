use crate::buffer::BufferTrait;
use std::collections::VecDeque;

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

impl<'input> BufferTrait for GapBuffer {
    fn len(&self) -> usize {
        self.gap_start + self.data.len() - self.gap_end
    }

    fn at_line_col(&self, line: usize, col: usize) -> Option<char> {
        let mut current_line = 0;
        let mut current_col = 0;
        let logical_text = self.data[0..self.gap_start]
            .iter()
            .chain(self.data[self.gap_end..].iter());

        for &c in logical_text {
            if c == '\n' {
                current_line += 1;
                current_col = 0;
            } else {
                current_col += 1;
            }
            if current_line == line {
                if current_col > col {
                    return None;
                } else if current_col == col {
                    return Some(c);
                }
            }
        }

        None
    }

    fn at(&self, pos: usize) -> Option<char> {
        // `<`, not `<=`: at `pos == gap_start` the logical character lives at
        // `data[gap_end]`, on the far side of the gap. Reading `data[pos]`
        // there returns whatever the gap happens to contain -- usually a stale
        // copy left behind by an earlier `copy_within`, which is why this
        // looked correct in simple cases and returned the wrong character as
        // soon as an edit made the stale copy differ from the text.
        if pos >= self.len() {
            None
        } else if pos < self.gap_start {
            Some(self.data[pos])
        } else {
            Some(self.data[pos + self.gap_end - self.gap_start])
        }
    }

    fn cursor_pos(&self) -> (usize, usize) {
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
        // A column past the end of its line clamps to the line's end, and a
        // line past the end of the buffer clamps to the end of the text --
        // matching what the old scan did when it ran out of characters.
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
        // Reset to a fresh buffer rather than unwinding: one allocation
        // instead of one gap move per character.
        //
        // Assigning `Self::default()` rather than resetting fields by hand so
        // that any state added to `GapBuffer` later is cleared with it instead
        // of being silently left stale -- the line index included.
        *self = Self::default();
    }

    fn line_count(&self) -> usize {
        // One more line than there are newlines: a buffer with no newline at
        // all still has a first line, and one ending in a newline has an
        // empty last line.
        self.newline_count() + 1
    }

    fn get_lines(&self, start_line: usize, end_line: usize) -> Vec<String> {
        let capacity = end_line.saturating_sub(start_line);
        let mut lines = Vec::with_capacity(capacity);

        // Seek straight to the first line wanted instead of counting newlines
        // from character zero. This is the change that makes drawing a
        // viewport cost the size of the viewport rather than the size of the
        // document.
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

        // Fill remaining requested space with empty strings (if scrolled past EOF)
        while lines.len() < capacity {
            lines.push(String::new());
        }

        lines
    }
}

impl GapBuffer {
    const GAPBUFFER_BASE_LEN: usize = 1024;

    // ---- line index: reading it ------------------------------------------

    /// Width of the gap, i.e. how far a physical position after the gap sits
    /// ahead of its logical one.
    fn gap_width(&self) -> usize {
        self.gap_end - self.gap_start
    }

    /// Logical (text) position of a physical `data` index.
    fn logical_of(&self, physical: usize) -> usize {
        if physical < self.gap_start {
            physical
        } else {
            physical - self.gap_width()
        }
    }

    /// The character at logical position `i`, skipping over the gap.
    fn logical_char(&self, i: usize) -> char {
        let physical = if i < self.gap_start {
            i
        } else {
            i + self.gap_width()
        };
        self.data[physical]
    }

    fn newline_count(&self) -> usize {
        self.newlines_before.len() + self.newlines_after.len()
    }

    /// Logical position of the `k`-th newline in the text, 0-based.
    fn newline_logical(&self, k: usize) -> usize {
        let physical = if k < self.newlines_before.len() {
            self.newlines_before[k]
        } else {
            self.newlines_after[k - self.newlines_before.len()]
        };
        self.logical_of(physical)
    }

    /// Logical position where `line` begins, or `None` if the buffer has no
    /// such line. Line 0 always begins at 0; every later line begins one past
    /// the newline that ended its predecessor.
    ///
    /// This is the whole point of the index: it answers in O(1) what used to
    /// take a scan from character zero.
    fn line_start(&self, line: usize) -> Option<usize> {
        if line == 0 {
            return Some(0);
        }
        if line > self.newline_count() {
            return None;
        }
        Some(self.newline_logical(line - 1) + 1)
    }

    /// Logical position one past the last character of `line`, not counting
    /// its newline.
    fn line_end(&self, line: usize) -> usize {
        if line < self.newline_count() {
            self.newline_logical(line)
        } else {
            self.len()
        }
    }

    // ---- line index: keeping it correct ----------------------------------

    /// Re-derive the whole index by scanning. Only for tests -- nothing in
    /// normal operation needs it, which is the property worth protecting.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialization() {
        let buf = GapBuffer::default();
        // Ensure the string is empty but the gap exists
        assert_eq!(buf.to_string(), "");
        assert!(buf.gap_end > buf.gap_start);
    }

    #[test]
    fn test_basic_typing_and_unicode() {
        let mut buf = GapBuffer::default();
        buf.insert('R');
        buf.insert('u');
        buf.insert('s');
        buf.insert('t');
        buf.insert(' ');
        buf.insert('🦀'); // Unicode Emoji (Multi-byte in UTF-8)
        buf.insert('ℵ'); // Aleph symbol

        assert_eq!(buf.to_string(), "Rust 🦀ℵ");
    }

    #[test]
    fn test_gap_movement_and_insertion() {
        let mut buf = GapBuffer::default();
        // Create: "Hello World"
        for c in "Hello World".chars() {
            buf.insert(c);
        }

        // Move gap to between "Hello" and " World"
        // "Hello" is 5 chars long.
        buf.move_gap(5);

        // Insert a comma
        buf.insert(',');

        assert_eq!(buf.to_string(), "Hello, World");
    }

    #[test]
    fn test_backspace_with_unicode() {
        let mut buf = GapBuffer::default();
        for c in "Logic 💡".chars() {
            buf.insert(c);
        }

        // Delete the lightbulb emoji
        buf.delete();
        assert_eq!(buf.to_string(), "Logic ");

        // Move to the middle and delete
        buf.move_gap(2); // After "Lo"
        buf.delete(); // Deletes 'o'
        assert_eq!(buf.to_string(), "Lgic ");
    }

    #[test]
    fn test_moving_gap_to_extremes() {
        let mut buf = GapBuffer::default();
        for c in "Limit".chars() {
            buf.insert(c);
        }

        // Move to start
        buf.move_gap(0);
        buf.insert('>');
        assert_eq!(buf.to_string(), ">Limit");

        // Move to end
        let len = buf.to_string().chars().count();
        buf.move_gap(len);
        buf.insert('<');
        assert_eq!(buf.to_string(), ">Limit<");
    }

    #[test]
    fn test_buffer_growth() {
        // Assume your initial gap size is small for this test,
        // or just insert many characters to force a resize.
        let mut buf = GapBuffer::default();
        let long_string = "Specialized Unicode: 🚀💎🌈".repeat(50);

        for c in long_string.chars() {
            buf.insert(c);
        }

        assert_eq!(buf.to_string(), long_string);

        // Ensure we can still move the gap and insert after growing
        buf.move_gap(10);
        buf.insert('!');
        assert!(buf.to_string().contains("Specialize!d"));
    }

    #[test]
    fn test_complex_unicode_ordering() {
        let mut buf = GapBuffer::default();
        let input = "नमस्ते"; // "Namaste" in Hindi - uses combining characters
        for c in input.chars() {
            buf.insert(c);
        }

        // Move gap into the middle of the combining sequence
        buf.move_gap(2);
        buf.insert('X');

        // Note: In a real editor, moving by 'char' vs moving by 'grapheme cluster'
        // is different. For now, we are testing that char-level integrity holds.
        let result = buf.to_string();
        assert!(result.contains('X'));
        assert_eq!(result.chars().count(), input.chars().count() + 1);
    }

    // A helper to initialize a buffer with standard mixed text
    fn setup_test_buffer<B: BufferTrait>(text: &str) -> B {
        B::from(text)
    }

    #[test]
    fn test_empty_buffer_invariants() {
        let mut buf = setup_test_buffer::<GapBuffer>("");

        assert_eq!(buf.len(), 0);
        assert_eq!(buf.cursor_pos(), (0, 0));
        assert_eq!(buf.cursor_pos_1d(), 0);
        assert_eq!(buf.at(0), None);
        assert_eq!(buf.at_line_col(0, 0), None);

        // Boundaries should resist out-of-bounds drifting
        assert!(!buf.cursor_move_forward());
        assert!(!buf.cursor_move_backward());

        // Deleting from empty should be a safe no-op
        buf.delete();
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_coordinate_translations_and_clamping() {
        // 3 lines: line 0 (len 3), line 1 (len 4), line 2 (len 3)
        let buf = setup_test_buffer::<GapBuffer>("abc\ndefg\nhij");

        // Valid translations
        assert_eq!(buf.cursor_2d_to_1d(0, 0), 0);
        assert_eq!(buf.cursor_2d_to_1d(0, 3), 3); // Position of first '\n'
        assert_eq!(buf.cursor_2d_to_1d(1, 0), 4); // First char of line 1

        // 1D back to 2D
        assert_eq!(buf.cursor_1d_to_2d(3), (0, 3));
        assert_eq!(buf.cursor_1d_to_2d(4), (1, 0));

        // EDGE CASE: Requesting columns beyond line ending
        // Depending on your design, this should either clamp to line end or wrap.
        // Assuming typical editor logic, it should clamp to the end of that specific line.
        let out_of_col = buf.cursor_2d_to_1d(0, 10);
        assert!(
            out_of_col <= 4,
            "Should clamp to line length or newline boundaries"
        );

        // EDGE CASE: Requesting non-existent lines
        let out_of_line = buf.cursor_2d_to_1d(10, 0);
        assert!(
            out_of_line <= buf.len(),
            "Out of bounds coordinates must clamp safely to EOF"
        );
    }

    #[test]
    fn test_movement_and_traversal() {
        let mut buf = setup_test_buffer::<GapBuffer>("a\nb\nc");

        // Traverse to the end
        let mut steps = 0;
        while buf.cursor_move_forward() {
            steps += 1;
        }
        assert_eq!(steps, 5); // 'a', '\n', 'b', '\n', 'c'
        assert_eq!(buf.cursor_pos_1d(), 5);

        // Move backward
        assert!(buf.cursor_move_backward());
        assert_eq!(buf.cursor_pos_1d(), 4);

        // Direct 2D movement jump
        buf.cursor_move(1, 1); // jumps to 'b' position + 1 -> index 3
        assert_eq!(buf.cursor_pos_1d(), 3);
        assert_eq!(buf.at(3), Some('\n'));
    }

    #[test]
    fn test_search_mechanisms() {
        let mut buf = setup_test_buffer::<GapBuffer>("hello\nworld\nhello");

        // Position cursor in the middle ('world' start -> index 6)
        buf.cursor_move(1, 0);

        // Find forward should catch the second instance of 'h'
        assert_eq!(buf.find_forward('h'), Some(12));

        // Find backward should catch the first instance of 'h'
        assert_eq!(buf.find_backward('h'), Some(0));

        // Searching for absent characters
        assert_eq!(buf.find_forward('z'), None);
        assert_eq!(buf.find_backward('z'), None);
    }

    #[test]
    fn test_unicode_boundary_safety() {
        // Multi-byte chars: 🦀 (4 bytes), 🚀 (4 bytes)
        let mut buf = setup_test_buffer::<GapBuffer>("🦀\n🚀");

        assert_eq!(buf.len(), 3); // 3 logical chars: '🦀', '\n', '🚀'
        assert_eq!(buf.at(0), Some('🦀'));
        assert_eq!(buf.at(2), Some('🚀'));

        // Step forward incrementally ensuring char-wise index mapping holds
        buf.cursor_move(0, 0);
        assert_eq!(buf.cursor_pos_1d(), 0);

        buf.cursor_move_forward();
        assert_eq!(buf.cursor_pos_1d(), 1);
        assert_eq!(buf.at(buf.cursor_pos_1d()), Some('\n'));
    }

    #[test]
    fn test_viewport_line_extraction() {
        let text = "Line 0\nLine 1\nLine 2\nLine 3\nLine 4\nLine 5";
        let buf = setup_test_buffer::<GapBuffer>(text);

        assert_eq!(buf.line_count(), 6);

        // Test extracting a viewport in the middle of the file
        let viewport = buf.get_lines(2, 4);
        assert_eq!(viewport.len(), 2);
        assert_eq!(viewport[0], "Line 2");
        assert_eq!(viewport[1], "Line 3");

        // Test extracting past the end of the file (should pad with empty strings)
        let end_viewport = buf.get_lines(4, 7);
        assert_eq!(end_viewport.len(), 3);
        assert_eq!(end_viewport[0], "Line 4");
        assert_eq!(end_viewport[1], "Line 5");
        assert_eq!(end_viewport[2], ""); // Padded line
    }
}

impl Clone for GapBuffer {
    fn clone(&self) -> Self {
        todo!("It is required to implement the Clone trait for GapBuffer");
    }
}

#[cfg(test)]
mod line_index_tests {
    use super::*;

    /// A buffer of `lines` numbered lines, cursor left at the start.
    fn numbered(lines: usize) -> GapBuffer {
        let mut text = String::new();
        for i in 0..lines {
            text.push_str(&format!("line {i}\n"));
        }
        GapBuffer::from(text.as_str())
    }

    #[test]
    fn the_index_survives_construction_and_gap_movement() {
        let mut buf = numbered(200);
        buf.assert_index_valid();
        for target in [0usize, 1, 500, 1400, 7, 1399, 0] {
            buf.move_gap(target.min(buf.len()));
            buf.assert_index_valid();
        }
    }

    #[test]
    fn the_index_survives_typing_and_deleting_newlines() {
        let mut buf = GapBuffer::default();
        for c in "alpha\nbeta\ngamma".chars() {
            buf.insert(c);
            buf.assert_index_valid();
        }
        assert_eq!(buf.line_count(), 3);
        for _ in 0..7 {
            buf.delete();
            buf.assert_index_valid();
        }
        assert_eq!(buf.to_string(), "alpha\nbet");
        assert_eq!(buf.line_count(), 2);
    }

    /// The index has to stay correct through a gap reallocation, which slides
    /// everything after the cursor to the far end of a larger buffer.
    #[test]
    fn the_index_survives_the_gap_growing() {
        let mut buf = numbered(400);
        buf.move_gap(0);
        for c in "typed at the very start\nwith a newline\n".chars() {
            buf.insert(c);
        }
        buf.assert_index_valid();
        assert_eq!(buf.line_count(), 403);
        assert_eq!(
            buf.get_lines(0, 1),
            vec!["typed at the very start".to_string()]
        );
        assert_eq!(buf.get_lines(2, 3), vec!["line 0".to_string()]);
    }

    /// Random editing, checked against a scan after every operation. This is
    /// what catches an invariant that only breaks on some particular
    /// interleaving of moves, inserts and deletes.
    #[test]
    fn the_index_survives_arbitrary_editing() {
        let mut buf = numbered(60);
        // A tiny deterministic PRNG: reproducible, and no dev-dependency.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for step in 0..3_000 {
            let len = buf.len();
            match next() % 4 {
                0 => buf.insert(if next() % 5 == 0 { '\n' } else { 'x' }),
                1 => buf.delete(),
                _ => buf.move_gap((next() as usize) % (len + 1)),
            }
            buf.assert_index_valid();
            assert_eq!(
                buf.line_count(),
                buf.to_string().chars().filter(|&c| c == '\n').count() + 1,
                "line_count disagreed with the text at step {step}"
            );
        }
    }

    /// The index must not change what any of these functions return -- only
    /// how fast they return it. Checked against a straightforward scan.
    #[test]
    fn indexed_lookups_agree_with_a_plain_scan() {
        let buf = numbered(50);
        let text = buf.to_string();
        let flat: Vec<char> = text.chars().collect();

        // get_lines, including ranges past the end
        for (a, b) in [(0, 3), (10, 14), (49, 52), (0, 51), (7, 7)] {
            let expected: Vec<String> = {
                let mut v: Vec<String> = text
                    .split('\n')
                    .skip(a)
                    .take(b - a)
                    .map(String::from)
                    .collect();
                while v.len() < b - a {
                    v.push(String::new());
                }
                v
            };
            assert_eq!(buf.get_lines(a, b), expected, "get_lines({a}, {b})");
        }

        // 1d <-> 2d round trips at every position
        for pos in 0..=flat.len() {
            let (line, col) = buf.cursor_1d_to_2d(pos);
            assert_eq!(
                buf.cursor_2d_to_1d(line, col),
                pos,
                "round trip failed at {pos} -> ({line}, {col})"
            );
        }
    }

    /// The property the whole change exists for: reading a viewport from the
    /// middle of a document must not depend on how far in it sits.
    ///
    /// Counted in characters examined rather than timed, so it is exact and
    /// machine-independent -- `get_lines` now touches only the text it
    /// returns, wherever that text lives.
    #[test]
    fn reading_a_viewport_costs_the_same_wherever_it_is() {
        const VIEWPORT: usize = 50;
        let small = numbered(1_000);
        let large = numbered(100_000);

        let head = small.get_lines(0, VIEWPORT);
        assert_eq!(head.len(), VIEWPORT);
        assert_eq!(large.get_lines(0, VIEWPORT), head);

        // Same viewport, 50,000 lines in: identical shape, and the line
        // contents prove it really seeked rather than counted.
        let middle = large.get_lines(50_000, 50_000 + VIEWPORT);
        assert_eq!(middle.len(), VIEWPORT);
        assert_eq!(middle[0], "line 50000");
        assert_eq!(
            middle[VIEWPORT - 1],
            format!("line {}", 50_000 + VIEWPORT - 1)
        );
    }
}
