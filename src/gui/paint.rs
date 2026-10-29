//! Drawing a frame in the window: a grid of cells as the terminal has, the
//! faces' styles as egui draws them, and a view with its border, rows,
//! gutter, status line and cursor.
//!
//! Design: `doc/architettura-editor.typ, <gui>`.
use super::keys::mods;
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use rsedit_core::{Face, FrameSnapshot, MouseEvent, MouseKind, RenderableWindowView, Style};

/// The cells the window is drawn in: where the first is, how big one is, and
/// how many fit.
#[derive(Clone, Copy)]
pub(super) struct Grid {
    pub(super) origin: Pos2,
    pub(super) cell: Vec2,
    pub(super) columns: usize,
    pub(super) rows: usize,
}

impl Grid {
    /// Returns the grid of cells of size CELL that fit in AREA, at least one
    /// column and two rows.
    pub(super) fn new(area: Rect, cell: Vec2) -> Self {
        Self {
            origin: area.min,
            cell,
            columns: ((area.width() / cell.x).floor() as usize).clamp(1, 8192),
            rows: ((area.height() / cell.y).floor() as usize).clamp(2, 4096),
        }
    }

    /// Returns where cell (X, Y) begins.
    pub(super) fn at(self, x: isize, y: isize) -> Pos2 {
        self.origin + Vec2::new(x as f32 * self.cell.x, y as f32 * self.cell.y)
    }

    /// Returns the rectangle of WIDTH by HEIGHT cells from cell (X, Y).
    pub(super) fn rect(self, x: isize, y: isize, width: usize, height: usize) -> Rect {
        Rect::from_min_size(
            self.at(x, y),
            self.cell * Vec2::new(width as f32, height as f32),
        )
    }

    /// Returns the editor's mouse event KIND, with MODIFIERS, in the cell
    /// under AT.
    pub(super) fn mouse(self, at: Pos2, kind: MouseKind, modifiers: egui::Modifiers) -> MouseEvent {
        let offset = (at - self.origin) / self.cell;
        MouseEvent {
            column: offset.x.floor().clamp(0., u16::MAX as f32) as u16,
            row: offset.y.floor().clamp(0., u16::MAX as f32) as u16,
            kind,
            modifiers: mods(modifiers),
        }
    }
}

/// Returns COLOR as egui's.
fn rgb(color: rsedit_core::Color) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}

/// Returns the foreground and background STYLE draws in, DEFAULT's where it
/// sets none, a dark scheme's where neither does, swapped when it is reverse.
pub(super) fn colors(style: Style, default: Style) -> (Color32, Color32) {
    let mut foreground = style
        .fg
        .or(default.fg)
        .map(rgb)
        .unwrap_or(Color32::from_rgb(215, 220, 229));
    let mut background = style
        .bg
        .or(default.bg)
        .map(rgb)
        .unwrap_or(Color32::from_rgb(23, 26, 33));
    if style.reverse {
        std::mem::swap(&mut foreground, &mut background);
    }
    (foreground, background)
}

/// Adds OVERLAY to BASE: its colours where it sets them, and its attributes.
fn merge(base: &mut Style, overlay: Style) {
    if overlay.fg.is_some() {
        base.fg = overlay.fg;
    }
    if overlay.bg.is_some() {
        base.bg = overlay.bg;
    }
    base.bold |= overlay.bold;
    base.italic |= overlay.italic;
    base.underline |= overlay.underline;
    base.reverse |= overlay.reverse;
}

/// Draws TEXT at AT in STYLE; bold is the text drawn twice, a little apart.
fn label(
    painter: &egui::Painter,
    at: Pos2,
    text: &str,
    font: &FontId,
    style: Style,
    default: Style,
) {
    let (color, _) = colors(style, default);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.,
        egui::TextFormat {
            font_id: font.clone(),
            color,
            italics: style.italic,
            underline: if style.underline {
                Stroke::new(1_f32, color)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        },
    );
    let galley = painter.layout_job(job);
    painter.galley(at, galley.clone(), color);
    if style.bold {
        painter.galley(at + Vec2::new(0.35, 0.), galley, color);
    }
}

/// Draws TEXT in row Y of GRID, from column X, WIDTH cells, a character to a
/// cell, each in its style in STYLES or in DEFAULT.
pub(super) fn row(
    painter: &egui::Painter,
    grid: Grid,
    x: isize,
    y: isize,
    width: usize,
    text: &str,
    styles: &[Style],
    font: &FontId,
    default: Style,
) {
    // A character to a cell, as the editor counts columns, so that a click,
    // a highlight and the cursor fall on the same cells.
    let (_, base_background) = colors(default, default);
    painter.rect_filled(grid.rect(x, y, width, 1), 0., base_background);
    let mut chars = text.chars();
    for column in 0..width {
        let c = chars.next().unwrap_or(' ');
        let style = styles.get(column).copied().unwrap_or(default);
        let (_, background) = colors(style, default);
        let cell = grid.rect(x + column as isize, y, 1, 1);
        if background != base_background {
            painter.rect_filled(cell, 0., background);
        }
        if c != ' ' && !c.is_control() {
            let mut encoded = [0; 4];
            label(
                painter,
                cell.min,
                c.encode_utf8(&mut encoded),
                font,
                style,
                default,
            );
        }
    }
}

/// Draws VIEW of FRAME: its border and title, its rows with the highlights'
/// styles merged, its gutter, its status line and, when it has focus, the
/// cursor, a box when BLOCK and a bar otherwise, with PREEDIT after it.
/// Returns the cursor's rectangle, for the input method.
pub(super) fn paint_view(
    painter: &egui::Painter,
    grid: Grid,
    view: &RenderableWindowView,
    frame: &FrameSnapshot,
    font: &FontId,
    block: bool,
    preedit: &str,
) -> Option<Rect> {
    let rect = view.rect;
    let default = frame.theme.style(Face::DEFAULT);
    let (_, background) = colors(default, default);
    if view.has_border {
        let outer = grid.rect(rect.x - 1, rect.y - 1, rect.width + 2, rect.height + 2);
        painter.rect_filled(outer, 3., background);
        let (color, _) = colors(frame.theme.style(Face::WINDOW_SEPARATOR), default);
        painter.rect_stroke(
            outer.shrink(0.5),
            3.,
            Stroke::new(1_f32, color),
            egui::StrokeKind::Inside,
        );
        let title = view.title.as_deref().unwrap_or(&view.buffer_name);
        let title: String = title.chars().take(rect.width).collect();
        label(
            painter,
            grid.at(rect.x, rect.y - 1),
            &title,
            font,
            default,
            default,
        );
    }
    let text_area = grid
        .rect(rect.x, rect.y, rect.width, rect.height)
        .intersect(painter.clip_rect());
    let clipped = painter.with_clip_rect(text_area);
    clipped.rect_filled(text_area, 0., background);
    for line in 0..rect.height {
        let mut styles = vec![default; rect.width];
        for highlight in view
            .highlights
            .iter()
            .filter(|highlight| highlight.row == line)
        {
            let end = highlight.end_col.min(styles.len());
            let start = highlight.start_col.min(end);
            for style in &mut styles[start..end] {
                merge(style, frame.theme.style(highlight.face));
            }
        }
        row(
            &clipped,
            grid,
            rect.x,
            rect.y + line as isize,
            rect.width,
            view.lines.get(line).map(String::as_str).unwrap_or(""),
            &styles,
            font,
            default,
        );
    }
    for (line, cell) in view.gutter.iter().enumerate() {
        let style = frame.theme.style(cell.face);
        row(
            painter,
            grid,
            rect.x - view.gutter_width as isize,
            rect.y + line as isize,
            view.gutter_width,
            &cell.text,
            &vec![style; view.gutter_width],
            font,
            default,
        );
    }
    if let Some(text) = &view.mode_line {
        let style = frame.theme.style(if view.is_focused {
            Face::MODE_LINE
        } else {
            Face::MODE_LINE_INACTIVE
        });
        let width = rect.width + view.gutter_width;
        row(
            painter,
            grid,
            rect.x - view.gutter_width as isize,
            rect.y + rect.height as isize,
            width,
            text,
            &vec![style; width],
            font,
            default,
        );
    }
    if view.is_focused {
        if let Some((column, line)) = view.cursor_rel_pos {
            let caret = grid.rect(rect.x + column as isize, rect.y + line as isize, 1, 1);
            let (color, _) = colors(default, default);
            if block {
                clipped.rect_stroke(
                    caret.shrink(0.5),
                    0.,
                    Stroke::new(1.5_f32, color),
                    egui::StrokeKind::Inside,
                );
            } else {
                clipped.line_segment(
                    [caret.left_top(), caret.left_bottom()],
                    Stroke::new(1.5_f32, color),
                );
            }
            if !preedit.is_empty() {
                let composition = Rect::from_min_size(
                    caret.min,
                    Vec2::new(
                        (preedit.chars().count() + 1) as f32 * grid.cell.x,
                        grid.cell.y,
                    ),
                );
                clipped.rect_filled(composition, 0., background);
                let style = Style {
                    underline: true,
                    ..default
                };
                label(&clipped, caret.min, preedit, font, style, default);
            }
            return Some(caret);
        }
    }
    None
}
