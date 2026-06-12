pub trait BufferTrait: Default + ToString + for<'input> From<&'input str> {
    fn len(&self) -> usize;
    fn at_line_col(&self, line: usize, col: usize) -> Option<char>;
    fn at(&self, pos: usize) -> Option<char>;
    fn cursor_pos(&self) -> (usize, usize);
    fn cursor_pos_1d(&self) -> usize;
    fn cursor_1d_to_2d(&self, pos: usize) -> (usize, usize);
    fn cursor_2d_to_1d(&self, line: usize, col: usize) -> usize;
    fn cursor_move_forward(&mut self) -> bool;
    fn cursor_move_backward(&mut self) -> bool;
    fn cursor_move(&mut self, line: usize, col: usize);
    fn find_backward(&mut self, c: char) -> Option<usize>;
    fn find_forward(&mut self, c: char) -> Option<usize>;
    fn insert(&mut self, c: char);
    fn delete(&mut self);
}
