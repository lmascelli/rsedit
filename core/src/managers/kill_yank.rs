//! Killed text, and what a yank put where.
//!
//! # Why this is a compartment and not seven fields
//!
//! The ring alone would be a field, not a compartment. What makes these one
//! fact is the pair of questions Emacs asks around every kill and every yank:
//! *did the command before this one also kill?* and *did it also yank?* A run
//! of `C-k` has to accumulate into one ring entry rather than filling the ring
//! with fragments, and `yank-pop` has to know exactly which text a yank just
//! inserted so it can take it back out. Neither question can be answered from
//! the ring; both are answered by flags that are only meaningful *against* it.
//!
//! Held apart -- a ring under one lock, four `AtomicBool`s beside it, and the
//! last yank's extent under a third -- they could be observed disagreeing. The
//! window is real: `kill` read the "did the last one kill" flag, then took the
//! ring, and appended or pushed according to a flag read before it held
//! anything. `note_yank` set a flag and then wrote the extent, so a reader
//! between the two saw a yank that had happened at no particular place.
//!
//! Held as one, `append`-or-`push` and flag-then-extent are each a single
//! operation, and there is no order left for anybody to get wrong.
//!
//! # The two flags per question
//!
//! `killed_this`/`killed_last` are not redundant. A command sets the *this*
//! flag while it runs; [`KillYank::roll_over`] moves it to *last* once the
//! command is over. Without the second, a command could not ask what its
//! predecessor did without also seeing what it had itself just done.
//!
//! # What is deliberately not here
//!
//! Buffers. A kill is text and an offset, and nothing in this file reads or
//! writes a buffer -- the text arrives as a `String`, and `note_yank` is told
//! where it went rather than looking.
//!
//! Lisp. Whether killed text should also reach the system clipboard is a
//! setting the interpreter holds, so the *answer* is passed in to
//! [`KillYank::kill`] rather than asked for here. No compartment takes an
//! `Env`.
use crate::kill_ring::{Direction, KillRing};

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
}

impl KillYank {
    // ------------------------------------------------------------------
    // Killing
    // ------------------------------------------------------------------

    /// Save TEXT as killed text.
    ///
    /// A run of kill commands accumulates into one entry rather than filling
    /// the ring with fragments -- that is what makes repeated `C-k` yank back
    /// as the whole passage. DIRECTION says which end of the entry a
    /// continued kill joins onto, so a backward kill does not assemble its
    /// text inside out.
    ///
    /// `to_clipboard` is whether the system clipboard should be told, decided
    /// by the caller because it is a Lisp setting and this file knows no Lisp.
    /// What the clipboard is given is whatever the ring now holds, not the
    /// fragment that just arrived: a run of `C-k` is one kill as far as the
    /// user is concerned, and sending each line on its own would leave the
    /// clipboard holding the last line of a passage they meant to take whole.
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

    /// Take the text owed to the system clipboard, leaving nothing behind.
    ///
    /// Called once per frame by the snapshot. Taking rather than reading is
    /// what stops a redraw of an unchanged frame from re-sending the same
    /// escape.
    pub fn take_pending_clipboard(&mut self) -> Option<String> {
        self.pending_clipboard.take()
    }

    // ------------------------------------------------------------------
    // The ring
    // ------------------------------------------------------------------

    /// What `yank` would insert, if anything.
    pub fn current(&self) -> Option<String> {
        self.ring.current().map(str::to_string)
    }

    /// The entry N kills back, without moving the ring.
    pub fn nth(&self, n: usize) -> Option<String> {
        self.ring.nth(n).map(str::to_string)
    }

    /// Step the ring back one entry and return what is now current.
    pub fn rotate(&mut self) -> Option<String> {
        self.ring.rotate().map(str::to_string)
    }

    pub fn set_max(&mut self, max: usize) {
        self.ring.set_max(max);
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.len() == 0
    }

    // ------------------------------------------------------------------
    // Yanking
    // ------------------------------------------------------------------

    /// Remember that a yank put LEN characters at AT, so `yank-pop` knows what
    /// to take back out.
    pub fn note_yank(&mut self, at: usize, len: usize) {
        self.yanked_this = true;
        self.last_yank = Some((at, len));
    }

    /// What the previous command yanked, if the previous command was a yank.
    ///
    /// `yank-pop` replaces the text a yank just inserted, so it is only
    /// meaningful directly after one; anything else in between and there is
    /// nothing it would be safe to remove.
    pub fn yank_to_replace(&self) -> Option<(usize, usize)> {
        self.yanked_last.then_some(self.last_yank).flatten()
    }

    // ------------------------------------------------------------------
    // Between commands
    // ------------------------------------------------------------------

    /// Roll "this command" into "the previous command" for both flags.
    ///
    /// One operation over both pairs, where it used to be two atomics swapped
    /// in a loop: they are rolled at the same instant because a command asks
    /// about both, and a reader that caught them half-rolled would be told
    /// its predecessor killed but did not yank when in fact it did neither.
    pub fn roll_over(&mut self) {
        self.killed_last = std::mem::replace(&mut self.killed_this, false);
        self.yanked_last = std::mem::replace(&mut self.yanked_this, false);
    }
}
