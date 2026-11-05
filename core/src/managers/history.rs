//! What has been typed at each prompt before: one ring per prompt, and the
//! place a walk through one has got to -- a place that means nothing without
//! the ring it indexes.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
use std::collections::HashMap;

/// How many entries one prompt remembers when nothing says otherwise: a
/// hundred, as in Emacs. Per prompt, so that a prompt used all the time cannot
/// push another's entries out.
pub const DEFAULT_HISTORY_LENGTH: usize = 100;

/// Where a walk through one ring has got to.
#[derive(Clone, Debug, PartialEq)]
struct Walk {
    /// Which ring is being walked.
    key: String,
    /// How far back, counting from the newest entry: 0 is the newest, and one
    /// past the end is where `stashed` belongs.
    position: usize,
    /// What was in the prompt when the walk started, given back by walking
    /// forward past the newest entry: looking back must not lose what was
    /// being typed.
    stashed: String,
}

/// Each prompt's ring of entries, and the walk in progress.
#[derive(Default)]
pub struct History {
    /// Newest first, so the walk below is an index and not a subtraction.
    rings: HashMap<String, Vec<String>>,
    /// The walk in progress, or `None` when the prompt is showing something
    /// typed rather than something recalled.
    walk: Option<Walk>,
}

/// What to put in the prompt, if anything: `None` at either end of a ring,
/// which leaves the prompt exactly as it is rather than handing it a copy of
/// itself.
pub type Recalled = Option<String>;

impl History {
    /// Remembers TEXT as the newest thing typed at KEY, keeping LIMIT entries.
    ///
    /// Neither the empty string nor a repeat of the newest entry is kept. A
    /// repeat of an older entry is kept where it is, and added again at the
    /// front.
    pub fn remember(&mut self, key: &str, text: &str, limit: usize) {
        // A walk cannot survive the prompt it was walking in.
        self.walk = None;
        if text.is_empty() || limit == 0 {
            return;
        }
        let ring = self.rings.entry(key.to_string()).or_default();
        if ring.first().is_some_and(|newest| newest == text) {
            return;
        }
        ring.insert(0, text.to_string());
        ring.truncate(limit);
    }

    /// Steps one entry further back in KEY's ring.
    ///
    /// On the first step of a walk CURRENT, what the prompt shows, is
    /// stashed; on later steps the prompt shows an entry, and stashing it
    /// would lose what was typed.
    pub fn previous(&mut self, key: &str, current: &str) -> Recalled {
        let length = self.rings.get(key).map_or(0, Vec::len);
        if length == 0 {
            return None;
        }
        let position = match &self.walk {
            // Continuing a walk in this same ring.
            Some(walk) if walk.key == key => walk.position + 1,
            // Starting one: the first step, or the first in a ring that
            // another prompt has opened.
            _ => 0,
        };
        if position >= length {
            return None;
        }
        let stashed = match &self.walk {
            Some(walk) if walk.key == key => walk.stashed.clone(),
            _ => current.to_string(),
        };
        self.walk = Some(Walk {
            key: key.to_string(),
            position,
            stashed,
        });
        self.rings.get(key)?.get(position).cloned()
    }

    /// Steps one entry forward, and past the newest entry back to what was
    /// being typed when the walk started.
    pub fn next(&mut self, key: &str) -> Recalled {
        let walk = self.walk.as_ref().filter(|walk| walk.key == key)?;
        let Some(position) = walk.position.checked_sub(1) else {
            // Forward from the newest entry: the walk is over, and what comes
            // back is what it interrupted.
            let stashed = walk.stashed.clone();
            self.walk = None;
            return Some(stashed);
        };
        let stashed = walk.stashed.clone();
        self.walk = Some(Walk {
            key: key.to_string(),
            position,
            stashed,
        });
        self.rings.get(key)?.get(position).cloned()
    }

    /// Forgets where a walk had got to, but not the history.
    ///
    /// Called when a prompt closes and when its text is edited: a stale
    /// position would make the next step back continue a walk through a line
    /// no longer on screen.
    pub fn end_walk(&mut self) {
        self.walk = None;
    }

    /// Returns KEY's entries, newest first.
    pub fn entries(&self, key: &str) -> Vec<String> {
        self.rings.get(key).cloned().unwrap_or_default()
    }

    /// Returns every ring that holds something, as its key and its entries
    /// newest first, the keys in order: what a written history holds, and in
    /// the same order each time it is written.
    pub fn rings(&self) -> Vec<(String, Vec<String>)> {
        let mut rings: Vec<(String, Vec<String>)> = self
            .rings
            .iter()
            .filter(|(_, ring)| !ring.is_empty())
            .map(|(key, ring)| (key.clone(), ring.clone()))
            .collect();
        rings.sort_by(|a, b| a.0.cmp(&b.0));
        rings
    }

    /// Puts ENTRIES, newest first, behind what KEY already holds, keeping
    /// LIMIT entries in all: what was typed in this run stays newer than what
    /// was read back from an earlier one. Empty entries are left out, as
    /// [`Self::remember`] leaves them out.
    pub fn restore(&mut self, key: &str, entries: Vec<String>, limit: usize) {
        if limit == 0 {
            return;
        }
        let ring = self.rings.entry(key.to_string()).or_default();
        ring.extend(entries.into_iter().filter(|entry| !entry.is_empty()));
        ring.truncate(limit);
        if ring.is_empty() {
            self.rings.remove(key);
        }
    }

    /// Throws away what KEY remembers, and returns whether there was
    /// anything.
    pub fn forget(&mut self, key: &str) -> bool {
        if self.walk.as_ref().is_some_and(|walk| walk.key == key) {
            self.walk = None;
        }
        self.rings.remove(key).is_some_and(|ring| !ring.is_empty())
    }
}
