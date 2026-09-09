use crate::buffer::{Buffer, BufferTrait, mark::region_bounds};
use crate::ui::Face;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: isize,
    pub y: isize,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: usize,
    pub buffer_name: String,
    pub scroll_x: usize,
    pub scroll_y: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutNode {
    Leaf(Window),
    Split {
        orientation: Orientation,
        ratio: f32,
        left: Box<LayoutNode>,
        right: Box<LayoutNode>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FloatingWindow {
    pub window: Window,
    pub rect: Rect,
    pub has_border: bool,
    pub title: Option<String>,
    /// The window that was focused right before this floating window was
    /// opened. Restored when the floating window closes, so closing one
    /// (including a floating window opened while another one already had
    /// focus) always lands back exactly where the user was.
    pub previous_focused_window_id: usize,
}

/// A run of characters in one drawn row that should be drawn differently.
///
/// In *screen* coordinates relative to the window's rect, already clipped to
/// it, so a renderer needs no knowledge of scrolling or of the buffer to draw
/// one. Kept beside `lines` rather than replacing them with styled cells: the
/// plain text is what almost every row is, and a list of exceptions is both
/// smaller and easier for a renderer to ignore than a parallel array of
/// attributes.
///
/// The region is the first thing to use this. Syntax highlighting (#22) is the
/// next, and adds entries here rather than a second mechanism.
#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
    /// Row within the window, 0 being its first drawn line.
    pub row: usize,
    /// Half-open column range within the row, in characters.
    pub start_col: usize,
    pub end_col: usize,
    pub face: Face,
}

/// One window, resolved to exactly what should appear on screen.
///
/// Owned data with no borrows back into editor state, so a [`FrameSnapshot`]
/// built from these can outlive the locks it was captured under.
///
/// [`FrameSnapshot`]: crate::ui::FrameSnapshot
#[derive(Clone, Debug, PartialEq)]
pub struct RenderableWindowView {
    pub rect: Rect,
    pub buffer_name: String,
    /// What the window's border should be labelled, when it has one.
    ///
    /// A floating window carries the caller's title -- for the minibuffer that
    /// is the prompt, which is the entire point of showing a border. Tiled
    /// windows have none and fall back to the buffer name.
    pub title: Option<String>,
    pub is_focused: bool,
    pub cursor_rel_pos: Option<(usize, usize)>,
    pub lines: Vec<String>,
    /// Runs within `lines` to draw with a face other than the default.
    pub highlights: Vec<Highlight>,
    pub has_border: bool,
}

impl LayoutNode {
    pub fn get_window_by_id(&mut self, id: usize) -> Option<&mut Window> {
        match self {
            LayoutNode::Leaf(window) => Some(window),
            LayoutNode::Split {
                orientation: _,
                ratio: _,
                left,
                right,
            } => left
                .get_window_by_id(id)
                .or_else(|| right.get_window_by_id(id)),
        }
    }

    pub fn compute_tiled_views<B: BufferTrait>(
        &mut self,
        rect: Rect,
        focused_id: usize,
        buffers: &HashMap<String, Arc<RwLock<Buffer<B>>>>,
        out_views: &mut Vec<RenderableWindowView>,
    ) {
        match self {
            LayoutNode::Leaf(win) => {
                let is_focused = win.id == focused_id;
                let mut cursor_rel_pos = None;

                if is_focused {
                    if let Some(buf) = buffers.get(&win.buffer_name) {
                        let (c_line, c_col) = buf
                            .read()
                            .expect("Failed to acquire read lock on buffer")
                            .text
                            .cursor_pos();

                        if c_line < win.scroll_y {
                            win.scroll_y = c_line;
                        } else if c_line >= win.scroll_y + rect.height {
                            win.scroll_y = c_line - rect.height + 1;
                        }

                        if c_col < win.scroll_x {
                            win.scroll_x = c_col;
                        } else if c_col >= win.scroll_x + rect.width {
                            win.scroll_x = c_col - rect.width + 1;
                        }

                        cursor_rel_pos = Some((
                            c_col.saturating_sub(win.scroll_x),
                            c_line.saturating_sub(win.scroll_y),
                        ));
                    }
                }

                let lines = extract_buffer_lines(win, &rect, buffers);
                let highlights = region_highlights(win, &rect, buffers);

                out_views.push(RenderableWindowView {
                    rect,
                    buffer_name: win.buffer_name.clone(),
                    title: None,
                    is_focused,
                    cursor_rel_pos,
                    lines,
                    highlights,
                    has_border: false,
                });
            }

            LayoutNode::Split {
                orientation,
                ratio,
                left,
                right,
            } => match orientation {
                Orientation::Horizontal => {
                    let left_height = ((rect.height as f32) * *ratio).round() as usize;
                    let right_height = rect.height.saturating_sub(left_height);

                    left.compute_tiled_views(
                        Rect {
                            height: left_height,
                            ..rect
                        },
                        focused_id,
                        buffers,
                        out_views,
                    );
                    right.compute_tiled_views(
                        Rect {
                            y: rect.y + left_height as isize,
                            height: right_height,
                            ..rect
                        },
                        focused_id,
                        buffers,
                        out_views,
                    );
                }
                Orientation::Vertical => {
                    let left_width = ((rect.width as f32) * *ratio).round() as usize;
                    let right_width = rect.width.saturating_sub(left_width);

                    left.compute_tiled_views(
                        Rect {
                            width: left_width,
                            ..rect
                        },
                        focused_id,
                        buffers,
                        out_views,
                    );
                    right.compute_tiled_views(
                        Rect {
                            x: rect.x + left_width as isize,
                            width: right_width,
                            ..rect
                        },
                        focused_id,
                        buffers,
                        out_views,
                    );
                }
            },
        }
    }
}

/// The active region of WIN's buffer, as spans within the rows WIN is
/// showing.
///
/// Returns nothing when there is no active region, when the buffer is not the
/// one on screen, or when the region lies entirely outside the visible rows --
/// so a selection made and then scrolled away from costs nothing to not draw.
///
/// Coordinates come back relative to the window, with horizontal scrolling
/// already applied and both ends clipped to the window's width, because the
/// renderer knows about the screen and should not have to know about the
/// buffer.
pub fn region_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &HashMap<String, Arc<RwLock<Buffer<B>>>>,
) -> Vec<Highlight> {
    let Some(buf) = buffers.get(&win.buffer_name) else {
        return Vec::new();
    };
    let buf = buf
        .read()
        .expect("Failed to acquire read lock on buffer for highlighting");
    let Some((start, end)) = region_bounds(buf.mark, buf.text.cursor_pos_1d(), buf.text.len())
    else {
        return Vec::new();
    };

    let (start_line, start_col) = buf.text.cursor_1d_to_2d(start);
    let (end_line, end_col) = buf.text.cursor_1d_to_2d(end);

    let first_visible = win.scroll_y;
    let last_visible = win.scroll_y + rect.height;
    let mut highlights = Vec::new();

    for line in start_line.max(first_visible)..=end_line.min(last_visible.saturating_sub(1)) {
        // A line in the middle of the region is selected from its first
        // character to its last; only the two ends of the region are partial.
        let from = if line == start_line { start_col } else { 0 };
        let to = if line == end_line {
            end_col
        } else {
            // One past the last character, so the newline shows as selected --
            // which is how a multi-line selection reads as covering whole
            // lines rather than stopping raggedly at each line's end.
            line_width(&buf.text, line) + 1
        };

        let from = from.saturating_sub(win.scroll_x);
        let to = to.saturating_sub(win.scroll_x).min(rect.width);
        if from >= to {
            continue;
        }
        highlights.push(Highlight {
            row: line - win.scroll_y,
            start_col: from,
            end_col: to,
            face: Face::Region,
        });
    }
    highlights
}

/// How many characters LINE holds, not counting its newline.
fn line_width<B: BufferTrait>(text: &B, line: usize) -> usize {
    text.get_lines(line, line + 1)
        .first()
        .map(|l| l.chars().count())
        .unwrap_or(0)
}

pub fn extract_buffer_lines<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &HashMap<String, Arc<RwLock<Buffer<B>>>>,
) -> Vec<String> {
    let mut visible_lines = Vec::new();
    if let Some(buf) = buffers.get(&win.buffer_name) {
        let lines = buf
            .read()
            .expect("Failed to acquire read lock for buffer")
            .text
            .get_lines(win.scroll_y, win.scroll_y + rect.height);

        for line in lines {
            let chopped: String = line.chars().skip(win.scroll_x).take(rect.width).collect();
            visible_lines.push(chopped);
        }
    }
    visible_lines
}
