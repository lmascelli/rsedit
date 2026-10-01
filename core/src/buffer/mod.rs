// implementors
mod buffer_trait;
pub use buffer_trait::BufferTrait;
pub mod disk;
pub mod gap_buffer;
pub mod mark;
pub mod overlay;
pub mod virtual_text;
pub mod scan;
pub mod syntax;
pub use disk::{FileStamp, OnDisk};
pub use mark::Mark;
pub use overlay::{Overlay, OverlayTable};
pub use virtual_text::{VirtualText, VirtualTextTable};
pub use scan::ScanCache;
pub mod undo;
pub use undo::UndoHistory;

use crate::input::KeyEvent;
use std::any::Any;
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
    /// Faces put on this buffer's text by something other than its mode.
    ///
    /// A search marking its matches, a diagnostic from elsewhere, a manual
    /// page whose emphasis was in the bytes -- none of those can be said as a
    /// syntax rule, which is a pattern over the text rather than a statement
    /// about one particular span of it. See [`crate::buffer::overlay`].
    ///
    /// Unlike everything else here that knows a position, these survive edits:
    /// the two doors adjust them. That is the whole of what makes them
    /// interesting, and the reason they are a table rather than a `Vec`.
    pub overlays: OverlayTable,

    /// Text shown in a window at positions in this buffer, without being in
    /// it: an inlay hint, a preview of what a command would insert. See
    /// [`crate::buffer::virtual_text`], and [`crate::ui::layout`] for how a
    /// row that is longer on screen than it is in the buffer is drawn and
    /// clicked on.
    ///
    /// Beside the overlays rather than part of them because it is anchored at
    /// a point, and an overlay with no width is one the overlay table deletes.
    pub virtual_text: VirtualTextTable,

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
    /// What has been worked out about reading this buffer as balanced
    /// expressions. See [`crate::buffer::scan::ScanCache`].
    ///
    /// Beside the colouring cache and not part of it: the grammar and the
    /// syntax table are two different lexers answering two different questions,
    /// and this is the one `forward-sexp`, the indenter and `syntax-ppss` read.
    pub scan: scan::ScanCache,
    /// What has been worked out about colouring this buffer. See
    /// [`crate::buffer::syntax::SyntaxCache`].
    pub syntax: syntax::SyntaxCache,
    /// What the file this buffer is visiting looked like when it was last
    /// read or written *by this editor*.
    ///
    /// `None` for a buffer visiting no file, and for one whose file was not
    /// there when it was opened. See [`crate::buffer::disk`] for why it is a
    /// stamp rather than a hash.
    ///
    /// # Why writing has to record it too
    ///
    /// Because saving changes the file, and a stamp taken only at read time
    /// would then differ from what is on disk the instant the buffer is
    /// saved -- so every save would look like somebody else's write, and the
    /// editor would report a conflict with itself.
    pub file_stamp: Option<FileStamp>,
    /// Whether the file has been seen to differ from [`Self::file_stamp`] in a
    /// way that could not be taken silently.
    ///
    /// Set by the worker that watches open files when it finds a change it
    /// must not act on -- because the buffer has unsaved edits, or because the
    /// file has gone. Cleared by reading the file again, by saving over it, or
    /// by being told to stop worrying.
    ///
    /// It exists so that the *next* save can ask. A buffer silently written
    /// over somebody else's work is the loss this whole mechanism is for, and
    /// the moment to catch it is the save, not the instant the change is
    /// noticed -- which is why this is a flag rather than a prompt.
    pub stale: bool,
    /// The version whose contents were last written to this buffer's
    /// auto-save file, or `None` when there is no such file.
    ///
    /// # Why a version and not a flag
    ///
    /// Because "modified" stays true from the first keystroke until the save,
    /// so a job that wrote every modified buffer every turn would rewrite an
    /// untouched one forever. The version moves only when the text does, which
    /// turns "is there anything to write" into an integer comparison -- and
    /// makes an editor left open overnight cost nothing.
    pub auto_saved_at: Option<u64>,
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
    /// Things other parts of the editor have hung on this buffer, by name.
    ///
    /// # What this is for
    ///
    /// Some features need somewhere to keep what they worked out *about one
    /// buffer*: a list of search results shown in it, a parsed table, the
    /// state of a view. Until now there were two places to put that, and each
    /// is wrong in its own way -- a global keyed by buffer name, which goes
    /// stale the moment the buffer is renamed and is still there after it is
    /// killed; or the buffer's own text, which works only when the value is
    /// something you would have displayed anyway.
    ///
    /// This is the third place, and it has the property the other two lack:
    /// **the value lives exactly as long as the buffer does**. Killing a
    /// buffer drops it from the table it lives in, which drops this map,
    /// which runs each value's own destructor. Nothing has to remember to
    /// clean up, because there is nothing separate to clean.
    ///
    /// # Why `dyn Any` and not a field per feature
    ///
    /// So that a feature can attach whatever it likes without `Buffer`
    /// growing a field for it, and while knowing nothing about the other
    /// features that do. The alternative considered was a raw pointer cast
    /// back on demand, and it would have defeated the purpose: a raw pointer
    /// has no destructor, so dropping the buffer would free the *pointer* and
    /// leak everything behind it, and a cast with no type tag is undefined
    /// behaviour rather than a refusal when it is wrong. `Box<dyn Any>`
    /// carries the concrete type's `Drop`, and `downcast_ref` checks.
    ///
    /// Keyed by name, the way overlays are keyed by category: two features
    /// attach to one buffer without knowing about each other, and a feature
    /// replaces its own entry by putting to the same name.
    ///
    /// Not cleared by [`Buffer::adopt_text`], unlike the undo history and the
    /// overlays. Those describe positions in text that no longer exists; an
    /// attachment describes whatever its owner says it does, and only its
    /// owner knows whether a reverted file invalidates it.
    pub data: HashMap<String, Box<dyn Any + Send + Sync>>,
}

impl<B: BufferTrait> Buffer<B> {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            text: B::default(),
            file_path: None,
            is_modified: false,
            local_keymap: None,
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
        }
    }

    // ------------------------------------------------------------------
    // Things hung on this buffer
    // ------------------------------------------------------------------

    /// Attach VALUE under KEY, replacing whatever was there.
    ///
    /// Returns nothing: a caller that wanted the old value can ask for it
    /// first, and handing back a `Box<dyn Any>` that nobody downcasts would
    /// be a value quietly going nowhere.
    pub fn put_data<T: Any + Send + Sync>(&mut self, key: &str, value: T) {
        self.data.insert(key.to_string(), Box::new(value));
    }

    /// What is attached under KEY, if it is a `T`.
    ///
    /// `None` both when nothing is there and when what is there is something
    /// else. The two are not worth telling apart: a caller asking for its own
    /// key with its own type is asking "is mine there", and a caller asking
    /// about somebody else's has no business with either answer.
    pub fn data<T: Any + Send + Sync>(&self, key: &str) -> Option<&T> {
        self.data.get(key)?.downcast_ref::<T>()
    }

    /// The same, to be changed in place.
    pub fn data_mut<T: Any + Send + Sync>(&mut self, key: &str) -> Option<&mut T> {
        self.data.get_mut(key)?.downcast_mut::<T>()
    }

    /// Remove whatever is attached under KEY. True when there was something.
    pub fn forget_data(&mut self, key: &str) -> bool {
        self.data.remove(key).is_some()
    }

    /// Where a scan of this buffer that will be asked about POS may carry on
    /// from, or nothing when it has to start at the top.
    ///
    /// Here rather than at each call site because the three things that decide
    /// it -- the version, the mode and the line POS is on -- all live on the
    /// buffer, and a caller that read two of them and forgot the third would
    /// get a plausible answer computed under the wrong table.
    pub fn scan_resume(&self, pos: usize) -> Option<&crate::modes::sexp::Resume> {
        let line = self.text.cursor_1d_to_2d(pos.min(self.text.len())).0;
        self.scan.resume_for(&self.current_mode, self.version, line)
    }

    /// Replace everything in this buffer with TEXT, as though it had just been
    /// read from the file.
    ///
    /// # Why this is one method and not six statements at the call site
    ///
    /// Because six statements is six chances to forget one, and the ones that
    /// are easy to forget are the ones whose absence is invisible until much
    /// later. A revert that did not bump the version would leave the
    /// highlighter storing colours computed from the *old* text at offsets in
    /// the new one; a revert that did not clear the overlays would leave a
    /// diagnostic pinned to a line that has moved; a revert that did not clamp
    /// point could leave it past the end of a file that got shorter.
    ///
    /// # What it deliberately throws away
    ///
    /// The undo history, the mark, the overlays and the virtual text. All of
    /// them describe
    /// *positions in text that no longer exists*, and keeping them would mean
    /// keeping a promise the buffer can no longer honour -- undoing back into
    /// a state the file never had is worse than not being able to undo.
    ///
    /// The mode is kept, because the file is still the same file. So is the
    /// undo history's configured limit, which is a setting rather than
    /// history.
    pub fn adopt_text(&mut self, text: &str) {
        self.text = B::from(text);
        self.version = self.version.wrapping_add(1);
        // From line zero: every line is new.
        self.syntax.invalidate_from(self.version, 0);
        self.scan.invalidate_from(self.version, 0);
        self.overlays = OverlayTable::default();
        // For the same reason as the overlays, and more plainly: a hint says
        // what a position in the old text meant.
        self.virtual_text = VirtualTextTable::default();
        self.mark = None;
        let limit = self.undo.limit();
        self.undo = UndoHistory::default();
        self.undo.set_limit(limit);
        // Clamped rather than reset: coming back to roughly where you were is
        // the point of reverting a file you are reading, and the top of the
        // buffer is where you were not.
        let point = self.text.cursor_pos_1d().min(self.text.len());
        let (line, column) = self.text.cursor_1d_to_2d(point);
        self.text.cursor_move(line, column);
        self.is_modified = false;
        self.stale = false;
    }

    pub fn from_text(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            text: B::from(text),
            file_path: None,
            is_modified: false,
            local_keymap: None,
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
        }
    }
}
