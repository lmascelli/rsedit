//! The features whose mechanics are in Rust rather than in a `.lisp` file,
//! because too much depends on them: the minibuffer; incremental search, a
//! prompt whose every keystroke searches again; and the password prompt,
//! whose answer must never be in a buffer. Their primitives are in
//! `primitives/minibuffer.rs`, `primitives/isearch.rs` and
//! `primitives/password.rs`.
//!
//! Design: `doc/architettura-editor.typ, <minibuffer>`.
pub mod isearch;
pub mod minibuffer;
pub mod password;
