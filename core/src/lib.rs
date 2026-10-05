pub(crate) mod buffer;
pub(crate) mod commands;
pub(crate) mod editor;
pub(crate) mod input;
pub(crate) mod isearch;
pub(crate) mod kill_ring;
pub(crate) mod lisp;
pub(crate) mod managers;
pub(crate) mod minibuffer;
pub(crate) mod modes;
pub(crate) mod primitives;
pub(crate) mod rectangle;
pub(crate) mod results;
pub(crate) mod search;
pub(crate) mod task;
pub(crate) mod ui;
pub(crate) mod worker;

#[cfg(test)]
pub mod tests;

pub type ELispExp<B> = lisp::LispExp<editor::EditorState<B>>;

pub use crate::{
    buffer::{BufferTrait, gap_buffer::GapBuffer},
    editor::{
        CONFIG_DIR, EditorState, MOUSE_MODE, XDG_CONFIG_HOME, create_global_env,
        isolate_config_for_tests, mouse_mode,
    },
    input::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseKind},
    lisp::{Env, LispContext, Parser, eval},
    managers::{Buffers, Windows},
    ui::{
        Color, Face, FrameSnapshot, GutterCell, Highlight, NAMED_COLORS, Rect,
        RenderableWindowView, Separator, Style, Theme,
    },
};
