mod faces;
mod frame;
pub mod layout;
mod windows;

pub use faces::{Color, Face, NAMED_COLORS, Style, Theme};
pub use frame::FrameSnapshot;
pub use windows::{
    ComposeSettings, Division, FloatingWindow, Focus, GutterCell, GutterSpec, Highlight,
    LayoutNode, LineNumbers, Orientation, Rect, RenderableWindowView, Separator, Side, SplitPath,
    Window, WindowId, compose_layout, region_highlights,
};
// Reached only by the tests, which check the layout against it directly.
#[cfg(test)]
pub use windows::MIN_DRAGGED_WIDTH;
