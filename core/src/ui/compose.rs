//! Composing a window: from its buffer and its scroll to the rows, the
//! highlights, the gutter, the cursor and the status line a front end draws.
//!
//! Design: `doc/architettura-editor.typ, <composizione>`.
use crate::buffer::{BufferTrait, mark::region_bounds};
use crate::managers::Buffers;
use crate::text::rectangle::Rectangle;
use crate::text::search::Pattern;
use crate::ui::Face;
use crate::ui::gutter::{self, GutterCell, GutterSpec, LineNumbers, gutter_cells, gutter_columns};
use crate::ui::layout::{self, BufferHighlight, Layout, TabWidths};
use crate::ui::mode_line::expand_mode_line;
use crate::ui::visual_lines::{self, VisualLines, VisualPoint};
use crate::ui::width::{cells, cut};
use crate::ui::windows::WrappedWindow;
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
    /// The line numbers of a window whose buffer has them on.
    pub gutter: GutterSpec,
    /// How far a Tab reaches, mode by mode.
    pub tabs: &'a TabWidths,
    /// Effective fill columns where a vertical guide is enabled.
    pub fill_guide: Option<&'a super::FillColumnGuide>,
    /// The matches of the search or the replace in progress, if they are
    /// drawn.
    pub matches: Option<&'a SearchMarks>,
    /// The delimiters drawn in the focused window, if any.
    pub spots: Option<&'a SpotMarks>,
    /// How many lines a window scrolls to bring back a point that has left
    /// it, before it puts point's line in the middle instead: see
    /// [`follow_point`].
    pub scroll_conservatively: usize,
}

/// The name of the minor mode that draws the blanks at the ends of lines, in
/// [`Face::TRAILING_WHITESPACE`], in the buffers where it is on. Only a
/// buffer visiting a file draws them.
pub(crate) const WHITESPACE_MODE: &str = "whitespace-mode";

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

                // Point, the line count and whether the lines are numbered,
                // read under one acquisition of the buffer's lock: the gutter
                // and the scroll both need them.
                let (point_and_size, numbered) = buffers
                    .handle(&win.buffer_name)
                    .map(|buf| {
                        let buf = buf
                            .read()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let (line, col) = buf.text.cursor_pos();
                        (
                            Some((line, col, buf.text.cursor_pos_1d(), buf.text.line_count())),
                            buf.minor_mode_enabled(gutter::MODE),
                        )
                    })
                    .unwrap_or((None, false));
                // No buffer is numbered as an empty one.
                let (point_line, line_count) = point_and_size
                    .map(|(line, _, _, count)| (line, count))
                    .unwrap_or((0, 0));
                // The numbers Lisp asks for, where the buffer has its mode on.
                let gutter_spec = if numbered {
                    settings.gutter
                } else {
                    GutterSpec {
                        numbers: LineNumbers::Off,
                        ..settings.gutter
                    }
                };

                // The gutter comes out of the rectangle, as the status line
                // did: the scroll, the cursor and the highlights know only
                // the text's columns.
                // The minibuffer's gutter is its question, in place of the
                // line numbers: the answer starts after it, and a click is
                // counted from there as from the numbers.
                let gutter_width = match &win.prompt {
                    Some(prompt) => prompt_columns(&prompt.text, rect.width),
                    None => gutter_columns(gutter_spec, line_count, rect.width),
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
                let wrapping = buffers.handle(&win.buffer_name).is_some_and(|buf| {
                    buf.read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .minor_mode_enabled(visual_lines::MODE)
                });
                let unwrapped = !wrapping && win.wrap.take().is_some();
                let mut layout = if wrapping {
                    reconcile_wrapped(win, &rect, buffers, settings, follows_points, resized)
                } else {
                    compose_layout(win, &rect, buffers, settings.tabs)
                };
                if wrapping {
                    if is_focused && let Some((line, column, _, _)) = point_and_size {
                        cursor_rel_pos = layout
                            .row_of_position(line, column)
                            .map(|(index, row)| (row.to_screen(column), index));
                    }
                } else if let Some((c_line, c_col, c_offset, _)) = point_and_size
                    && follows_points
                {
                    // The view follows point only when point has moved, so
                    // that the wheel can scroll away from it, as in Emacs.
                    let point_moved = win.point != Some(c_offset) || unwrapped;
                    // What the window gives back when it is focused again.
                    win.point = Some(c_offset);

                    if point_moved || resized {
                        let scrolled_from = win.scroll_y;
                        // A window that only changed shape scrolls just
                        // enough: centring would move the text under a
                        // cursor that has not moved.
                        let conservatively = if point_moved {
                            settings.scroll_conservatively
                        } else {
                            usize::MAX
                        };
                        win.scroll_y =
                            follow_point(win.scroll_y, c_line, rect.height, conservatively);

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

                let mut lines = layout.visible_text(win.scroll_x, rect.width);
                // Later entries are drawn over earlier ones: see
                // `window_highlights`.
                let in_buffer = window_highlights(win, &rect, buffers, settings, is_focused);
                // In buffer columns until here; `place` maps, scrolls and cuts
                // them.
                let mut highlights =
                    hl_line_highlights(win, &layout, buffers, is_focused, rect.width);
                highlights.extend(layout::place(&layout, in_buffer, win.scroll_x, rect.width));
                // The virtual text's own faces last, over the colouring around
                // it.
                highlights.extend(layout.virtual_highlights(win.scroll_x, rect.width));
                if !win.is_prompt()
                    && let Some(guide) = settings.fill_guide
                {
                    guide.draw(
                        &win.buffer_name,
                        &rect,
                        win.scroll_x,
                        &mut lines,
                        &mut highlights,
                    );
                }

                // After the scroll, which decides the lines numbered.
                let gutter_cells = match &win.prompt {
                    Some(prompt) => {
                        prompt_cells(&prompt.text, gutter_width, win.scroll_y, rect.height)
                    }
                    None if wrapping && gutter_width > 0 => (0..rect.height)
                        .map(|index| {
                            if let Some(row) = layout.row(index)
                                && row.continuation == 0
                            {
                                gutter_cells(
                                    gutter_spec,
                                    gutter_width,
                                    row.line,
                                    1,
                                    line_count,
                                    point_line,
                                )
                                .remove(0)
                            } else {
                                GutterCell {
                                    text: " ".repeat(gutter_width),
                                    face: Face::LINE_NUMBER,
                                }
                            }
                        })
                        .collect(),
                    None => gutter_cells(
                        gutter_spec,
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

/// Reconciles a wrapped viewport in visual rows. `scroll_y` remains a logical
/// line; the continuation at its top belongs to this particular window.
fn reconcile_wrapped<B: BufferTrait>(
    win: &mut Window,
    rect: &Rect,
    buffers: &Buffers<B>,
    settings: ComposeSettings<'_>,
    follows: bool,
    resized: bool,
) -> Layout {
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Layout::default();
    };
    let buf = buf
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tab_width = settings.tabs.of(&buf.current_mode);
    let visual = VisualLines::new(&buf.text, &buf.virtual_text, rect.width, tab_width);
    let (line, column) = buf.text.cursor_pos();
    let offset = buf.text.cursor_pos_1d();
    let (point, _) = visual.position(line, column);
    let mut top = visual.normalize(VisualPoint {
        line: win.scroll_y,
        row: win.wrap.as_ref().map_or(0, |wrap| wrap.row),
    });
    let moved = win.point != Some(offset);
    let reflowed = win.wrap.as_ref().is_none_or(|old| {
        old.width != rect.width || old.tab_width != tab_width || old.version != buf.version
    });
    let mut layout = visual.compose(top, rect.height);
    let lost_point = win.wrap.as_ref().is_some_and(|old| {
        old.layout.row_of_position(line, column).is_some()
            && layout.row_of_position(line, column).is_none()
    });
    if follows {
        win.point = Some(offset);
        if rect.height > 0
            && (moved || resized || reflowed || lost_point)
            && layout.row_of_position(line, column).is_none()
        {
            let height = rect.height.saturating_sub(1).min(isize::MAX as usize) as isize;
            let bottom = visual.advance(top, height);
            let conservative = settings.scroll_conservatively.min(isize::MAX as usize) as isize;
            let close = if point < top {
                visual.advance(point, conservative) >= top
            } else {
                point <= visual.advance(bottom, conservative)
            };
            top = if !moved || settings.scroll_conservatively >= NEVER_RECENTER || close {
                if point < top {
                    point
                } else {
                    visual.advance(point, -height)
                }
            } else {
                visual.advance(point, -((height as f64 * 0.5).round() as isize))
            };
            layout = visual.compose(top, rect.height);
        }
    }
    win.scroll_y = top.line;
    win.scroll_x = 0;
    win.wrap = Some(WrappedWindow {
        row: top.row,
        layout: layout.clone(),
        width: rect.width,
        tab_width,
        version: buf.version,
    });
    layout
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

/// Above this, `scroll-conservatively` never centres, as in Emacs: the
/// window always scrolls just enough, however far point went.
pub const NEVER_RECENTER: usize = 100;

/// Returns the first line a window HEIGHT rows high, showing lines from
/// SCROLL_Y, shows once it has followed point to POINT_LINE.
///
/// Point still in view, nothing moves. Point out of it by no more than
/// CONSERVATIVELY lines -- `C-n` on the last row, `C-p` on the first -- the
/// window scrolls just enough to show it again, on the row at that edge.
/// Further -- a search, `goto-line`, a jump to an error -- point's line goes
/// in the middle, where `C-l` would put it, so that a jump does not leave the
/// cursor on the last row with nothing of what follows in sight. Emacs'
/// `scroll-conservatively`, which never centres above [`NEVER_RECENTER`].
pub fn follow_point(
    scroll_y: usize,
    point_line: usize,
    height: usize,
    conservatively: usize,
) -> usize {
    let height = height.max(1);
    let distance = if point_line < scroll_y {
        scroll_y - point_line
    } else if point_line >= scroll_y + height {
        point_line + 1 - (scroll_y + height)
    } else {
        return scroll_y;
    };
    if distance <= conservatively || conservatively > NEVER_RECENTER {
        if point_line < scroll_y {
            point_line
        } else {
            point_line + 1 - height
        }
    } else {
        // As `recenter` puts it: the rows above point are half of those
        // that are not point's.
        let above = ((height - 1) as f64 * 0.5).round() as usize;
        point_line.saturating_sub(above)
    }
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
    in_buffer.extend(trailing_highlights(win, rect, buffers, focused));
    in_buffer.extend(match_highlights(win, rect, buffers, settings.matches));
    in_buffer.extend(overlay_highlights(win, rect, buffers));
    in_buffer.extend(region_highlights(win, rect, buffers));
    in_buffer.extend(spot_highlights(win, rect, buffers, settings.spots));
    in_buffer
}

/// Returns the full visible rows of point's logical line in the focused window.
pub(crate) fn hl_line_highlights<B: BufferTrait>(
    win: &Window,
    layout: &Layout,
    buffers: &Buffers<B>,
    focused: bool,
    width: usize,
) -> Vec<Highlight> {
    if !focused || win.is_prompt() {
        return Vec::new();
    }
    let Some(handle) = buffers.handle(&win.buffer_name) else {
        return Vec::new();
    };
    let buf = handle
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !buf.minor_mode_enabled("hl-line-mode") {
        return Vec::new();
    }
    let line = buf.text.cursor_pos().0;
    layout
        .rows()
        .iter()
        .enumerate()
        .filter(|(_, row)| row.line == line)
        .map(|(row, _)| Highlight {
            row,
            start_col: 0,
            end_col: width,
            face: Face::HL_LINE,
        })
        .collect()
}

/// Returns the blanks, spaces and Tabs, that end the lines WIN shows in RECT,
/// in buffer columns, when its buffer visits a file and has
/// [`WHITESPACE_MODE`] on. In the window with the keyboard, the line point is
/// on does not draw them while point is among them, or at their end: what is
/// being typed there is not yet a mistake.
pub fn trailing_highlights<B: BufferTrait>(
    win: &Window,
    rect: &Rect,
    buffers: &Buffers<B>,
    focused: bool,
) -> Vec<BufferHighlight> {
    let Some(buf) = buffers.handle(&win.buffer_name) else {
        return Vec::new();
    };
    let buf = buf
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if buf.file_path.is_none() || !buf.minor_mode_enabled(WHITESPACE_MODE) {
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
    if buf.minor_mode_enabled(visual_lines::MODE) {
        return VisualLines::new(
            &buf.text,
            &buf.virtual_text,
            rect.width,
            tabs.of(&buf.current_mode),
        )
        .compose(
            VisualPoint {
                line: win.scroll_y,
                row: win.wrap.as_ref().map_or(0, |wrap| wrap.row),
            },
            rect.height,
        );
    }
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
