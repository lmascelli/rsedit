//! Text, and what can be worked out about it without an editor: searching, the
//! sessions of incremental search and replace, the kill ring, rectangles and
//! lists of places. Everything here needs only a haystack that hands over
//! characters by position -- a `BufferTrait` or a `&str` -- and could be used
//! by a program that is not an editor.
//!
//! Design: `doc/architettura-editor.typ, <testo>`.
pub mod isearch;
pub mod kill_ring;
pub mod rectangle;
pub mod replace;
pub mod results;
pub mod search;
