pub mod buffer;
pub(crate) mod commands;
pub(crate) mod editor;
pub mod input;
pub(crate) mod isearch;
pub(crate) mod kill_ring;
pub mod lisp;
pub mod managers;
pub(crate) mod minibuffer;
pub(crate) mod modes;
pub(crate) mod primitives;
pub mod results;
pub mod rectangle;
pub mod search;
pub(crate) mod task;
pub mod ui;
pub(crate) mod worker;
pub type ELispExp<B> = lisp::LispExp<editor::EditorState<B>>;

pub use crate::{
    buffer::BufferTrait,
    editor::{
        CONFIG_DIR, EditorState, MOUSE_MODE, XDG_CONFIG_HOME, create_global_env,
        isolate_config_for_tests, mouse_mode,
    },
    managers::{Buffers, Windows},
};

#[cfg(test)]
pub mod tests;
