//! Every buffer the editor holds, which one is current, and which were current
//! lately: one fact in three pieces, kept so that `current` can only ever name
//! a buffer the table holds. A buffer is reached through a closure, with the
//! table's lock already given back.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
use crate::buffer::{Buffer, BufferTrait};
use crate::work::Topic;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// The buffer table, the current buffer's name, and the recency list.
pub struct Buffers<B: BufferTrait> {
    table: HashMap<String, Arc<RwLock<Buffer<B>>>>,
    /// Shared rather than owned: every buffer access starts by reading it,
    /// several times a keystroke, and sharing it costs no copy.
    current: Arc<str>,
    /// Most recently current first. It may name buffers that are gone: its
    /// one reader, [`Buffers::most_recent`], skips them.
    recency: Vec<String>,
    /// What every buffer in the table emits when its text changes: in the
    /// editor, its `buffer-changed`. Handed to each buffer as it comes in, so
    /// that no buffer can be in the table and change without saying so,
    /// whoever made it.
    changed: Topic,
}

/// The name of the buffer that exists before anything else does.
pub const SCRATCH: &str = "*scratch*";

/// A table whose buffers emit a topic nobody listens to: for what needs a
/// table but no editor.
impl<B: BufferTrait> Default for Buffers<B> {
    fn default() -> Self {
        Self::new(Topic::detached())
    }
}

impl<B: BufferTrait> Buffers<B> {
    /// Creates a table holding `*scratch*`, whose buffers emit CHANGED.
    pub fn new(changed: Topic) -> Self {
        // Never empty: every path here assumes there is a current buffer.
        let mut buffers = Self {
            table: HashMap::new(),
            current: Arc::from(SCRATCH),
            recency: vec![SCRATCH.to_string()],
            changed,
        };
        buffers.insert(SCRATCH, Buffer::new(SCRATCH));
        buffers
    }

    // ------------------------------------------------------------------
    // Handles, for the accessors on the facade
    // ------------------------------------------------------------------

    /// Returns the `Arc` for NAME, cloned out so that the caller can give the
    /// table back before locking the buffer.
    ///
    /// Only `EditorState`'s `with_buffer` family uses it, and each hands the
    /// buffer's guard to a closure.
    pub(crate) fn handle(&self, name: &str) -> Option<Arc<RwLock<Buffer<B>>>> {
        self.table.get(name).cloned()
    }

    /// Returns the `Arc` for the current buffer. Infallible: `current` only
    /// ever names a buffer the table holds.
    pub(crate) fn current_handle(&self) -> Arc<RwLock<Buffer<B>>> {
        self.table
            .get(&*self.current)
            .expect("Corruption in the hashmap of buffers")
            .clone()
    }

    // ------------------------------------------------------------------
    // Asking
    // ------------------------------------------------------------------

    /// Returns the current buffer's name, without copying it.
    pub fn current_name(&self) -> Arc<str> {
        self.current.clone()
    }

    /// Returns whether a buffer is called NAME.
    pub fn contains(&self, name: &str) -> bool {
        self.table.contains_key(name)
    }

    /// Returns how many buffers there are.
    pub fn count(&self) -> usize {
        self.table.len()
    }

    /// Returns every buffer's name, sorted, for completion.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.table.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    /// Returns every buffer's name, FIRST at the head if it is one of them
    /// and the rest in no particular order: the background jobs reach what is
    /// on screen before what is not.
    pub fn names_with_first(&self, first: Option<&str>) -> Vec<String> {
        let first = first.filter(|name| self.table.contains_key(*name));
        first
            .iter()
            .map(|name| name.to_string())
            .chain(
                self.table
                    .keys()
                    .filter(|name| Some(name.as_str()) != first)
                    .cloned(),
            )
            .collect()
    }

    /// Returns the most recently current buffer that still exists and is not
    /// EXCEPT, or `None` when there is none: falling back to `*scratch*` is
    /// the caller's choice, not this table's.
    pub fn most_recent(&self, except: &str) -> Option<String> {
        self.recency
            .iter()
            .find(|name| name.as_str() != except && self.table.contains_key(*name))
            .cloned()
    }

    // ------------------------------------------------------------------
    // Changing
    // ------------------------------------------------------------------

    /// Adds BUFFER under NAME, replacing any buffer already there.
    ///
    /// The buffer is handed this table's topic, which is then emitted: a
    /// buffer that has just come in is text nothing has looked at yet.
    pub fn insert(&mut self, name: &str, mut buffer: Buffer<B>) {
        buffer.changed = self.changed.clone();
        self.table
            .insert(name.to_string(), Arc::new(RwLock::new(buffer)));
        self.changed.emit();
    }

    /// Makes NAME current and returns what was current before; `None`,
    /// changing nothing, when there is no such buffer.
    pub fn make_current(&mut self, name: &str) -> Option<Arc<str>> {
        let previous = self.bind_current(name)?;
        self.record_use(name);
        Some(previous)
    }

    /// Makes NAME current without recording a visit, and returns what was
    /// current before, or `None` as [`Buffers::make_current`] does.
    ///
    /// What `set-buffer` does: Lisp acting on a buffer it is not showing, which
    /// must not reorder the recency list.
    pub fn bind_current(&mut self, name: &str) -> Option<Arc<str>> {
        if !self.table.contains_key(name) {
            return None;
        }
        let previous = self.current.clone();
        self.current = Arc::from(name);
        Some(previous)
    }

    /// Moves NAME to the front of the recency list.
    fn record_use(&mut self, name: &str) {
        if self.recency.first().is_some_and(|first| first == name) {
            // The common case by far: the list stays as it is.
            return;
        }
        self.recency.retain(|existing| existing != name);
        self.recency.insert(0, name.to_string());
    }

    /// Renames OLD to NEW, and says whether the rest of the editor has work to
    /// do about it.
    ///
    /// Refused when there is no OLD, and when NEW belongs to another buffer: a
    /// table keyed by name cannot hold two, and finding a free name is the
    /// caller's job (`buffer_name_for` makes `name<2>`). Renaming a buffer to
    /// the name it has succeeds and changes nothing.
    ///
    /// The buffer's own `name`, and the windows showing OLD, are the caller's
    /// to update: this does not lock the buffer while it holds the table, and
    /// does not reach the windows.
    pub fn rename(&mut self, old: &str, new: &str) -> BufferRenamed {
        if old == new {
            return BufferRenamed::Yes;
        }
        if self.table.contains_key(new) {
            return BufferRenamed::No;
        }
        let Some(handle) = self.table.remove(old) else {
            return BufferRenamed::No;
        };
        self.table.insert(new.to_string(), handle);
        for remembered in self.recency.iter_mut() {
            if remembered == old {
                *remembered = new.to_string();
            }
        }
        // Here rather than left to the caller: `current` must not name a
        // missing buffer for even one statement.
        if &*self.current == old {
            self.current = Arc::from(new);
        }
        BufferRenamed::Yes
    }

    /// Removes NAME, and says what the caller must now do about what is
    /// current.
    ///
    /// A fresh `*scratch*` is made when that empties the table. When NAME was
    /// current, the most recent survivor is current now, or `*scratch*`; the
    /// answer says so, because a window to repoint and a mode line to redraw
    /// are the caller's.
    pub fn remove(&mut self, name: &str) -> BufferRemoved {
        if self.table.remove(name).is_none() {
            return BufferRemoved::No;
        }
        self.recency.retain(|existing| existing != name);
        if self.table.is_empty() {
            self.insert(SCRATCH, Buffer::new(SCRATCH));
        }
        if &*self.current != name {
            return BufferRemoved::Yes;
        }
        // The most recent survivor, or `*scratch*`, which the table holds by
        // the branch above.
        let successor = self
            .most_recent(name)
            .unwrap_or_else(|| SCRATCH.to_string());
        self.current = Arc::from(successor.as_str());
        self.record_use(&successor);
        BufferRemoved::Current(successor)
    }
}

/// What [`Buffers::rename`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferRenamed {
    /// No such buffer, or the new name belongs to a different one. Nothing
    /// changed.
    No,
    /// Renamed. Every window showing the old name must now be repointed.
    Yes,
}

/// What [`Buffers::remove`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BufferRemoved {
    /// No such buffer.
    No,
    /// Gone, and it was not the current one.
    Yes,
    /// Gone, and it *was* current. The named buffer is current now; telling
    /// the rest of the editor is the caller's half of that.
    Current(String),
}
