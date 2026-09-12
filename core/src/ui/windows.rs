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

#[derive(Clone, Debug, PartialEq)]
pub struct Separator {
    pub rect: Rect,
    pub ch: char,
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
    /// The status line for this window, drawn on the row immediately below
    /// `rect` -- outside it, as a border is. `None` for a window with no room
    /// for one, and for floating windows, which say what they are with a title
    /// on their border instead.
    pub mode_line: Option<String>,
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

    /// Every window, in the order they are laid out on screen.
    ///
    /// Left-to-right, top-to-bottom, because that is the order the tree is
    /// built in -- which is what makes "the next window" mean what a user
    /// expects when they cycle through them.
    pub fn window_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        self.collect_window_ids(&mut ids);
        ids
    }

    fn collect_window_ids(&self, out: &mut Vec<usize>) {
        match self {
            LayoutNode::Leaf(window) => out.push(window.id),
            LayoutNode::Split { left, right, .. } => {
                left.collect_window_ids(out);
                right.collect_window_ids(out);
            }
        }
    }

    /// Turn the window with ID into a split holding it and NEW_WINDOW.
    ///
    /// The existing window keeps the first half, so a split "below" or "right"
    /// puts the new window where its name says. Returns false when no window
    /// has that id.
    ///
    /// The tree knows nothing about sizes -- those are computed at render time
    /// from whatever rect the frame gives it -- so there is no "too small to
    /// split" to check here. A split of a one-row window yields two windows,
    /// one of which draws nothing until the frame grows.
    pub fn split_window(
        &mut self,
        id: usize,
        orientation: Orientation,
        new_window: Window,
    ) -> bool {
        match self {
            LayoutNode::Leaf(window) if window.id == id => {
                let existing = std::mem::replace(window, placeholder_window());
                *self = LayoutNode::Split {
                    orientation,
                    ratio: 0.5,
                    left: Box::new(LayoutNode::Leaf(existing)),
                    right: Box::new(LayoutNode::Leaf(new_window)),
                };
                true
            }
            LayoutNode::Leaf(_) => false,
            LayoutNode::Split { left, right, .. } => {
                left.split_window(id, orientation, new_window.clone())
                    || right.split_window(id, orientation, new_window)
            }
        }
    }

    /// Remove the window with ID, and collapse the split it was half of.
    ///
    /// Returns false when no window has that id, and when the window is the
    /// only one there is -- a frame with no windows has nowhere to put the
    /// cursor, so the last one cannot be closed.
    pub fn remove_window(&mut self, id: usize) -> bool {
        let LayoutNode::Split { left, right, .. } = self else {
            // A lone leaf. Even if it matches, it cannot go.
            return false;
        };
        let is_target = |node: &LayoutNode| matches!(node, LayoutNode::Leaf(w) if w.id == id);

        if is_target(left) || is_target(right) {
            // The sibling takes the place of the split entirely, which is what
            // makes the remaining window grow into the space.
            let survivor = if is_target(left) { right } else { left };
            *self = std::mem::replace(&mut **survivor, LayoutNode::Leaf(placeholder_window()));
            return true;
        }
        left.remove_window(id) || right.remove_window(id)
    }

    /// Replace the whole tree with just the window with ID.
    pub fn keep_only(&mut self, id: usize) -> bool {
        let Some(window) = self.window(id).cloned() else {
            return false;
        };
        *self = LayoutNode::Leaf(window);
        true
    }

    /// The window with ID, if the tree holds one.
    pub fn window(&self, id: usize) -> Option<&Window> {
        match self {
            LayoutNode::Leaf(window) => (window.id == id).then_some(window),
            LayoutNode::Split { left, right, .. } => left.window(id).or_else(|| right.window(id)),
        }
    }
}

/// A window that exists only long enough to be overwritten.
///
/// `std::mem::replace` needs something to leave behind while a subtree is
/// moved out of the tree it is part of. Nothing ever reads this, and it is
/// gone by the time the function it is used in returns.
fn placeholder_window() -> Window {
    Window {
        id: usize::MAX,
        buffer_name: String::new(),
        scroll_x: 0,
        scroll_y: 0,
    }
}

/// The narrowest a rect can be and still give a column to a divider.
///
/// Three: one for each window and one for the rule.
const MIN_WIDTH_FOR_SEPARATOR: usize = 3;

impl LayoutNode {
    pub fn compute_tiled_views<B: BufferTrait>(
        &mut self,
        rect: Rect,
        focused_id: usize,
        buffers: &HashMap<String, Arc<RwLock<Buffer<B>>>>,
        mode_line_format: &str,
        out_views: &mut Vec<RenderableWindowView>,
        out_separators: &mut Vec<Rect>,
    ) {
        match self {
            LayoutNode::Leaf(win) => {
                let is_focused = win.id == focused_id;

                // The status line takes the window's bottom row, so the text
                // gets one fewer -- computed here, before anything reads the
                // rect, so scrolling and the cursor agree with what is drawn.
                // A window with only one row keeps it for text: a status line
                // with nothing under it says nothing useful.
                let mode_line = buffers.get(&win.buffer_name).and_then(|buffer| {
                    (rect.height >= 2).then(|| {
                        expand_mode_line(
                            mode_line_format,
                            &buffer
                                .read()
                                .expect("Failed to acquire read lock on buffer"),
                        )
                    })
                });
                let rect = if mode_line.is_some() {
                    Rect {
                        height: rect.height - 1,
                        ..rect
                    }
                } else {
                    rect
                };

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
                    mode_line,
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
                        mode_line_format,
                        out_views,
                        out_separators,
                    );
                    right.compute_tiled_views(
                        Rect {
                            y: rect.y + left_height as isize,
                            height: right_height,
                            ..rect
                        },
                        focused_id,
                        buffers,
                        mode_line_format,
                        out_views,
                        out_separators,
                    );
                }
                Orientation::Vertical => {
                    let divided = rect.width >= MIN_WIDTH_FOR_SEPARATOR;
                    let usable = rect.width.saturating_sub(divided as usize);
                    let left_width = ((usable as f32) * *ratio).round() as usize;
                    let right_width = usable.saturating_sub(left_width);

                    if divided {
                        out_separators.push(Rect {
                            x: rect.x + left_width as isize,
                            y: rect.y,
                            width: 1,
                            height: rect.height,
                        });
                    }

                    left.compute_tiled_views(
                        Rect {
                            width: left_width,
                            ..rect
                        },
                        focused_id,
                        buffers,
                        mode_line_format,
                        out_views,
                        out_separators,
                    );
                    right.compute_tiled_views(
                        Rect {
                            x: rect.x + left_width as isize + divided as isize,
                            width: right_width,
                            ..rect
                        },
                        focused_id,
                        buffers,
                        mode_line_format,
                        out_views,
                        out_separators,
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
            face: Face::REGION,
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

/// Expand a mode-line format string against WIN's buffer.
///
/// The escapes are Emacs' own, so a format copied from an Emacs configuration
/// means the same thing here:
///
/// | escape | shows                                        |
/// |--------|----------------------------------------------|
/// | `%b`   | buffer name                                  |
/// | `%f`   | file path, or the buffer name when unvisited |
/// | `%m`   | major mode                                   |
/// | `%l`   | line number, counting from one               |
/// | `%c`   | column, counting from zero as Emacs does     |
/// | `%p`   | how far down the buffer point is             |
/// | `%*`   | `**` when modified, `--` when not            |
/// | `%%`   | a literal per cent                           |
///
/// An unknown escape is left as it was written rather than swallowed: a format
/// that silently loses a chunk of itself is harder to debug than one that
/// shows you the `%q` you meant to be something else.
pub fn expand_mode_line<B: BufferTrait>(format: &str, buffer: &Buffer<B>) -> String {
    let (line, column) = buffer.text.cursor_pos();
    let lines = buffer.text.line_count().max(1);
    let mut out = String::with_capacity(format.len() + 16);
    let mut chars = format.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('b') => out.push_str(&buffer.name),
            Some('f') => out.push_str(buffer.file_path.as_deref().unwrap_or(&buffer.name)),
            Some('m') => out.push_str(&buffer.current_mode),
            Some('l') => out.push_str(&(line + 1).to_string()),
            Some('c') => out.push_str(&column.to_string()),
            Some('p') => out.push_str(&position_in_buffer(line, lines)),
            Some('*') => out.push_str(if buffer.is_modified { "**" } else { "--" }),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Where point sits in the buffer, as Emacs words it: `All` when the whole
/// buffer is one screen's worth of lines, otherwise `Top`, `Bot` or a
/// percentage.
fn position_in_buffer(line: usize, lines: usize) -> String {
    if lines <= 1 {
        return "All".to_string();
    }
    if line == 0 {
        return "Top".to_string();
    }
    if line + 1 >= lines {
        return "Bot".to_string();
    }
    format!("{}%", (line * 100) / (lines - 1))
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
