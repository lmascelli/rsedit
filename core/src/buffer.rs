pub trait BufferTrait: Default + ToString {
    fn from_text(text: &str) -> Self;
    fn cursor_pos(&self) -> (usize, usize);
    fn cursor_move(&mut self, row: usize, col: usize);
    fn insert(&mut self, c: char);
    fn delete_char(&mut self);
}
