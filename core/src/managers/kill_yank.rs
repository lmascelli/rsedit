//! Killed text, and what a yank put where: the ring, and the flags that say
//! whether the command before this one killed or yanked. The flags only mean
//! something against the ring -- they are what makes a run of `C-k` one entry,
//! and what tells `yank-pop` which text to take back out.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
use crate::text::kill_ring::{Direction, KillRing};

/// The kill ring, the killed rectangle, the text owed to the clipboard, and
/// whether the last two commands killed or yanked.
#[derive(Default)]
pub struct KillYank {
    ring: KillRing,
    /// Text waiting to be handed to the *system* clipboard, or `None` when
    /// there is nothing outstanding.
    pending_clipboard: Option<String>,
    /// Whether the command before this one killed, and whether this one has.
    killed_last: bool,
    killed_this: bool,
    /// The same pair for yanking, and where the last yank put its text.
    yanked_last: bool,
    yanked_this: bool,
    last_yank: Option<(usize, usize)>,
    /// The last rectangle killed or copied, one string per line, or `None`
    /// when none has been: a shape, kept apart from the ring of text, so that
    /// what goes in as a block comes out as one.
    rectangle: Option<Vec<String>>,
}

impl KillYank {
    // ------------------------------------------------------------------
    // Killing
    // ------------------------------------------------------------------

    /// Saves TEXT as killed text: joined onto the newest entry, at the end
    /// DIRECTION says, when the previous command killed too, and as a new
    /// entry otherwise.
    ///
    /// With TO_CLIPBOARD, a Lisp setting the caller has read, the whole entry
    /// is then owed to the system clipboard rather than the piece that just
    /// arrived: a run of `C-k` is one kill.
    pub fn kill(&mut self, text: String, direction: Direction, to_clipboard: bool) {
        if self.killed_last {
            self.ring.append(text, direction);
        } else {
            self.ring.push(text);
        }
        self.killed_this = true;
        if to_clipboard && let Some(current) = self.ring.current() {
            self.pending_clipboard = Some(current.to_string());
        }
    }

    /// Takes the text owed to the system clipboard. The snapshot takes it
    /// once per frame, so that redrawing an unchanged frame does not send it
    /// again.
    pub fn take_pending_clipboard(&mut self) -> Option<String> {
        self.pending_clipboard.take()
    }

    // ------------------------------------------------------------------
    // The ring
    // ------------------------------------------------------------------

    /// Returns what `yank` would insert, if anything.
    pub fn current(&self) -> Option<String> {
        self.ring.current().map(str::to_string)
    }

    /// Returns the entry N kills back, without moving the ring.
    pub fn nth(&self, n: usize) -> Option<String> {
        self.ring.nth(n).map(str::to_string)
    }

    /// Steps the ring back one entry, and returns what is now current.
    pub fn rotate(&mut self) -> Option<String> {
        self.ring.rotate().map(str::to_string)
    }

    /// Sets how many entries the ring keeps.
    pub fn set_max(&mut self, max: usize) {
        self.ring.set_max(max);
    }

    /// Returns how many entries the ring holds.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    // ------------------------------------------------------------------
    // Rectangles
    // ------------------------------------------------------------------

    /// Saves LINES as the killed rectangle, replacing the last one: two
    /// rectangle kills in a row are two rectangles, not one block that was
    /// never on the screen.
    pub fn set_rectangle(&mut self, lines: Vec<String>) {
        self.rectangle = Some(lines);
    }

    /// Returns the killed rectangle, or `None` if nothing has been killed as
    /// one.
    pub fn rectangle(&self) -> Option<Vec<String>> {
        self.rectangle.clone()
    }

    // ------------------------------------------------------------------
    // Yanking
    // ------------------------------------------------------------------

    /// Records that a yank put LEN characters at AT, for `yank-pop` to take
    /// back out.
    pub fn note_yank(&mut self, at: usize, len: usize) {
        self.yanked_this = true;
        self.last_yank = Some((at, len));
    }

    /// Returns what the previous command yanked, when it was a yank:
    /// `yank-pop` can only replace text a yank has just inserted.
    pub fn yank_to_replace(&self) -> Option<(usize, usize)> {
        self.yanked_last.then_some(self.last_yank).flatten()
    }

    // ------------------------------------------------------------------
    // Between commands
    // ------------------------------------------------------------------

    /// Moves "this command" into "the previous command" for both flags at
    /// once: a command asks about both, and must not see them half moved.
    pub fn roll_over(&mut self) {
        self.killed_last = std::mem::replace(&mut self.killed_this, false);
        self.yanked_last = std::mem::replace(&mut self.yanked_this, false);
    }
}
