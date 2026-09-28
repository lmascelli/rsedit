//! What has been typed at each prompt before.
//!
//! One ring per prompt, and a place in whichever ring is being walked. Those
//! two are one fact in two pieces -- a position means nothing without the list
//! it indexes, and moving through a list is the only thing a position is for
//! -- which is what makes them a compartment rather than two fields.
use std::collections::HashMap;

/// How many entries one prompt remembers when nothing says otherwise.
///
/// A hundred, which is Emacs' number and roughly a week of `M-x`. The cap is
/// per prompt rather than overall, so a prompt used constantly cannot push a
/// rarely-used one out of its own history.
pub const DEFAULT_HISTORY_LENGTH: usize = 100;

/// Where a walk through one ring has got to.
#[derive(Clone, Debug, PartialEq)]
struct Walk {
    /// Which ring is being walked.
    key: String,
    /// How far back, counting from the newest entry: 0 is the newest, and one
    /// past the end is where `stashed` belongs.
    position: usize,
    /// What was in the prompt when the walk started.
    ///
    /// # Why it is kept
    ///
    /// Walking back is a way of *looking*, and looking has to be undoable. A
    /// half-typed line replaced by the previous entry and then not recoverable
    /// is a prompt that punishes curiosity: the way back to what you were
    /// writing is to cancel and start again. So the first step back puts it
    /// here, and walking forward past the newest entry hands it out again.
    stashed: String,
}

#[derive(Default)]
pub struct History {
    /// Newest first, so the walk below is an index and not a subtraction.
    rings: HashMap<String, Vec<String>>,
    /// The walk in progress, or `None` when the prompt is showing something
    /// typed rather than something recalled.
    walk: Option<Walk>,
}

/// What to put in the prompt, if anything.
///
/// `None` rather than the unchanged text, so a prompt already at the oldest
/// entry is left exactly as it is: handing back a copy of the current line
/// would be indistinguishable from a successful step, and the difference is
/// the whole of what "you are at the end" feels like.
pub type Recalled = Option<String>;

impl History {
    /// Remember TEXT as the newest thing typed at KEY.
    ///
    /// # What is not remembered
    ///
    /// The empty string, which is what answering a prompt with nothing looks
    /// like and is never worth recalling. And a repeat of the newest entry:
    /// running the same command four times should leave one entry, or walking
    /// back through the history means pressing the key four times to move one
    /// step.
    ///
    /// A repeat further back *is* kept where it is, and a second copy added at
    /// the front. Hoisting it instead would reorder the history under the
    /// user, so a sequence they had walked through once would not be there the
    /// next time.
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

    /// Step one entry further back in KEY's ring, stashing CURRENT if this is
    /// the first step.
    ///
    /// CURRENT is passed in rather than read, because this compartment knows
    /// nothing about buffers -- see the rules in [`super`]. It is only looked
    /// at on the first step of a walk; on later ones the prompt is showing an
    /// entry, and stashing that would overwrite what was actually typed.
    pub fn previous(&mut self, key: &str, current: &str) -> Recalled {
        let length = self.rings.get(key).map_or(0, Vec::len);
        if length == 0 {
            return None;
        }
        let position = match &self.walk {
            // Continuing a walk in this same ring.
            Some(walk) if walk.key == key => walk.position + 1,
            // Starting one -- either the first step, or the first step after
            // the prompt changed rings, which can only happen if one prompt
            // closed and another opened.
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

    /// Step one entry forward, and past the newest entry back to whatever was
    /// being typed when the walk started.
    pub fn next(&mut self, key: &str) -> Recalled {
        let walk = self.walk.as_ref().filter(|walk| walk.key == key)?;
        let Some(position) = walk.position.checked_sub(1) else {
            // Forward from the newest entry: the walk is over, and what comes
            // back is what the walk interrupted.
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

    /// Forget where a walk had got to, without forgetting the history.
    ///
    /// Called when a prompt closes and when its text is edited: in both cases
    /// the position stops meaning anything, and a stale one would make the
    /// *next* step back continue a walk through a line that is no longer on
    /// screen.
    pub fn end_walk(&mut self) {
        self.walk = None;
    }

    /// KEY's entries, newest first.
    pub fn entries(&self, key: &str) -> Vec<String> {
        self.rings.get(key).cloned().unwrap_or_default()
    }

    /// Throw away what KEY remembers. True when there was something.
    pub fn forget(&mut self, key: &str) -> bool {
        if self.walk.as_ref().is_some_and(|walk| walk.key == key) {
            self.walk = None;
        }
        self.rings.remove(key).is_some_and(|ring| !ring.is_empty())
    }
}
