//! The two features whose mechanics are in Rust rather than in a `.lisp` file,
//! because too much depends on them: the minibuffer, and incremental search,
//! a prompt whose every keystroke searches again. Their primitives are in
//! `primitives/minibuffer.rs` and `primitives/isearch.rs`.
//!
//! Design: `doc/architettura-editor.typ, <minibuffer>`.
pub mod isearch;
pub mod minibuffer;
