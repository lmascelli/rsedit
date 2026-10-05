//! The features whose mechanics are in Rust rather than in a `.lisp` file.
//!
//! # Why anything is here at all
//!
//! Editor *policy* belongs in `core/lisp/`, where it can be redefined,
//! rebound, or left out. These two are the exceptions, and both for the same
//! reason: too much else depends on them for them to be optional.
//!
//! - [`minibuffer`] is how the editor reads a line of input. `M-x`, `M-:`,
//!   `find-file`, search and replace are all a prompt with a different
//!   callback. A `.lisp` file that failed to load would leave an editor that
//!   could not ask a question.
//! - [`isearch`] is a prompt whose every keystroke re-runs a search. Its
//!   keymap is the first one consulted while the prompt is open, so a file that
//!   failed to load would leave a prompt whose keys did not mean what the
//!   prompt says they mean.
//!
//! A third candidate would have to argue the same case. "It would be faster in
//! Rust" is not that case.
//!
//! # Where each one's Lisp surface is
//!
//! Not here. Every primitive the editor provides lives under `primitives/`
//! without exception, so the answer to "where is this primitive defined" is
//! always the same answer -- see [`crate::primitives::minibuffer`] and
//! [`crate::primitives::isearch`]. What stays here is the state machine the
//! primitives drive, and the mode each one installs: a keymap that must exist
//! whatever `.lisp` loaded is the same kind of fact as the mechanics it binds.
pub mod isearch;
pub mod minibuffer;
