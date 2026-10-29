//! What the screen shows, apart from any front end: the windows and how they
//! share the frame, how one is composed into rows, highlights, a gutter and a
//! status line, the faces and the theme, and the frame a front end draws.
//!
//! Design: `doc/architettura-editor.typ, <frame>`.
mod compose;
mod faces;
mod frame;
mod gutter;
pub mod layout;
mod mode_line;
mod windows;

pub use compose::{
    ComposeSettings, Highlight, RenderableWindowView, Separator, compose_layout, region_highlights,
};
pub use faces::{Color, Face, NAMED_COLORS, Style, Theme};
pub use frame::FrameSnapshot;
pub use gutter::{GutterCell, GutterSpec, LineNumbers};
pub use windows::{
    Division, FloatingWindow, Focus, LayoutNode, Orientation, Rect, Side, SplitPath, Window,
    WindowId,
};
// For the tests, which check the layout against it.
#[cfg(test)]
pub use windows::MIN_DRAGGED_WIDTH;
