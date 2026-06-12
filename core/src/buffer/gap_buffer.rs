use crate::buffer::BufferTrait;

pub struct GapBuffer {
    data: Vec<char>,
    gap_start: usize,
    gap_end: usize,
}

impl Default for GapBuffer {
    fn default() -> Self {
        Self {
            data: vec![char::default(); GapBuffer::GAPBUFFER_BASE_LEN],
            gap_start: 0,
            gap_end: GapBuffer::GAPBUFFER_BASE_LEN,
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
        if pos >= self.len() {
            None
        } else if pos <= self.gap_start {
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
        let mut line = 0;
        let mut col = 0;
        let logical_text = self.data[0..self.gap_start]
            .iter()
            .chain(self.data[self.gap_end..].iter())
            .take(pos);

        for &c in logical_text {
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }

        (line, col)
    }

    fn cursor_2d_to_1d(&self, line: usize, col: usize) -> usize {
        let mut pos = 0;
        let mut line_count = 0;
        let mut col_count = 0;

        let logical_text = self.data[0..self.gap_start]
            .iter()
            .chain(self.data[self.gap_end..].iter());

        for &c in logical_text {
            if col_count == col && line_count == line {
                break;
            }
            if c == '\n' {
                if line_count == line {
                    break;
                }
                line_count += 1;
                col_count = 0;
            } else {
                col_count += 1;
            }
            pos += 1;
        }

        pos
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
        let mut current_line = 0;
        let mut current_col = 0;
        let mut new_pos = 0;

        let before_gap = self.data[..self.gap_start].iter();
        let after_gap = self.data[self.gap_end..].iter();
        let logical_text = before_gap.chain(after_gap);

        for &c in logical_text {
            // reached the goal cursor position
            if current_line == line && current_col == col {
                break;
            }
            // the goal line has no the goal col
            if current_line == line && c == '\n' {
                break;
            }

            new_pos += 1;

            if c == '\n' {
                current_line += 1;
                current_col = 0;
                if current_line > line {
                    unreachable!();
                    // new_pos -= 1;
                    // break;
                }
            } else {
                current_col += 1;
            }
        }

        self.move_gap(new_pos);
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
        self.gap_start += 1;
    }

    fn delete(&mut self) {
        if self.gap_start > 0 {
            self.gap_start -= 1;
        }
    }
}

impl GapBuffer {
    const GAPBUFFER_BASE_LEN: usize = 1024;

    pub fn move_gap(&mut self, new_cursor_pos: usize) {
        if new_cursor_pos > self.data.len() - self.gap_end + self.gap_start {
            return;
        }
        if new_cursor_pos < self.gap_start {
            let range = self.gap_start - new_cursor_pos;
            self.data
                .copy_within(new_cursor_pos..self.gap_start, self.gap_end - range);
            self.gap_start -= range;
            self.gap_end -= range;
        } else if new_cursor_pos > self.gap_start {
            let range = new_cursor_pos - self.gap_start;
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
}
