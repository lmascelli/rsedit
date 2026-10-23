//! A buffer: its text, behind [`BufferTrait`], and everything kept about it
//! -- the file, the mode, the undo history, the mark, overlays and virtual
//! text, the caches. The text is changed through the two doors in
//! `primitives::edits`, which keep the rest in step with it.
//!
//! Design: `doc/architettura-editor.typ, <buffer>`.
mod buffer_trait;
pub mod disk;
pub mod gap_buffer;
pub mod mark;
pub mod overlay;
pub mod scan;
pub mod syntax;
pub mod undo;
pub mod virtual_text;

pub use buffer_trait::BufferTrait;
pub use disk::{FileStamp, OnDisk};
pub use mark::Mark;
pub use overlay::OverlayTable;
pub use undo::UndoHistory;
pub use virtual_text::VirtualTextTable;

use crate::work::{Topic, WorkId};
use std::any::Any;
use std::collections::HashMap;

/// A buffer: its name and text, the file it visits, and everything worked out
/// about it or hung on it.
pub struct Buffer<B: BufferTrait> {
    pub name: String,
    pub text: B,
    pub file_path: Option<String>,
    pub is_modified: bool,
    pub current_mode: String,
    /// This buffer's edit history: recorded changes for undo and redo.
    pub undo: UndoHistory,
    /// Faces put on this buffer's text by something other than its mode: see
    /// [`crate::buffer::overlay`]. The two doors move them with the text.
    pub overlays: OverlayTable,

    /// Text shown in a window at positions in this buffer, without being in
    /// it: see [`crate::buffer::virtual_text`], and [`crate::ui::layout`] for
    /// how a window draws it and takes clicks on it.
    pub virtual_text: VirtualTextTable,

    /// Where the mark is, when this buffer has one. The region is the text
    /// between it and point -- see [`crate::buffer::mark::region_bounds`].
    pub mark: Option<Mark>,
    /// Bumped by every change to the text: at the two doors, and by undo and
    /// redo. Work done with the buffer's lock given back -- the highlighter's,
    /// the prescanner's -- stores its result only if the version has not
    /// moved since it read the text.
    pub version: u64,
    /// Emitted at every change to the text, beside the version: what wakes the
    /// work that has to look at it again. See [`crate::work::topic`].
    ///
    /// A topic of its own until the buffer joins an editor, which hands it the
    /// editor's `buffer-changed` -- see `managers::Buffers::insert`. Emitting
    /// is an atomic increment, cheap under the buffer's lock.
    pub changed: Topic,
    /// What has been worked out about reading this buffer as balanced
    /// expressions. See [`crate::buffer::scan::ScanCache`].
    pub scan: scan::ScanCache,
    /// What has been worked out about colouring this buffer. See
    /// [`crate::buffer::syntax::SyntaxCache`].
    pub syntax: syntax::SyntaxCache,
    /// What the file this buffer is visiting looked like when this editor last
    /// read or wrote it: see [`crate::buffer::disk`]. Recorded at a write too,
    /// or every save would look like somebody else's change.
    ///
    /// `None` for a buffer visiting no file, and for one whose file was not
    /// there when it was opened.
    pub file_stamp: Option<FileStamp>,
    /// Whether the file has been seen to differ from [`Self::file_stamp`] in a
    /// way that could not be taken silently: the buffer has unsaved edits, or
    /// the file has gone. The next save asks before writing over it.
    ///
    /// Set by the file watcher; cleared by reading the file again, by saving
    /// over it, or by being told to stop worrying.
    pub stale: bool,
    /// The version whose contents were last written to this buffer's
    /// auto-save file, or `None` when there is no such file. A version and
    /// not a flag, so that a buffer not edited since is not written again.
    pub auto_saved_at: Option<u64>,
    /// Whether this buffer refuses to have its text changed. Checked at the
    /// two doors; point still moves.
    pub read_only: bool,
    /// Things other parts of the editor have hung on this buffer, by name: a
    /// list of search results shown in it, a parsed table, the state of a
    /// view. A value lives exactly as long as the buffer, whose killing runs
    /// the value's own `Drop`.
    ///
    /// Not cleared by [`Buffer::adopt_text`], unlike the undo history and the
    /// overlays: an attachment describes whatever its owner says it does, and
    /// only its owner knows whether a reverted file invalidates it.
    pub data: HashMap<String, Box<dyn Any + Send + Sync>>,
    /// The work writing into this buffer from a thread of its own -- a shell
    /// command, a search -- for as long as it runs. Closing the buffer stops
    /// it.
    ///
    /// A number rather than a name, so that renaming the buffer does not lose
    /// track of it; once the work has finished, stopping the number does
    /// nothing, since a [`WorkId`] is never used twice.
    pub writer: Option<WorkId>,
}

impl<B: BufferTrait> Buffer<B> {
    /// Creates an empty buffer called NAME, in `fundamental-mode`.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            text: B::default(),
            file_path: None,
            is_modified: false,
            current_mode: "fundamental-mode".into(),
            undo: UndoHistory::default(),
            mark: None,
            overlays: OverlayTable::default(),
            virtual_text: VirtualTextTable::default(),
            version: 0,
            scan: scan::ScanCache::default(),
            syntax: syntax::SyntaxCache::default(),
            file_stamp: None,
            stale: false,
            auto_saved_at: None,
            read_only: false,
            data: HashMap::new(),
            changed: Topic::detached(),
            writer: None,
        }
    }

    // ------------------------------------------------------------------
    // Things hung on this buffer
    // ------------------------------------------------------------------

    /// Attaches VALUE under KEY, replacing whatever was there. A caller that
    /// wants the old value asks for it first.
    pub fn put_data<T: Any + Send + Sync>(&mut self, key: &str, value: T) {
        self.data.insert(key.to_string(), Box::new(value));
    }

    /// Returns what is attached under KEY, if it is a `T`: `None` both when
    /// nothing is there and when something else is.
    pub fn data<T: Any + Send + Sync>(&self, key: &str) -> Option<&T> {
        self.data.get(key)?.downcast_ref::<T>()
    }

    /// Returns what is attached under KEY, if it is a `T`, to be changed in
    /// place.
    pub fn data_mut<T: Any + Send + Sync>(&mut self, key: &str) -> Option<&mut T> {
        self.data.get_mut(key)?.downcast_mut::<T>()
    }

    /// Removes whatever is attached under KEY. True when there was something.
    pub fn forget_data(&mut self, key: &str) -> bool {
        self.data.remove(key).is_some()
    }

    /// Returns where a scan of this buffer with TABLE, to be asked about POS,
    /// may carry on from, or `None` when it has to start at the top.
    ///
    /// Here because three of the four things that decide it -- the version,
    /// the mode and the line POS is on -- live on the buffer. The fourth is
    /// TABLE: a checkpoint is a shortcut only for a scan with the table it was
    /// taken with.
    pub fn scan_resume(
        &self,
        pos: usize,
        table: &crate::modes::SyntaxTable,
    ) -> Option<&crate::modes::sexp::Resume> {
        let line = self.text.cursor_1d_to_2d(pos.min(self.text.len())).0;
        self.scan
            .resume_for(&self.current_mode, self.version, table.edition(), line)
    }

    /// Replaces everything in this buffer with TEXT, as though it had just
    /// been read from the file.
    ///
    /// The undo history, the mark, the overlays and the virtual text go: they
    /// describe positions in text that no longer exists. The mode stays, as
    /// does the undo history's limit, which is a setting; point keeps its line
    /// and column, clamped to the new text.
    pub fn adopt_text(&mut self, text: &str) {
        // Taken before the text is replaced: a fresh `B` starts with its cursor
        // at the top, so asking afterwards would always answer line zero.
        let (line, column) = self.text.cursor_pos();
        self.text = B::from(text);
        self.version = self.version.wrapping_add(1);
        // From line zero: every line is new.
        self.syntax.invalidate_from(self.version, 0);
        self.scan.invalidate_from(self.version, 0);
        self.overlays = OverlayTable::default();
        self.virtual_text = VirtualTextTable::default();
        self.mark = None;
        let limit = self.undo.limit();
        self.undo = UndoHistory::default();
        self.undo.set_limit(limit);
        // By line and column rather than by offset, so that text changed above
        // point does not move it onto another line. `cursor_move` clamps both:
        // a column past its line's end lands on that end, a line past the last
        // lands at the end of the text.
        self.text.cursor_move(line, column);
        self.is_modified = false;
        self.stale = false;
        self.changed.emit();
    }

    /// Creates a buffer called NAME holding TEXT, in `fundamental-mode`.
    pub fn from_text(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            text: B::from(text),
            file_path: None,
            is_modified: false,
            current_mode: "fundamental-mode".into(),
            undo: UndoHistory::default(),
            mark: None,
            overlays: OverlayTable::default(),
            virtual_text: VirtualTextTable::default(),
            version: 0,
            scan: scan::ScanCache::default(),
            syntax: syntax::SyntaxCache::default(),
            file_stamp: None,
            stale: false,
            auto_saved_at: None,
            read_only: false,
            data: HashMap::new(),
            changed: Topic::detached(),
            writer: None,
        }
    }
}
