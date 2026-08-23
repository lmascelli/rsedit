use crate::buffer::{Buffer, BufferTrait};
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

pub struct RenderableWindowView {
    pub rect: Rect,
    pub buffer_name: String,
    pub is_focused: bool,
    pub cursor_rel_pos: Option<(usize, usize)>,
    pub lines: Vec<String>,
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

                out_views.push(RenderableWindowView {
                    rect,
                    buffer_name: win.buffer_name.clone(),
                    is_focused,
                    cursor_rel_pos,
                    lines,
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
