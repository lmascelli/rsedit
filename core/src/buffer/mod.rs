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
    /// Whether this buffer refuses to have its text changed.
    ///
    /// # Why the flag lives here and is checked at the doors
    ///
    /// A buffer that *presents* something -- a directory listing, a listing of
    /// faces, a backtrace -- is a view of state that lives elsewhere. Typing
    /// into it cannot change that state, so what typing actually does is make
    /// the screen disagree with the world while looking as though it worked.
    ///
    /// The protection cannot be keys alone. Every printable key is bound to
    /// `self-insert` globally, so a mode would have to shadow all of them to
    /// be safe, and `M-x kill-line` or a line of Lisp would still walk in. It
    /// belongs where undo and syntax invalidation already are: at
    /// `edits::insert_text` and `edits::delete_range`, which nothing that
    /// changes text can go around.
    ///
    /// Point still moves freely. Reading a read-only buffer is the entire
    /// point of having one.
    pub read_only: bool,
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
            read_only: false,
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
            read_only: false,
        }
    }
}
