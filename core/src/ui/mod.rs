use crate::buffer::{Buffer, BufferTrait};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
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
    pub fn compute_tiled_views<B: BufferTrait>(
        &self,
        rect: Rect,
        focused_id: usize,
        buffers: &HashMap<String, Buffer<B>>,
        out_views: &mut Vec<RenderableWindowView>,
    ) {
        match self {
            LayoutNode::Leaf(win) => {
                let lines = extract_buffer_lines(win, &rect, buffers);
                let is_focused = win.id == focused_id;
                let cursor_rel_pos = if is_focused {
                    if let Some(buf) = buffers.get(&win.buffer_name) {
                        let (c_line, c_col) = buf.text.cursor_pos();
                        Some((
                            c_col.saturating_sub(win.scroll_x),
                            c_line.saturating_sub(win.scroll_y),
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };

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
                    let left_height = ((rect.height as f32) * ratio).round() as usize;
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
                            y: rect.y + left_height,
                            height: right_height,
                            ..rect
                        },
                        focused_id,
                        buffers,
                        out_views,
                    );
                }
                Orientation::Vertical => {
                    let left_width = ((rect.width as f32) * ratio).round() as usize;
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
                            x: rect.x + left_width,
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
    buffers: &HashMap<String, Buffer<B>>,
) -> Vec<String> {
    let mut visible_lines = Vec::new();
    if let Some(buf) = buffers.get(&win.buffer_name) {
        // TODO(LM) do not take the full text but use the BufferTrait
        // functionalities to extract the useful text (maybe it can
        // increase the performance for some implementations of
        // buffers)
        let full_text = buf.text.to_string();
        let lines: Vec<&str> = full_text.split('\n').collect();

        for &line in lines.iter().skip(win.scroll_y).take(rect.height) {
            let chopped: String = line.chars().skip(win.scroll_x).take(rect.width).collect();
            visible_lines.push(chopped);
        }
    }
    visible_lines
}
