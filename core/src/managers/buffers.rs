//! Every buffer the editor holds, which one is current, and which were current
//! lately.
//!
//! # Why this is a compartment and not three fields
//!
//! The three are one fact in three pieces. `current` names a key in `table`,
//! and `recency` is a list of keys in it -- so two of the three are only
//! meaningful against the third. Held apart they can be observed disagreeing,
//! and the editor has already been taken down once by exactly that: a window
//! left naming a buffer that had been removed, focus moved there, `current`
//! became a name the table did not hold, and the next lookup hit its
//! "Corruption in the hashmap of buffers" panic with whatever was unsaved in
//! the other windows.
//!
//! Held as one, `current` cannot be set to a name the table does not hold --
//! not because everybody remembers to check, but because `make_current` is
//! the only way to set it, and it checks.
//!
//! # The handle rule
//!
//! A buffer is reached through a *closure*, never by being handed out. What
//! the accessor hands the closure is a locked buffer; what it never hands
//! anybody is a guard that outlives the question.
//!
//! The table's lock is **not** held while that closure runs. `handle` clones
//! the `Arc` out and the caller drops the table lock before locking the
//! buffer, so editing a buffer does not shut every other thread
//! out of the table -- which matters because the highlighter and the
//! prescanner are walking it on their own threads while somebody types.
//! Holding the table across a buffer's `write()` would make every keystroke
//! wait for a scan.
//!
//! # What is deliberately not here
//!
//! What a buffer *contains*. This compartment owns the table, the current
//! name and the recency list; the text, point, mark and modes belong to
//! [`Buffer`] itself. Nothing in this file reads a buffer's text.
//!
//! Windows, too. A buffer does not know what is showing it -- that is the
//! other direction of a relationship the `windows` compartment owns the near
//! end of, and keeping it one-way is what stops the two compartments needing
//! each other's locks.
use crate::buffer::{Buffer, BufferTrait};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

pub struct Buffers<B: BufferTrait> {
    table: HashMap<String, Arc<RwLock<Buffer<B>>>>,
    /// An `Arc<str>` rather than a `String` because every buffer access starts
    /// by reading this name, and reading a `String` out of a lock means
    /// copying it. Sharing it instead makes the current-buffer path allocation
    /// free, on a path that runs several times per keystroke.
    current: Arc<str>,
    /// Most recently current first. Only ever read to answer "what should this
    /// window show now that what it showed is gone", so it is allowed to name
    /// buffers that no longer exist -- the reader skips them.
    recency: Vec<String>,
}

/// The name of the buffer that exists before anything else does.
pub const SCRATCH: &str = "*scratch*";

impl<B: BufferTrait> Default for Buffers<B> {
    fn default() -> Self {
        // Never empty, and never empty from the first instant: an editor with
        // no buffers has no answer to "what is current", and every path here
        // is written on the assumption that there is one.
        let mut table = HashMap::new();
        table.insert(
            SCRATCH.to_string(),
            Arc::new(RwLock::new(Buffer::new(SCRATCH))),
        );
        Self {
            table,
            current: Arc::from(SCRATCH),
            recency: vec![SCRATCH.to_string()],
        }
    }
}

impl<B: BufferTrait> Buffers<B> {
    // ------------------------------------------------------------------
    // Handles, for the accessors on the facade
    // ------------------------------------------------------------------

    /// The `Arc` for NAME, cloned out so the caller can drop the table lock
    /// before taking the buffer's.
    ///
    /// Crate-private on purpose. It is the one hole in the closure discipline
    /// and it exists so that the table is not held across a buffer's lock;
    /// `EditorState`'s `with_buffer` family is the only thing that should use
    /// it, and each of those hands the guard to a closure and drops it.
    pub(crate) fn handle(&self, name: &str) -> Option<Arc<RwLock<Buffer<B>>>> {
        self.table.get(name).cloned()
    }

    /// The `Arc` for whatever is current.
    ///
    /// Infallible: `current` is only ever set to a name the table holds, which
    /// is the invariant this whole compartment exists to keep.
    pub(crate) fn current_handle(&self) -> Arc<RwLock<Buffer<B>>> {
        self.table
            .get(&*self.current)
            .expect("Corruption in the hashmap of buffers")
            .clone()
    }

    // ------------------------------------------------------------------
    // Asking
    // ------------------------------------------------------------------

    /// The current buffer's name, without copying it.
    pub fn current_name(&self) -> Arc<str> {
        self.current.clone()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.table.contains_key(name)
    }

    pub fn count(&self) -> usize {
        self.table.len()
    }

    /// Every live buffer's name, sorted. Used for buffer-name completion.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.table.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    /// Every live buffer's name with FIRST at the head, if it is one of them.
    ///
    /// For the background walkers: the highlighter and the prescanner both
    /// want to reach what is on screen before what is not, and both used to
    /// read the table directly to do it. The order among the rest is whatever
    /// the table gives -- the point is only that the focused one is not last.
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

    /// The most recently current buffer that still exists and is not EXCEPT.
    ///
    /// `None` when there is no such buffer. The caller answers that for
    /// itself -- there is always `*scratch*`, but falling back to it is a
    /// policy this does not get to make.
    pub fn most_recent(&self, except: &str) -> Option<String> {
        self.recency
            .iter()
            .find(|name| name.as_str() != except && self.table.contains_key(*name))
            .cloned()
    }

    // ------------------------------------------------------------------
    // Changing
    // ------------------------------------------------------------------

    /// Add BUFFER under NAME, replacing anything already there.
    pub fn insert(&mut self, name: &str, buffer: Buffer<B>) {
        self.table
            .insert(name.to_string(), Arc::new(RwLock::new(buffer)));
    }

    /// Make NAME current, and answer what was current before.
    ///
    /// `None` when there is no such buffer, having changed nothing. That
    /// refusal is the invariant: it is not possible from outside this file to
    /// leave `current` naming a buffer the table does not hold.
    pub fn make_current(&mut self, name: &str) -> Option<Arc<str>> {
        if !self.table.contains_key(name) {
            return None;
        }
        let previous = self.current.clone();
        self.current = Arc::from(name);
        self.record_use(name);
        Some(previous)
    }

    /// Move NAME to the front of the recency list.
    fn record_use(&mut self, name: &str) {
        if self.recency.first().is_some_and(|first| first == name) {
            // Already the most recent, which is the common case by far: this
            // runs whenever a buffer becomes current, `with-current-buffer'
            // included, so it is worth not touching the list at all.
            return;
        }
        self.recency.retain(|existing| existing != name);
        self.recency.insert(0, name.to_string());
    }

    /// Remove NAME, and say what the caller must now do about what is current.
    ///
    /// A fresh `*scratch*` is made when that emptied the table, because an
    /// editor with no buffers has nothing to be current and every path here
    /// assumes there is one.
    ///
    /// Whether `current` moved is the *caller's* problem to finish: this moves
    /// it, because leaving it dangling for even one statement is the bug this
    /// compartment exists to prevent, and then says so, because making a
    /// buffer current has consequences out in the editor -- a window to
    /// repoint, a mode line to redraw -- that are none of this file's
    /// business.
    pub fn remove(&mut self, name: &str) -> Removed {
        if self.table.remove(name).is_none() {
            return Removed::No;
        }
        self.recency.retain(|existing| existing != name);
        if self.table.is_empty() {
            self.table.insert(
                SCRATCH.to_string(),
                Arc::new(RwLock::new(Buffer::new(SCRATCH))),
            );
        }
        if &*self.current != name {
            return Removed::Yes;
        }
        // What was current has gone. The most recent survivor, or `*scratch*`,
        // which the table is guaranteed to hold by the branch above.
        let successor = self
            .most_recent(name)
            .unwrap_or_else(|| SCRATCH.to_string());
        self.current = Arc::from(successor.as_str());
        self.record_use(&successor);
        Removed::Current(successor)
    }
}

/// What [`Buffers::remove`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Removed {
    /// No such buffer.
    No,
    /// Gone, and it was not the current one.
    Yes,
    /// Gone, and it *was* current. The named buffer is current now; telling
    /// the rest of the editor is the caller's half of that.
    Current(String),
}
