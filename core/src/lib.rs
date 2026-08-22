pub mod buffer;
pub(crate) mod editor;
pub mod input;
pub mod lisp;
pub(crate) mod modes;
pub(crate) mod primitives;
pub(crate) mod task;
pub mod ui;
pub type ELispExp<B> = lisp::LispExp<editor::EditorState<B>>;

pub use crate::{
    buffer::BufferTrait,
    editor::{EditorState, create_global_env},
};

#[cfg(test)]
pub mod tests;
