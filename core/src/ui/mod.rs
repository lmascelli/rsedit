mod faces;
mod frame;
pub mod layout;
mod windows;

pub use faces::{Color, Face, NAMED_COLORS, Style, Theme};
pub use frame::FrameSnapshot;
pub use windows::{
    ComposeSettings, Division, FloatingWindow, Focus, GutterCell, GutterSpec, Highlight,
    LayoutNode, LineNumbers, MIN_DRAGGED_HEIGHT, MIN_DRAGGED_WIDTH, MIN_TEXT_WIDTH_FOR_GUTTER,
    Orientation, Rect, RenderableWindowView, Separator, Side, SplitPath, Window, WindowId,
    compose_layout, gutter_cells, gutter_columns, region_highlights, split_rects,
    syntax_highlights,
};
