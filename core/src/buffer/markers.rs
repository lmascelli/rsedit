//! Places in a buffer that follow its text: an offset kept as the text is
//! edited before it, so that what was found there is still found there --
//! the places of a list of results, which `next-error` goes back to after
//! the lines above them have moved.
//!
//! Moved by the three doors, as overlays and virtual text are.
//!
//! Design: `doc/architettura-editor.typ, <risultati>`.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// The next marker's id, across every buffer: an id that a buffer killed and
/// opened again under the same name does not have is one it never had,
/// rather than another marker's.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The markers of one buffer, by id.
#[derive(Clone, Debug, Default)]
pub struct Markers {
    at: HashMap<u64, usize>,
}

impl Markers {
    /// Adds a marker at offset AT, and returns its id.
    pub fn add(&mut self, at: usize) -> u64 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        self.at.insert(id, at);
        id
    }

    /// Returns where marker ID is now, or `None` when this buffer has none.
    pub fn get(&self, id: u64) -> Option<usize> {
        self.at.get(&id).copied()
    }

    /// Removes marker ID, if this buffer has it.
    pub fn remove(&mut self, id: u64) {
        self.at.remove(&id);
    }

    /// Returns how many markers there are.
    pub fn len(&self) -> usize {
        self.at.len()
    }

    /// Returns whether there are none.
    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    /// Moves the markers at or after AT on by N, for N characters inserted
    /// there: one exactly at AT moves too, so that a marker at the start of
    /// a match stays on the match when text is typed in front of it.
    pub fn adjust_for_insert(&mut self, at: usize, n: usize) {
        if n == 0 {
            return;
        }
        for marker in self.at.values_mut() {
            if *marker >= at {
                *marker += n;
            }
        }
    }

    /// Moves the markers back for the deletion of `[from, to)`: one inside
    /// the deleted text lands at its start.
    pub fn adjust_for_delete(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        let shift = to - from;
        for marker in self.at.values_mut() {
            *marker = if *marker <= from {
                *marker
            } else if *marker >= to {
                *marker - shift
            } else {
                from
            };
        }
    }
}
