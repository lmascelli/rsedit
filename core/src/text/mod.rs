//! Text, and what can be worked out about it without an editor.
//!
//! # What belongs here
//!
//! Everything in this directory needs a haystack that can hand over characters
//! by position, and nothing else: no point, no mark, no undo history, no locks,
//! no window. The test is simple and worth applying before adding a file ---
//!
//! > Could this be used by a program that is not an editor?
//!
//! [`search`] could: it is a scanner over anything implementing
//! [`crate::buffer::BufferTrait`]. [`rectangle`] could: it is the arithmetic of
//! two corners and the lines between them. [`kill_ring`] could: it is a ring of
//! strings with a rotation rule. [`results`] could: it is a list of places and
//! a cursor into it.
//!
//! What does *not* belong is anything that reads a `Buffer` rather than a
//! `BufferTrait`, because `Buffer` is where the editor's own state begins ---
//! and anything that takes an `Env`, because that is the interpreter's.
//!
//! # Why this is a directory and not four files at the root
//!
//! These four used to sit beside `editor/`, `ui/`, `input.rs` and the rest, so
//! the root of the crate was a list of eleven things with no principle. The
//! layering was real -- it is described in several module docs -- but it was
//! only described. A directory makes it checkable: an import of `editor` or
//! `lisp` from anything in here is the mistake, and it is visible in the file's
//! first ten lines.
pub mod kill_ring;
pub mod rectangle;
pub mod results;
pub mod search;
