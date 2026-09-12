// implementors
mod buffer_trait;
pub use buffer_trait::BufferTrait;
pub mod gap_buffer;
pub mod mark;
pub mod syntax;
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
    /// Bumped by every change to the text.
    ///
    /// Syntax highlighting is computed on another thread, and by the time a
    /// result comes back the text may have moved on. The version is what lets
    /// the result be refused: it describes a particular state of the buffer,
    /// and without the stamp it would be painted at offsets that no longer
    /// mean anything.
    ///
    /// Bumped in `edits::insert_text` and `edits::delete_range` -- the only two
    /// doors into the text, which is the same property undo depends on.
    pub version: u64,
    /// What has been worked out about colouring this buffer. See
    /// [`crate::buffer::syntax::SyntaxCache`].
    pub syntax: syntax::SyntaxCache,
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
            version: 0,
            syntax: syntax::SyntaxCache::default(),
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
            version: 0,
            syntax: syntax::SyntaxCache::default(),
        }
    }
}
