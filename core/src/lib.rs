//! An editor, as a library: a Lisp interpreter, a text buffer, the state
//! between them, and a description of a screen for somebody else to draw.
//!
//! # How to find your way around
//!
//! The modules below are listed in **dependency order**, outwards from the two
//! things that know nothing about each other, and the list is the map. Each one
//! may use what is above it and must not use what is below:
//!
//! ```text
//!   lisp/        the interpreter. Knows nothing of editors: no buffer, no
//!                point, no window. Its context is a type parameter.
//!   buffer/      text storage, and BufferTrait -- the only thing anything
//!                else is allowed to assume about it.
//!   text/        what can be worked out about text with nothing but
//!                BufferTrait: searching, rectangles, the kill ring, lists of
//!                places.
//!   input.rs     keys, mouse events and keymaps: the vocabulary the editor
//!   commands.rs  takes input in, and the registry of what a name can mean.
//!   managers/    the editor's state, in compartments, each under one lock.
//!   editor/      EditorState: the facade over those compartments, and the
//!                only thing that may hold two of their locks at once.
//!   primitives/  everything Lisp can call. Without exception -- if it is a
//!                primitive, it is in here.
//!   modes/       the major and minor modes built in Rust, and the background
//!                jobs that serve them (colouring, prescanning, auto-save,
//!                the file watcher).
//!   feature/     the two features whose mechanics could not be left to a
//!                .lisp file: the minibuffer and incremental search.
//!   background/  which thread work runs on, and the one rule that decides.
//!   ui/          a description of what to draw. Draws nothing: there is no
//!                terminal, no window system and no crossterm in here.
//! ```
//!
//! Two of those boundaries are the load-bearing ones, and both are checkable in
//! a single `grep`: `lisp/` names no editor type, and `ui/` names nothing from
//! `editor/`. The first is what lets the interpreter be tested, and used, on
//! its own; the second is what lets a renderer be written against a snapshot
//! instead of against this crate's internals.
//!
//! # Why everything is `pub(crate)`
//!
//! The public surface is the `pub use` list at the bottom of this file and
//! nothing else, so that a host -- `rsedit`'s own terminal front end, or
//! another -- is given a named set of types rather than the whole tree. The
//! cost is that an item meant for a host but left out of that list is
//! unreachable *and* reported as dead code, which is worth knowing when the
//! compiler claims something documented as host-facing is never used.

// -- the interpreter, and the text it has no knowledge of ---------------------
pub(crate) mod buffer;
pub(crate) mod text;

// -- what the editor takes in, and what a name can mean ----------------------
pub(crate) mod commands;
pub(crate) mod input;

// -- the editor's own state --------------------------------------------------
pub(crate) mod editor;
pub(crate) mod managers;

// -- what Lisp can reach, and what is built on it ----------------------------
pub(crate) mod feature;
pub(crate) mod modes;
pub(crate) mod primitives;

// -- where work runs, and what the screen is asked to show -------------------
pub(crate) mod background;
pub(crate) mod ui;

#[cfg(test)]
pub mod tests;

/// The interpreter's expression type, with this editor as its context.
///
/// Spelled out once here because every primitive in the crate takes a slice of
/// these, and `LispExp<EditorState<B>>` written out in full at each of them
/// would say nothing the name does not.
pub type ELispExp<B> = risp::LispExp<editor::EditorState<B>>;

pub use crate::{
    buffer::{BufferTrait, gap_buffer::GapBuffer},
    editor::{
        CONFIG_DIR, EditorState, MOUSE_MODE, XDG_CONFIG_HOME, create_global_env,
        isolate_config_for_tests, mouse_mode,
    },
    input::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseKind},
    managers::{Buffers, Windows},
    ui::{
        Color, Face, FrameSnapshot, GutterCell, Highlight, NAMED_COLORS, Rect,
        RenderableWindowView, Separator, Style, Theme,
    },
};
pub use risp::{Env, LispContext, Parser, eval};