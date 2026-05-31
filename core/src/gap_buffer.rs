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

impl BufferTrait for GapBuffer {
    fn from_text(text: &str) -> Self {
        let mut ret = Self::default();
        for c in text.chars() {
            ret.insert(c);
        }
        ret.move_gap(0);
        ret
    }

    fn cursor_pos(&self) -> (usize, usize) {
        self.cursor_line_col()
    }

    fn cursor_move(&mut self, row: usize, col: usize) {
        self.move_to_line_col(row, col);
    }

    fn insert(&mut self, c: char) {
        self.insert(c)
    }

    fn delete_char(&mut self) {
        self.delete()
    }
}

impl GapBuffer {
    const GAPBUFFER_BASE_LEN: usize = 1024;

    pub fn new() -> Self {
        Default::default()
    }

    pub fn cursor_pos(&self) -> usize {
        self.gap_start
    }

    pub fn cursor_line_col(&self) -> (usize, usize) {
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

    pub fn move_to_line_col(&mut self, target_line: usize, target_col: usize) {
        let mut current_line = 0;
        let mut current_col = 0;
        let mut new_pos = 0;

        let before_gap = self.data[..self.gap_start].iter();
        let after_gap = self.data[self.gap_end..].iter();
        let logical_text = before_gap.chain(after_gap);

        for &c in logical_text {
            // reached the goal cursor position
            if current_line == target_line && current_col == target_col {
                break;
            }
            // the goal line has no the goal col
            if current_line == target_line && c == '\n' {
                break;
            }

            new_pos += 1;

            if c == '\n' {
                current_line += 1;
                current_col = 0;
                if current_line > target_line {
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

    pub fn insert(&mut self, c: char) {
        if self.gap_start == self.gap_end {
            self.gap_grow();
        }
        self.data[self.gap_start] = c;
        self.gap_start += 1;
    }

    pub fn delete(&mut self) {
        if self.gap_start > 0 {
            self.gap_start -= 1;
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
        // let buffer_data = core::mem::replace(&mut self.data, Vec::new());
        // let mut new_data = vec![char::default(); buffer_data.len() * 2];
        // new_data[0..self.gap_start].copy_from_slice(&buffer_data[0..self.gap_start]);
        // new_data[self.gap_end..self.data.len()].copy_from_slice(&buffer_data[self.gap_end..self.data.len()]);
        // self.gap_end +=
        // _ = core::mem::replace(&mut self.data, new_data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialization() {
        let buf = GapBuffer::new();
        // Ensure the string is empty but the gap exists
        assert_eq!(buf.to_string(), "");
        assert!(buf.gap_end > buf.gap_start);
    }

    #[test]
    fn test_basic_typing_and_unicode() {
        let mut buf = GapBuffer::new();
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
        let mut buf = GapBuffer::new();
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
        let mut buf = GapBuffer::new();
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
        let mut buf = GapBuffer::new();
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
        let mut buf = GapBuffer::new();
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
        let mut buf = GapBuffer::new();
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
}
