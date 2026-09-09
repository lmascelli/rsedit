// implementors
mod buffer_trait;
pub use buffer_trait::BufferTrait;
pub mod gap_buffer;
pub mod mark;
pub use mark::Mark;
pub mod undo;
pub use undo::UndoHistory;

use crate::input::KeyEvent;
use std::collections::HashMap;

pub struct Buffer<B: BufferTrait> {
    pub name: String,
    pub text: B,
    pub file_path: Option<String>,
    pub is_modified: bool,
    pub local_keymap: Option<HashMap<KeyEvent, String>>,
    pub current_mode: String,
    /// This buffer's edit history: recorded changes for undo and redo.
    pub undo: UndoHistory,
    /// Where the mark is, when this buffer has one. The region is the text
    /// between it and point -- see [`crate::buffer::mark::region_bounds`].
    pub mark: Option<Mark>,
}

impl<B: BufferTrait> Buffer<B> {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            text: B::default(),
            file_path: None,
            is_modified: false,
            local_keymap: None,
            current_mode: "fundamental".into(),
            undo: UndoHistory::default(),
            mark: None,
        }
    }

    pub fn from_text(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            text: B::from(text),
            file_path: None,
            is_modified: false,
            local_keymap: None,
            current_mode: "fundamental".into(),
            undo: UndoHistory::default(),
            mark: None,
        }
    }
}
