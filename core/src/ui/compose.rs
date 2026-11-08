//! Composing a window: from its buffer and its scroll to the rows, the
//! highlights, the gutter, the cursor and the status line a front end draws.
//!
//! Design: `doc/architettura-editor.typ, <composizione>`.
use crate::buffer::{BufferTrait, mark::region_bounds};
use crate::managers::Buffers;
use crate::text::rectangle::Rectangle;
use crate::text::search::Pattern;
use crate::ui::Face;
use crate::ui::gutter::{GutterCell, GutterSpec, gutter_cells, gutter_columns};
use crate::ui::layout::{self, BufferHighlight, Layout, TabWidths};
use crate::ui::mode_line::expand_mode_line;
use crate::ui::width::{cells, cut};
use crate::ui::windows::{Focus, LayoutNode, Rect, Window, WindowId, split_rects};

/// How many matches one window draws at most: a screenful of a pattern that
/// matches every character is still a screenful, but the scan stops.
pub const MATCHES_PER_WINDOW: usize = 2000;

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
    /// How far a Tab reaches, mode by mode.
    pub tabs: &'a TabWidths,
    /// The matches of the search or the replace in progress, if they are
    /// drawn.
    pub matches: Option<&'a SearchMarks>,
    /// The delimiters drawn in the focused window, if any.
    pub spots: Option<&'a SpotMarks>,
    /// Where the blanks at the ends of lines are drawn.
    pub trailing: &'a TrailingSpaces,
}

/// Whether the blanks at the ends of lines are drawn, in
/// [`Face::TRAILING_WHITESPACE`], mode by mode: what Lisp says, read before
/// any lock. Only a buffer visiting a file draws them.
#[derive(Clone, Debug, Default)]
pub struct TrailingSpaces {
    /// What a mode that says nothing of its own has.
    pub default: bool,
    /// The modes that say something of their own.
    pub by_mode: std::collections::HashMap<String, bool>,
}

impl TrailingSpaces {
    /// Returns whether the buffers of MODE draw them.
    pub fn of(&self, mode: &str) -> bool {
        self.by_mode.get(mode).copied().unwrap_or(self.default)
    }
}

/// The matches of a search or a replace in progress: drawn, in
/// [`Face::LAZY_HIGHLIGHT`], on the lines each window showing BUFFER shows,
/// and looked for nowhere else.
#[derive(Clone, Debug)]
pub struct SearchMarks {
    /// The buffer searched.
    pub buffer: String,
    pub pattern: Pattern,
    /// The characters a match must lie within: a replace's region, or the
    /// whole buffer.
    pub from: usize,
    pub to: usize,
}

/// Characters drawn in a face of their own in one window, each at its offset:
/// the delimiters `show-paren-mode` pairs.
#[derive(Clone, Debug, PartialEq)]
pub struct SpotMarks {
    pub window: WindowId,
    pub spots: Vec<(usize, Face)>,
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
                        // The rows of text it leaves, which `%p` asks whether
                        // the whole buffer fits in.
                        expand_mode_line(
                            settings.mode_line_format,
                            &buffer
                                .read()
                                .unwrap_or_else(std::sync::PoisonError::into_inner),
                            rect.height - 1,
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
                    let buf = buf
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                // The minibuffer's gutter is its question, in place of the
                // line numbers: the answer starts after it, and a click is
                // counted from there as from the numbers.
                let gutter_width = match &win.prompt {
                    Some(prompt) => prompt_columns(&prompt.text, rect.width),
                    None => gutter_columns(settings.gutter, line_count, rect.width),
                };

                win.gutter_width = gutter_width;
                let rect = Rect {
                    x: rect.x + gutter_width as isize,
                    width: rect.width.saturating_sub(gutter_width),
                    ..rect
                };

                // Composed before the scroll follows point, since what must be
                // visible is the cursor's screen column, which the row knows;
                // composed again if the scroll moves.
                let mut layout = compose_layout(win, &rect, buffers, settings.tabs);
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
                            layout = compose_layout(win, &rect, buffers, settings.tabs);
                        }

                        // The cells of point's character, which virtual text,
                        // a Tab or a wide character make differ from its
                        // column: all of them are to be seen.
                        let (c_screen, c_cells) = layout
                            .row_of_line(c_line)
                            .map(|(_, row)| (row.to_screen(c_col), row.cells_at(c_col)))
                            .unwrap_or((c_col, 1));
                        if c_screen < win.scroll_x {
                            win.scroll_x = c_screen;
                        } else if c_screen + c_cells > win.scroll_x + rect.width {
                            win.scroll_x = (c_screen + c_cells).saturating_sub(rect.width);
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
                // Later entries are drawn over earlier ones: see
                // `window_highlights`.
                let in_buffer = window_highlights(win, &rect, buffers, settings, is_focused);
                // In buffer columns until here; `place` maps, scrolls and cuts
                // them.
                let mut highlights = layout::place(&layout, in_buffer, win.scroll_x, rect.width);
                // The virtual text's own faces last, over the colouring around
                // it.
                highlights.extend(layout.virtual_highlights(win.scroll_x, rect.width));

                // After the scroll, which decides the lines numbered.
                let gutter_cells = match &win.prompt {
                    Some(prompt) => {
                        prompt_cells(&prompt.text, gutter_width, win.scroll_y, rect.height)
                    }
                    None => gutter_cells(
                        settings.gutter,
                        gutter_width,
                        win.scroll_y,
                        rect.height,
                        line_count,
                        point_line,
                    ),
                };

                out_views.push(RenderableWindowView {
                    rect,
                    buffer_name: win.buffer_name.clone(),
                    title: win.prompt.as_ref().map(|prompt| prompt.text.clone()),
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

/// Returns how many columns the question PROMPT takes in front of the answer
/// in a minibuffer window WIDTH columns wide: all of it, and one column more
/// when it does not end in a blank, but never so many that no column is left
/// for the answer.
fn prompt_columns(prompt: &str, width: usize) -> usize {
    let wanted = cells(prompt) + usize::from(!prompt.ends_with(char::is_whitespace));
    wanted.min(width.saturating_sub(1))
}

/// Returns the gutter of a minibuffer window: PROMPT on the row of the first
/// line, when the view starts there, and blanks on the others, WIDTH columns
/// each, for ROWS rows from FIRST_LINE.
fn prompt_cells(prompt: &str, width: usize, first_line: usize, rows: usize) -> Vec<GutterCell> {
    if width == 0 {
        return Vec::new();
    }
    (0..rows)
        .map(|row| {
            let text = if first_line + row == 0 {
                let shown = cut(prompt, 0, width);
                let pad = width.saturating_sub(cells(&shown));
                format!("{shown}{}", " ".repeat(pad))
            } else {
                " ".repeat(width)
            };
            GutterCell {
                text,
                face: Face::MINIBUFFER_PROMPT,
            }
        })
        .collect()
}

/// Returns what is drawn over WIN's text in RECT, in buffer columns, in the
/// order it is drawn, each over the one before: the colouring, the blanks at
/// the ends of lines, the matches of a search, the overlays, the region, and
/// the delimiters paired. A buffer looks the same whichever kind of window
/// shows it, tiled or floating; FOCUSED says whether WIN has the keyboard.
pub fn window_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
    settings: ComposeSettings<'_>,
    focused: bool,
) -> Vec<BufferHighlight> {
    let mut in_buffer = syntax_highlights(win, rect, buffers);
    in_buffer.extend(trailing_highlights(
        win,
        rect,
        buffers,
        settings.trailing,
        focused,
    ));
    in_buffer.extend(match_highlights(win, rect, buffers, settings.matches));
    in_buffer.extend(overlay_highlights(win, rect, buffers));
    in_buffer.extend(region_highlights(win, rect, buffers));
    in_buffer.extend(spot_highlights(win, rect, buffers, settings.spots));
    in_buffer
}

/// Returns the blanks, spaces and Tabs, that end the lines WIN shows in RECT,
/// in buffer columns, when its buffer visits a file and TRAILING draws them
/// in its mode. In the window with the keyboard, the line point is on does
/// not draw them while point is among them, or at their end: what is being
/// typed there is not yet a mistake.
pub fn trailing_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
    trailing: &TrailingSpaces,
    focused: bool,
) -> Vec<BufferHighlight> {
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Vec::new();
    };
    let buf = buf
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if buf.file_path.is_none() || !trailing.of(&buf.current_mode) {
        return Vec::new();
    }
    let first = win.scroll_y;
    let last = (win.scroll_y + rect.height).min(buf.text.line_count());
    if first >= last {
        return Vec::new();
    }
    let (point_line, point_column) = buf.text.cursor_pos();
    let mut highlights = Vec::new();
    for (index, text) in buf.text.get_lines(first, last).iter().enumerate() {
        let line = first + index;
        let length = text.chars().count();
        let kept = text.trim_end_matches([' ', '\t']).chars().count();
        if kept == length || (focused && line == point_line && point_column >= kept) {
            continue;
        }
        highlights.push(BufferHighlight {
            line,
            start: kept,
            end: length,
            face: Face::TRAILING_WHITESPACE,
        });
    }
    highlights
}

/// Returns the spans from START to END of TEXT on the lines FIRST_LINE to
/// LAST_LINE, the last excluded: one per line, a line the span crosses taken
/// up to its newline, included.
fn span_highlights<B: BufferTrait>(
    text: &B,
    (start, end): (usize, usize),
    (first_line, last_line): (usize, usize),
    face: Face,
) -> Vec<BufferHighlight> {
    let (start_line, start_col) = text.cursor_1d_to_2d(start);
    let (end_line, end_col) = text.cursor_1d_to_2d(end);
    let mut highlights = Vec::new();
    for line in start_line.max(first_line)..=end_line.min(last_line.saturating_sub(1)) {
        // Only the first and the last line are partial.
        let from = if line == start_line { start_col } else { 0 };
        let to = if line == end_line {
            end_col
        } else {
            line_width(text, line) + 1
        };
        highlights.push(BufferHighlight {
            line,
            start: from,
            end: to,
            face,
        });
    }
    highlights
}

/// Returns the matches of MARKS on the lines WIN shows in RECT, in buffer
/// columns; nothing when MARKS searches another buffer.
///
/// Only the lines on screen are searched, whole, so that `^` and `$` mean
/// what they mean to the search, and a match is drawn when it lies within the
/// characters MARKS allows. An empty match is not drawn: it has no character
/// to draw.
pub fn match_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
    marks: Option<&SearchMarks>,
) -> Vec<BufferHighlight> {
    let Some(marks) = marks.filter(|marks| marks.buffer == win.buffer_name) else {
        return Vec::new();
    };
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Vec::new();
    };
    let buf = buf
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = &buf.text;
    let first_line = win.scroll_y;
    let last_line = (win.scroll_y + rect.height).min(text.line_count());
    if first_line >= last_line {
        return Vec::new();
    }
    let visible_from = text.cursor_2d_to_1d(first_line, 0);
    let visible_to = if last_line < text.line_count() {
        text.cursor_2d_to_1d(last_line, 0)
    } else {
        text.len()
    };
    let haystack = text.slice(visible_from, visible_to);
    let mut highlights = Vec::new();
    for found in marks.pattern.scan(&haystack, MATCHES_PER_WINDOW).found {
        let (start, end) = (visible_from + found.start, visible_from + found.end);
        if start == end || start < marks.from || end > marks.to {
            continue;
        }
        highlights.extend(span_highlights(
            text,
            (start, end),
            (first_line, last_line),
            Face::LAZY_HIGHLIGHT,
        ));
    }
    highlights
}

/// Returns the characters MARKS names for WIN, on the lines WIN shows in RECT,
/// in buffer columns; nothing for any other window.
pub fn spot_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
    marks: Option<&SpotMarks>,
) -> Vec<BufferHighlight> {
    let Some(marks) = marks.filter(|marks| marks.window == win.id) else {
        return Vec::new();
    };
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Vec::new();
    };
    let buf = buf
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let visible = win.scroll_y..win.scroll_y + rect.height;
    marks
        .spots
        .iter()
        .filter(|(at, _)| *at < buf.text.len())
        .map(|&(at, face)| (buf.text.cursor_1d_to_2d(at), face))
        .filter(|((line, _), _)| visible.contains(line))
        .map(|((line, column), face)| BufferHighlight {
            line,
            start: column,
            end: column + 1,
            face,
        })
        .collect()
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
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        highlights.extend(span_highlights(
            &buf.text,
            (overlay.start, overlay.end),
            (first_line, last_line),
            overlay.face,
        ));
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
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
pub fn compose_layout<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
    tabs: &TabWidths,
) -> Layout {
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Layout::default();
    };
    let buf = buf
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Layout::compose(
        &buf.text,
        &buf.virtual_text,
        win.scroll_y,
        rect.height,
        tabs.of(&buf.current_mode),
    )
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
        .unwrap_or_else(std::sync::PoisonError::into_inner);

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
