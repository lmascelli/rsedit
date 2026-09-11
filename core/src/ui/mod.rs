mod frame;
pub use frame::FrameSnapshot;

mod windows;
pub use windows::{
    FloatingWindow, Highlight, LayoutNode, Orientation, Rect, RenderableWindowView, Separator,
    Window, extract_buffer_lines, region_highlights,
};

mod faces;
pub use faces::{Color, Face, NAMED_COLORS, Style, Theme};
