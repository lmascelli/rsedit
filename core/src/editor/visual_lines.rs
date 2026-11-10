//! Scrolling wrapped windows, in the same rows used for drawing and motion.
use super::*;
use crate::ui::WrappedWindow;
use crate::ui::visual_lines::{MODE, VisualLines, VisualPoint};

enum Change {
    Lines(isize),
    Pages(isize),
    Recenter(f64),
}

impl<B: BufferTrait> EditorState<B> {
    pub(crate) fn scroll_wrapped_window(
        &self,
        id: WindowId,
        amount: isize,
        page: bool,
    ) -> Option<bool> {
        self.change_wrapped_view(
            id,
            if page {
                Change::Pages(amount)
            } else {
                Change::Lines(amount)
            },
        )
    }

    pub(crate) fn recenter_wrapped_window(&self, id: WindowId, position: f64) -> Option<bool> {
        self.change_wrapped_view(id, Change::Recenter(position))
    }

    fn change_wrapped_view(&self, id: WindowId, change: Change) -> Option<bool> {
        let (win, focused) =
            self.windows(|windows| (windows.root().window(id).cloned(), windows.focused() == id));
        let win = win?;
        let width = win.rect.width.saturating_sub(win.gutter_width).max(1);
        let height = win.text_height.max(1);
        let tab_width = win
            .wrap
            .as_ref()
            .map_or(layout::DEFAULT_TAB_WIDTH, |wrap| wrap.tab_width);
        let result = self.with_buffer_mut(&win.buffer_name, |buf| {
            if !buf.minor_mode_enabled(MODE) {
                return None;
            }
            let visual = VisualLines::new(&buf.text, &buf.virtual_text, width, tab_width);
            let old = visual.normalize(VisualPoint {
                line: win.scroll_y,
                row: win.wrap.as_ref().map_or(0, |wrap| wrap.row),
            });
            let (line, column) = buf.text.cursor_pos();
            let (point, screen) = visual.position(line, column);
            let rows = height.saturating_sub(1).min(isize::MAX as usize) as isize;
            let top = match change {
                Change::Lines(delta) => visual.advance(old, delta),
                Change::Pages(amount) => {
                    let step = height.saturating_sub(2).max(1).min(isize::MAX as usize) as isize;
                    visual
                        .advance(old, amount.saturating_mul(step))
                        .min(visual.advance(visual.end(), -rows))
                }
                Change::Recenter(at) => visual.advance(
                    point,
                    -((rows as f64 * at.clamp(0.0, 1.0)).round() as isize),
                ),
            };
            let layout = visual.compose(top, height);
            let target = if focused && !matches!(change, Change::Recenter(_)) {
                let bottom = visual.advance(top, rows);
                let row = if point < top {
                    Some(top)
                } else if point > bottom {
                    Some(bottom)
                } else {
                    None
                };
                row.and_then(|at| {
                    visual
                        .line(at.line)
                        .get(at.row)
                        .map(|row| (at.line, row.to_buffer(screen)))
                })
            } else {
                None
            };
            if let Some((line, column)) = target {
                buf.text.cursor_move(line, column);
            }
            Some((
                top,
                layout,
                buf.version,
                buf.text.cursor_pos_1d(),
                old != top,
            ))
        })??;
        let (top, layout, version, point, changed) = result;
        self.windows_mut(|windows| {
            if let Some(win) = windows.root_mut().window_mut(id) {
                win.scroll_y = top.line;
                win.scroll_x = 0;
                win.wrap = Some(WrappedWindow {
                    row: top.row,
                    layout,
                    width,
                    tab_width,
                    version,
                });
                if focused {
                    win.point = Some(point);
                }
            }
        });
        Some(changed)
    }
}
