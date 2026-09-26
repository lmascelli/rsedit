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
//! - **It does nothing slow while holding the lock.** No disk, no network, no
//!   waiting. `Log::record` appends the line and hands the *file* back for the
//!   caller to write to, because `log_diagnostic` used to `write_all` with the
//!   list's write lock still open -- putting a disk write inside a lock that
//!   every diagnostic in the editor, from the worker thread as well as this
//!   one, had to queue behind.
//!
//! The first four rules are about *correctness*: each one removes a way for
//! two pieces of state to be seen disagreeing. The last is about *latency*,
//! and it is the one that will be broken by accident, because breaking it
//! costs nothing until the disk is slow or the tree is large. The tell is a
//! lock held across a call whose duration you do not control -- I/O, an
//! allocation the size of a file, a callback. If you cannot say how long the
//! body takes, copy what you need out and let go first.
mod buffers;
mod commands;
mod kill_yank;
mod log;
mod modes;
mod runtime;
mod windows;

pub use buffers::{Buffers, Removed as BufferRemoved, SCRATCH};
pub use commands::Commands;
pub use kill_yank::KillYank;
pub use log::Log;
pub use modes::Modes;
pub use runtime::Runtime;
pub use windows::{
    DRAG_SCROLL_INTERVAL, Hit, MouseDrag, Removed as WindowRemoved, Scrolled, Windows,
};
