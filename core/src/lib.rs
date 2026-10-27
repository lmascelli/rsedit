//! An editor, as a library: a Lisp interpreter, a text buffer, the state
//! between them, and a description of a screen for somebody else to draw.
//!
//! # How to find your way around
//!
//! The modules below are listed in **dependency order**, and the list is the
//! map. Each one may use what is above it, and the interpreter, which is the
//! `risp` crate; of what is below, only what its entry names:
//!
//! ```text
//!   work/        where asynchronous work runs: the WorkManager, its
//!                scheduler and its dedicated threads. Knows nothing of
//!                editors: the state a job works on is a type parameter.
//!   paths.rs     where the editor keeps what it keeps on disk -- its
//!                configuration, its cache -- by each system's conventions.
//!   buffer/      text storage, and BufferTrait -- the only thing anything
//!                else is allowed to assume about it. A buffer emits a topic
//!                of work/ when its text changes, not knowing who listens.
//!                Of what is below, it uses only the types it stores: the
//!                faces of ui/, the colouring and scan states of modes/.
//!   text/        what can be worked out about text with nothing but
//!                BufferTrait: searching and the sessions of incremental
//!                search and replace, rectangles, the kill ring, lists of
//!                places.
//!   input.rs     keys, mouse events and keymaps: the vocabulary the editor
//!                takes input in. Of what is below, it uses only what a key
//!                is bound to: ELispExp, whose type names EditorState.
//!   commands.rs  the registry of what a name can mean as a command.
//!   ui/          a description of what to draw: windows and their layout,
//!                faces and themes, the snapshot of a frame. Draws nothing:
//!                there is no terminal, no window system and no crossterm in
//!                here. Of what is below, it uses only the buffer table it
//!                composes against, Buffers, and ELispExp.
//!   managers/    the editor's state, in compartments, each under one lock.
//!                Of what is below, it uses only what it stores: the modes
//!                and syntax tables of modes/, and ELispExp for what Lisp
//!                registers.
//!   editor/      EditorState: the facade over those compartments, and the
//!                only thing that may hold two of their locks at once.
//!   primitives/  everything Lisp can call. Without exception -- if it is a
//!                primitive, it is in here.
//!   modes/       what a major mode is, and how its text is read: the
//!                grammar that colours it, the syntax table, and the scanner
//!                of expressions that reads the table.
//!   jobs/        the editor's own background jobs: colouring, prescanning,
//!                the file watcher and auto-save.
//!   feature/     the two features whose mechanics could not be left to a
//!                .lisp file: the minibuffer and incremental search.
//! ```
//!
//! The last five are one layer, not an order: they use one another --
//! `editor/` installs the primitives and starts the jobs, which act on the
//! editor -- and the map lists them by what they are. `layering_tests`
//! holds each module from `work/` to `managers/` to what its entry names.
//!
//! Two boundaries are the load-bearing ones. The interpreter is a crate of its
//! own, so it cannot name an editor type: that is what lets it be tested, and
//! used, on its own. And `ui/` comes before `editor/` in the map, and names
//! nothing of it: that is what lets a renderer be written against a snapshot
//! instead of against this crate's internals.
//!
//! `managers/` and `editor/` pair by name. `managers/X` is compartment X: its
//! fields under one lock, and what can be done to them with nothing else.
//! `editor/X` is its half of the facade: what needs X together with another
//! compartment, a buffer's text or the interpreter, and the methods that give
//! X's operations an editor's name. The doors that take the compartments'
//! locks are all in `editor/mod.rs`; the other files of `editor/` are named
//! after what they coordinate -- `keys.rs`, `mouse.rs`, `frame.rs` -- and
//! hold no method that only forwards to a compartment.
//!
//! # Why everything is `pub(crate)`
//!
//! The public surface is the `pub use` list at the bottom of this file and
//! nothing else, so that a host -- `rsedit`'s own terminal front end, or
//! another -- is given a named set of types rather than the whole tree. The
//! cost is that an item meant for a host but left out of that list is
//! unreachable *and* reported as dead code, which is worth knowing when the
//! compiler claims something documented as host-facing is never used.

// -- the text, and what can be worked out about it ---------------------------
pub(crate) mod buffer;
pub(crate) mod text;

// -- where the editor's own directories are ----------------------------------
pub(crate) mod paths;

// -- what the editor takes in, and what a name can mean ----------------------
pub(crate) mod commands;
pub(crate) mod input;

// -- the editor's own state, and the jobs that keep it up to date ------------
pub(crate) mod editor;
pub(crate) mod jobs;
pub(crate) mod managers;

// -- what Lisp can reach, and what is built on it ----------------------------
pub(crate) mod feature;
pub(crate) mod modes;
pub(crate) mod primitives;

// -- where work runs, and what the screen is asked to show -------------------
pub(crate) mod ui;
// Where all asynchronous work runs. Design:
// `doc/architettura-editor.typ, <sfondo>`. Without the allow, the compiler
// would call dead code what only the tests use: the manual mode's drives, the
// core's own entry points, some of the re-exports.
#[allow(dead_code, unused_imports)]
pub(crate) mod work;

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
    editor::{EditorState, MOUSE_MODE, create_global_env, isolate_config_for_tests, mouse_mode},
    input::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseKind},
    managers::{Buffers, Windows},
    paths::{
        AppDir, CACHE_DIR, CONFIG_DIR, DATA_DIR, DirError, DirOptions, XDG_CACHE_HOME,
        XDG_CONFIG_HOME, XDG_DATA_HOME, app_dir,
    },
    ui::{
        Color, Face, FrameSnapshot, GutterCell, Highlight, NAMED_COLORS, Rect,
        RenderableWindowView, Separator, Style, Theme,
    },
};
pub use risp::{Env, LispContext, Parser, eval};
