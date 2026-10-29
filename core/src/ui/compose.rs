//! Composing a window: from its buffer and its scroll to the rows, the
//! highlights, the gutter, the cursor and the status line a front end draws.
//!
//! Design: `doc/architettura-editor.typ, <composizione>`.
use crate::buffer::{BufferTrait, mark::region_bounds};
use crate::managers::Buffers;
use crate::text::rectangle::Rectangle;
use crate::ui::Face;
use crate::ui::gutter::{GutterCell, GutterSpec, gutter_cells, gutter_columns};
use crate::ui::layout::{self, BufferHighlight, Layout};
use crate::ui::mode_line::expand_mode_line;
use crate::ui::windows::{Focus, LayoutNode, Rect, Window, split_rects};

/// A run of a drawn row to draw in a face of its own, in screen coordinates
/// within the window, scrolled and clipped already.
#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
    /// The row within the window, from 0.
    pub row: usize,
    /// The columns within the row, in characters, the end excluded.
    pub start_col: usize,
    pub end_col: usize,
    pub face: Face,
}

/// A rule between windows side by side: where, drawn with which character,
/// in which face.
#[derive(Clone, Debug, PartialEq)]
pub struct Separator {
    pub rect: Rect,
    pub ch: char,
    pub face: Face,
}

/// What Lisp says about how windows are drawn, read once per frame before any
/// lock is taken, and carried down the tree as one argument.
#[derive(Clone, Copy, Debug)]
pub struct ComposeSettings<'a> {
    pub mode_line_format: &'a str,
    pub gutter: GutterSpec,
}

/// One window, as it is drawn: owned data, which outlives the locks it was
/// composed under. `Default` is a window of nothing, nowhere, for the tests.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RenderableWindowView {
    /// Where the text goes: the status line, the border and the gutter are
    /// drawn around it.
    pub rect: Rect,
    pub buffer_name: String,
    /// What its border is labelled with, when it has one; the buffer's name
    /// otherwise.
    pub title: Option<String>,
    pub is_focused: bool,
    /// Where the cursor is, from the rectangle's corner, when it is drawn.
    pub cursor_rel_pos: Option<(usize, usize)>,
    /// The text of each row, scrolled and cut to the rectangle.
    pub lines: Vec<String>,
    /// The status line, drawn on the row below the rectangle; `None` for a
    /// window with no room for one, and for a floating window.
    pub mode_line: Option<String>,
    /// Runs of `lines` drawn in a face other than the default, later ones
    /// over earlier ones.
    pub highlights: Vec<Highlight>,
    pub has_border: bool,
    /// The gutter, a cell for each row, drawn in the `gutter_width` columns
    /// left of the rectangle; empty when there is none.
    pub gutter: Vec<GutterCell>,
    /// How many columns [`Self::gutter`] takes, zero for none: stated, since
    /// the status line spans it too and a window with no rows has no cell.
    pub gutter_width: usize,
}

impl LayoutNode {
    /// Composes every window of this tree, drawn in RECT, into OUT_VIEWS, in
    /// layout order, and the rules between them into OUT_SEPARATORS; FOCUS
    /// says which window has the keyboard, SETTINGS what Lisp asks for.
    ///
    /// Each window's scroll follows point when point has moved, and what the
    /// window was composed into is recorded on it: see [`Window`].
    pub fn compute_tiled_views<B: BufferTrait>(
        &mut self,
        rect: Rect,
        focus: Focus,
        buffers: &Buffers<B>,
        settings: ComposeSettings<'_>,
        out_views: &mut Vec<RenderableWindowView>,
        out_separators: &mut Vec<Rect>,
    ) {
        match self {
            LayoutNode::Leaf(win) => {
                let is_focused = win.id == focus.id;
                // A window that changed shape follows point again, or a resize
                // could leave the cursor off its bottom.
                let resized = win.rect != rect;
                // The whole window, status line included: what a click is
                // tested against.
                win.rect = rect;

                // The status line takes the bottom row, before anything reads
                // the rectangle; a window of one row keeps it for text.
                let mode_line = buffers.handle(&win.buffer_name).and_then(|buffer| {
                    (win.show_mode_line && rect.height >= 2).then(|| {
                        expand_mode_line(
                            settings.mode_line_format,
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

                // The rows of text, without the status line: a screenful.
                win.text_height = rect.height;

                let mut cursor_rel_pos = None;

                // The focused window follows point, and every tiled one does
                // while the keyboard is in a floating window, as during an
                // incremental search; only the focused one draws the cursor.

                let follows_points = is_focused || !focus.tiled;

                // Point and the line count, read under one acquisition of the
                // buffer's lock: the gutter and the scroll both need them.
                let point_and_size = buffers.handle(&win.buffer_name).map(|buf| {
                    let buf = buf.read().expect("Failed to acquire read lock on buffer");
                    let (line, col) = buf.text.cursor_pos();
                    (line, col, buf.text.cursor_pos_1d(), buf.text.line_count())
                });
                // No buffer is numbered as an empty one.
                let (point_line, line_count) = point_and_size
                    .map(|(line, _, _, count)| (line, count))
                    .unwrap_or((0, 0));

                // The gutter comes out of the rectangle, as the status line
                // did: the scroll, the cursor and the highlights know only
                // the text's columns.
                let gutter_width = gutter_columns(settings.gutter, line_count, rect.width);
                win.gutter_width = gutter_width;
                let rect = Rect {
                    x: rect.x + gutter_width as isize,
                    width: rect.width.saturating_sub(gutter_width),
                    ..rect
                };

                // Composed before the scroll follows point, since what must be
                // visible is the cursor's screen column, which the row knows;
                // composed again if the scroll moves.
                let mut layout = compose_layout(win, &rect, buffers);
                if let Some((c_line, c_col, c_offset, _)) = point_and_size
                    && follows_points
                {
                    // The view follows point only when point has moved, so
                    // that the wheel can scroll away from it, as in Emacs.
                    let point_moved = win.point != Some(c_offset);
                    // What the window gives back when it is focused again.
                    win.point = Some(c_offset);

                    if point_moved || resized {
                        let scrolled_from = win.scroll_y;
                        if c_line < win.scroll_y {
                            win.scroll_y = c_line;
                        } else if c_line >= win.scroll_y + rect.height {
                            win.scroll_y = c_line - rect.height + 1;
                        }

                        // Other lines, so other rows: composed again.
                        if win.scroll_y != scrolled_from {
                            layout = compose_layout(win, &rect, buffers);
                        }

                        // The screen column, which virtual text makes differ
                        // from the buffer's.
                        let c_screen = layout
                            .row_of_line(c_line)
                            .map(|(_, row)| row.to_screen(c_col))
                            .unwrap_or(c_col);
                        if c_screen < win.scroll_x {
                            win.scroll_x = c_screen;
                        } else if c_screen >= win.scroll_x + rect.width {
                            win.scroll_x = c_screen - rect.width + 1;
                        }
                    }

                    if is_focused {
                        let c_screen = layout
                            .row_of_line(c_line)
                            .map(|(_, row)| row.to_screen(c_col))
                            .unwrap_or(c_col);
                        cursor_rel_pos = Some((
                            c_screen.saturating_sub(win.scroll_x),
                            c_line.saturating_sub(win.scroll_y),
                        ));
                    }
                }

                let lines = layout.visible_text(win.scroll_x, rect.width);
                // Colouring, then overlays, then the region: later entries are
                // drawn over earlier ones.
                let mut in_buffer = syntax_highlights(win, &rect, buffers);
                in_buffer.extend(overlay_highlights(win, &rect, buffers));
                in_buffer.extend(region_highlights(win, &rect, buffers));
                // In buffer columns until here; `place` maps, scrolls and cuts
                // them.
                let mut highlights = layout::place(&layout, in_buffer, win.scroll_x, rect.width);
                // The virtual text's own faces last, over the colouring around
                // it.
                highlights.extend(layout.virtual_highlights(win.scroll_x, rect.width));

                // After the scroll, which decides the lines numbered.
                let gutter_cells = gutter_cells(
                    settings.gutter,
                    gutter_width,
                    win.scroll_y,
                    rect.height,
                    line_count,
                    point_line,
                );

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
                    gutter: gutter_cells,
                    gutter_width,
                });
            }

            LayoutNode::Split {
                orientation,
                division,
                left,
                right,
            } => {
                let (first, second, rule) = split_rects(orientation, division, rect);
                // Before the children, so that the rules come out in the order
                // they are found.
                if let Some(rule) = rule {
                    out_separators.push(rule);
                }
                left.compute_tiled_views(
                    first,
                    focus,
                    buffers,
                    settings,
                    out_views,
                    out_separators,
                );
                right.compute_tiled_views(
                    second,
                    focus,
                    buffers,
                    settings,
                    out_views,
                    out_separators,
                );
            }
        }
    }
}

/// Returns the overlays of WIN's buffer on the lines WIN shows in RECT, in
/// buffer columns, a span for each line: the table is asked once for what
/// overlaps the lines on screen.
pub fn overlay_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
) -> Vec<BufferHighlight> {
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Vec::new();
    };
    let buf = buf
        .read()
        .expect("Failed to acquire read lock on buffer for highlighting");
    if buf.overlays.is_empty() {
        // What almost every buffer has.
        return Vec::new();
    }

    // The characters on screen, as one span, to the end of the buffer.
    let first_line = win.scroll_y;
    let last_line = (win.scroll_y + rect.height).min(buf.text.line_count());
    if first_line >= last_line {
        return Vec::new();
    }
    let visible_from = buf.text.cursor_2d_to_1d(first_line, 0);
    let visible_to = buf.text.len();

    let mut highlights = Vec::new();
    for overlay in buf.overlays.overlapping(visible_from, visible_to) {
        let (start_line, start_col) = buf.text.cursor_1d_to_2d(overlay.start);
        let (end_line, end_col) = buf.text.cursor_1d_to_2d(overlay.end);
        for line in start_line.max(first_line)..=end_line.min(last_line.saturating_sub(1)) {
            // Only the first and the last line are partial.
            let from = if line == start_line { start_col } else { 0 };
            let to = if line == end_line {
                end_col
            } else {
                // Up to the newline, included.
                line_width(&buf.text, line) + 1
            };
            highlights.push(BufferHighlight {
                line,
                start: from,
                end: to,
                face: overlay.face,
            });
        }
    }
    highlights
}

/// Returns the active region of WIN's buffer on the lines WIN shows in RECT,
/// in buffer columns, a span for each line; nothing when there is no active
/// region. A rectangle's span is its columns on every line.
pub fn region_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
) -> Vec<BufferHighlight> {
    let Some(buf) = buffers.handle(&win.buffer_name) else {
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

    // A rectangle is the same columns on every line, as the commands take
    // it, and nothing on a line too short to reach them.
    let rectangle = buf
        .mark
        .filter(|mark| mark.active && mark.rectangle)
        .map(|mark| {
            let point = buf.text.cursor_pos_1d();
            Rectangle::between_offsets(&buf.text, mark.at, point, mark.inclusive)
        });
    let (start_line, end_line) = rectangle
        .map(|rectangle| (rectangle.top, rectangle.bottom))
        .unwrap_or((start_line, end_line));

    for line in start_line.max(first_visible)..=end_line.min(last_visible.saturating_sub(1)) {
        let (from, to): (usize, usize) = if let Some(rectangle) = rectangle {
            // Clamped to the line.
            let width = line_width(&buf.text, line);
            (rectangle.left.min(width), rectangle.right.min(width))
        } else {
            // Only the first and the last line are partial.
            let from = if line == start_line { start_col } else { 0 };
            let to = if line == end_line {
                end_col
            } else {
                // Up to the newline, included, so that it shows as selected.
                line_width(&buf.text, line) + 1
            };
            (from, to)
        };

        highlights.push(BufferHighlight {
            line,
            start: from,
            end: to,
            face: Face::REGION,
        });
    }
    highlights
}

/// Returns how many characters LINE has, without its newline.
fn line_width<B: BufferTrait>(text: &B, line: usize) -> usize {
    text.get_lines(line, line + 1)
        .first()
        .map(|l| l.chars().count())
        .unwrap_or(0)
}

/// Returns the rows WIN shows in RECT, composed from its buffer and the
/// virtual text in it; none when the buffer has gone, so that the window
/// draws blank.
pub fn compose_layout<B: BufferTrait>(win: &Window, rect: &Rect, buffers: &Buffers<B>) -> Layout {
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Layout::default();
    };
    let buf = buf
        .read()
        .expect("Failed to acquire read lock on buffer for composing");
    Layout::compose(&buf.text, &buf.virtual_text, win.scroll_y, rect.height)
}

/// Returns the colouring of WIN's buffer on the lines WIN shows in RECT, in
/// buffer columns. Read from the cache: a line not coloured yet has none, and
/// composing never colours.
pub fn syntax_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
) -> Vec<BufferHighlight> {
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Vec::new();
    };
    let buf = buf
        .read()
        .expect("Failed to acquire read lock on buffer for highlighting");

    let mut highlights = Vec::new();
    for row in 0..rect.height {
        let line = win.scroll_y + row;
        for span in buf.syntax.spans(line) {
            highlights.push(BufferHighlight {
                line,
                start: span.start,
                end: span.end,
                face: span.face,
            });
        }
    }
    highlights
}
