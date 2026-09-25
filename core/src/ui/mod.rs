mod frame;
pub use frame::FrameSnapshot;

mod windows;
pub use windows::{
    Division, FloatingWindow, Focus, Highlight, LayoutNode, MIN_DRAGGED_HEIGHT, MIN_DRAGGED_WIDTH,
    Orientation, Rect, RenderableWindowView, Separator, Side, SplitPath, Window, WindowId,
    extract_buffer_lines, region_highlights, split_rects, syntax_highlights,
};

mod faces;
pub use faces::{Color, Face, NAMED_COLORS, Style, Theme};
