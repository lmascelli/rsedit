//! The editor's state, in compartments.
//!
//! Each module here owns a group of fields that are one fact in several
//! pieces, holds them under one lock, and exposes operations rather than the
//! pieces. [`crate::editor::EditorState`] keeps one handle to each and is the
//! facade: it coordinates between compartments, and it is the only thing that
//! may hold two of their locks at once.
//!
//! # The rules a compartment follows
//!
//! - **It does not reach another compartment.** Where an operation needs a
//!   fact from one, the *value* is passed in and the answer passed back:
//!   `Windows::scroll_focused` is told a line count rather than being handed a
//!   buffer. That is what keeps two of these locks from ever being held
//!   together by anything but the facade.
//! - **It hands out no guards.** State is reached through a closure, so a lock
//!   cannot outlive the question that needed it.
//! - **It knows nothing of Lisp.** No compartment takes an `Env`, so no
//!   compartment can be holding a lock when user code re-enters the editor.
//! - **It answers with a verdict, not a side effect.** Where an operation has
//!   consequences outside its own fields -- a buffer to repoint, a window to
//!   refocus -- it returns an enum saying so and lets the facade act.
mod buffers;
mod windows;

pub use buffers::{Buffers, Removed as BufferRemoved, SCRATCH};
pub use windows::{
    DRAG_SCROLL_INTERVAL, Hit, MouseDrag, Removed as WindowRemoved, Scrolled, Windows,
};
