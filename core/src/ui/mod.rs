mod frame;
pub use frame::FrameSnapshot;

mod windows;
pub use windows::{
    ComposeSettings, Division, FloatingWindow, Focus, GutterCell, GutterSpec, Highlight,
    LayoutNode, LineNumbers, MIN_DRAGGED_HEIGHT, MIN_DRAGGED_WIDTH, MIN_TEXT_WIDTH_FOR_GUTTER,
    Orientation, Rect, RenderableWindowView, Separator, Side, SplitPath, Window, WindowId,
    extract_buffer_lines, gutter_cells, gutter_columns, region_highlights, split_rects,
    syntax_highlights,
};

mod faces;
pub use faces::{Color, Face, NAMED_COLORS, Style, Theme};
