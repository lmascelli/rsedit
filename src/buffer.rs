pub struct GapBuffer {
    data: Vec<char>,
    gap_start: usize,
    gap_end: usize,
}

impl GapBuffer {
    const GAPBUFFER_BASE_LEN: usize = 1024;
    
    pub fn new() -> Self {
        Self {
            data: vec![char::default(); GapBuffer::GAPBUFFER_BASE_LEN],
            gap_start: 0,
            gap_end: 1024,
        }
    }

    pub fn move_gap(&mut self, new_cursor_pos: usize) {
        if new_cursor_pos > self.data.len() - self.gap_end + self.gap_start { return; }
        if new_cursor_pos < self.gap_start {
            let range = self.gap_start - new_cursor_pos;
            self.data.copy_within(
                new_cursor_pos..self.gap_start,
                self.gap_end - range,
            );
            self.gap_start -= range;
            self.gap_end -= range;
        } else if new_cursor_pos > self.gap_start {
            let range = new_cursor_pos - self.gap_start;
            self.data.copy_within(
                self.gap_end..self.gap_end + range,
                self.gap_start
            );
            self.gap_start += range;
            self.gap_end += range;
        }
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
        new_data[self.gap_end + old_data_len..new_data_len].copy_from_slice(&self.data[self.gap_end..self.data.len()]);
        _ = core::mem::replace(&mut self.data, new_data);
        self.gap_end += old_data_len;
        // let buffer_data = core::mem::replace(&mut self.data, Vec::new());
        // let mut new_data = vec![char::default(); buffer_data.len() * 2];
        // new_data[0..self.gap_start].copy_from_slice(&buffer_data[0..self.gap_start]);
        // new_data[self.gap_end..self.data.len()].copy_from_slice(&buffer_data[self.gap_end..self.data.len()]);
        // self.gap_end +=  
        // _ = core::mem::replace(&mut self.data, new_data);
    }

    pub fn to_string(&self) -> String {
        let ret = self.data[0..self.gap_start].iter()
            .chain(self.data[self.gap_end..self.data.len()].iter())
            .collect::<String>();
        ret
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
        buf.delete();    // Deletes 'o'
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
