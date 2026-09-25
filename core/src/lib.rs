pub mod buffer;
pub(crate) mod commands;
pub(crate) mod editor;
pub mod input;
pub(crate) mod isearch;
pub(crate) mod kill_ring;
pub mod lisp;
pub(crate) mod minibuffer;
pub(crate) mod modes;
pub(crate) mod primitives;
pub mod search;
pub(crate) mod task;
pub mod ui;
pub mod windows;
pub type ELispExp<B> = lisp::LispExp<editor::EditorState<B>>;

pub use crate::{
    buffer::BufferTrait,
    editor::{EditorState, MOUSE_MODE, create_global_env, mouse_mode},
    windows::Windows,
};

#[cfg(test)]
pub mod tests;
