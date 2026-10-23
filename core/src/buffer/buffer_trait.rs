//! [`BufferTrait`]: what the rest of the editor may assume about a buffer's
//! text.
//!
//! Design: `doc/architettura-editor.typ, <gap-buffer>`.

/// A buffer's text: characters by position, and a cursor where edits happen.
///
/// A position counts characters, not bytes, from 0 to [`Self::len`]. A line
/// ends at a newline, and a text that ends in one has an empty last line.
/// [`GapBuffer`](crate::GapBuffer) is the implementation the editor uses;
/// `buffer_conformance_tests` checks what every implementation has to do.
///
/// # Examples
///
/// ```
/// use rsedit_core::{BufferTrait, GapBuffer};
///
/// let mut text = GapBuffer::from("one\ntwo");
/// assert_eq!(text.line_count(), 2);
/// text.cursor_move(1, 0);
/// text.insert('>');
/// assert_eq!(text.to_string(), "one\n>two");
/// ```
pub trait BufferTrait:
    Clone + Default + ToString + Send + Sync + 'static + for<'input> From<&'input str>
{
    /// Returns how many characters the text holds.
    fn len(&self) -> usize;
    /// Returns whether the text is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns every character from POS onwards, in order: nothing when POS
    /// is at or past the end.
    ///
    /// What the scanner, the highlighter and search walk the text with: a
    /// stream lets an implementation walk its own storage once, rather than be
    /// asked for each character.
    fn chars_from(&self, pos: usize) -> impl Iterator<Item = char> + '_ {
        (pos..).map_while(move |i| self.at(i))
    }

    /// Returns the characters in `[from, to)`, as a string.
    ///
    /// A stale offset gives a short answer rather than an error: `to` past the
    /// end stops at the end, and `from` at or past `to` gives nothing.
    fn slice(&self, from: usize, to: usize) -> String {
        let end = to.min(self.len());
        if from >= end {
            return String::new();
        }
        self.chars_from(from).take(end - from).collect()
    }

    /// Returns the character at POS, or `None` at or past the end.
    fn at(&self, pos: usize) -> Option<char>;
    /// Returns the cursor's line and column.
    fn cursor_pos(&self) -> (usize, usize);
    /// Returns the cursor's position.
    fn cursor_pos_1d(&self) -> usize;
    /// Returns the line and column of POS.
    fn cursor_1d_to_2d(&self, pos: usize) -> (usize, usize);
    /// Returns the position at LINE and COL. A column past the end of its line
    /// stops at the line's end, and a line past the last at the end of the
    /// text.
    fn cursor_2d_to_1d(&self, line: usize, col: usize) -> usize;
    /// Moves the cursor one character forward. False at the end of the text.
    fn cursor_move_forward(&mut self) -> bool;
    /// Moves the cursor one character back. False at the start of the text.
    fn cursor_move_backward(&mut self) -> bool;
    /// Moves the cursor to LINE and COL, clamped as
    /// [`Self::cursor_2d_to_1d`] clamps them.
    fn cursor_move(&mut self, line: usize, col: usize);
    /// Returns the position of the nearest C before the cursor.
    fn find_backward(&mut self, c: char) -> Option<usize>;
    /// Returns the position of the nearest C after the character at the
    /// cursor.
    fn find_forward(&mut self, c: char) -> Option<usize>;
    /// Inserts C at the cursor, and leaves the cursor after it.
    fn insert(&mut self, c: char);
    /// Deletes the character before the cursor, if there is one.
    fn delete(&mut self);
    /// Returns how many lines the text has: one more than its newlines.
    fn line_count(&self) -> usize;
    /// Returns the lines from START_LINE up to but not including END_LINE,
    /// without their newlines. A line past the end of the text comes back
    /// empty, so there are always `end_line - start_line` of them.
    fn get_lines(&self, start_line: usize, end_line: usize) -> Vec<String>;
    /// Removes all the text, leaving the cursor at the start: in one step,
    /// not a character at a time.
    fn clear(&mut self);
}
