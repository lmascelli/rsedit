//! Turning pointer events into commands. Where a click landed is worked out
//! here, against the layout and each window's scroll; what it means is a
//! command, which Lisp can bind to something else.
//!
//! Design: `doc/architettura-editor.typ, <mouse>`.

use super::*;
use crate::ui::layout::Layout;

/// Returns the command form NAME with the numbers ARGS, built rather than
/// parsed: the numbers have just been worked out, and the parser would only
/// read back the text they were written into.
fn mouse_form<B: BufferTrait>(name: &str, args: &[f64]) -> ELispExp<B> {
    let mut items = vec![ELispExp::symbol(name.into())];
    items.extend(args.iter().copied().map(ELispExp::number));
    ELispExp::form(items)
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns where in WINDOW's buffer the cell (X, Y) is, clamped to what
    /// the window shows: a drag that leaves the window selects to its edge.
    /// Scrolling after the pointer is [`Self::drag_scroll_tick`]'s.
    fn position_in(&self, window: WindowId, x: isize, y: isize) -> Option<(usize, usize)> {
        let (line, screen_column) = self.windows(|windows| windows.position_in(window, x, y))?;
        Some((line, self.buffer_column(window, line, screen_column)))
    }

    /// Returns the buffer column that SCREEN_COLUMN is on, for LINE in WINDOW.
    ///
    /// The two differ wherever the row shows what the buffer does not hold --
    /// an inlay hint, a preview. The compartment that knows the geometry
    /// cannot reach the buffer, and the buffer does not know the geometry, so
    /// the translation is here, which reaches both. One row is composed, since
    /// a click is on one row; the windows are let go before the buffer is
    /// taken.
    fn buffer_column(&self, window: WindowId, line: usize, screen_column: usize) -> usize {
        let Some(name) = self.windows(|windows| windows.buffer_of(window)) else {
            return screen_column;
        };
        self.with_buffer(&name, |buf| {
            let layout = Layout::compose(&buf.text, &buf.virtual_text, line, 1);
            layout
                .row(0)
                .map(|row| row.to_buffer(screen_column))
                .unwrap_or(screen_column)
        })
        .unwrap_or(screen_column)
    }

    /// Scrolls a drag that has left its window by one line, and moves the
    /// selection with it; false when there was nothing to do.
    ///
    /// One line a turn, however far outside the pointer is: a selection that
    /// ran faster the further the hand strayed would be hard to stop where it
    /// was meant to, and a turn is short enough for the scroll to look smooth.
    pub fn drag_scroll_tick(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        let Some(MouseDrag::Text { window, at: (x, y) }) = self.mouse_drag() else {
            return false;
        };
        let Some(step) = self.drag_scroll_step(window, y) else {
            return false;
        };
        if !self.scroll_window_by(window, step) {
            // As far as the buffer goes, and the selection is there already.
            return false;
        }
        let Some((line, column)) = self.position_in(window, x, y) else {
            return false;
        };
        self.run_command_form(
            mouse_form(
                "mouse-drag-to",
                &[window.0 as f64, line as f64, column as f64],
            ),
            env,
        );
        true
    }

    /// Takes one more event, at (X, Y), of the drag in progress, and returns
    /// whether it ran a command.
    fn continue_drag(&self, x: isize, y: isize, env: &Arc<Env<EditorState<B>>>) -> bool {
        let form = match self.mouse_drag() {
            Some(MouseDrag::Text { window, .. }) => {
                self.set_mouse_drag(Some(MouseDrag::Text { window, at: (x, y) }));
                // Against the window the drag began in, not the one under the
                // pointer: a selection does not change buffers half-way.
                self.position_in(window, x, y).map(|(line, column)| {
                    mouse_form(
                        "mouse-drag-to",
                        &[window.0 as f64, line as f64, column as f64],
                    )
                })
            }
            Some(MouseDrag::Divider {
                path,
                orientation,
                last,
            }) => {
                // How far the pointer has come since the last event, along the
                // axis the boundary moves on. A running position, so that a
                // boundary that could not move as far as the pointer does not
                // lag behind it for the rest of the drag.
                let now = match orientation {
                    Orientation::Horizontal => y,
                    Orientation::Vertical => x,
                };
                self.set_mouse_drag(Some(MouseDrag::Divider {
                    path,
                    orientation,
                    last: now,
                }));
                (now != last).then(|| mouse_form("mouse-resize", &[(now - last) as f64]))
            }
            None => None,
        };
        let Some(form) = form else {
            return false;
        };
        self.run_command_form(form, env);
        true
    }

    /// Acts on a mouse event from the host, and returns whether it did
    /// anything: a terminal reporting the mouse sends motion by the dozen a
    /// second, and the host redraws only for an event that changed something.
    ///
    /// Where the event landed is worked out here. What it means is a command,
    /// run as a key's is: one undo step, one `post-command-hook`, a name that
    /// `M-x` and the log know, and a binding Lisp can replace.
    pub fn handle_mouse_event(&self, event: MouseEvent, env: &Arc<Env<EditorState<B>>>) -> bool {
        // Asked here and not only where the terminal turns the mouse on, so
        // that the setting means the same to every host: a GUI that always
        // delivers the mouse obeys it too.
        if !mouse_mode(env) {
            return false;
        }
        let (x, y) = (event.column as isize, event.row as isize);

        // A drag is answered before what is under the pointer is asked: a drag
        // is about where it began, and a few rows on the pointer is often over
        // another window, or off the frame, where the hit test finds nothing.
        match event.kind {
            MouseKind::Drag(MouseButton::Left) => return self.continue_drag(x, y, env),
            MouseKind::Up(MouseButton::Left) => {
                self.take_mouse_drag();
                return false;
            }
            _ => (),
        }

        let Some(hit) = self.hit_test(x, y) else {
            return false;
        };
        let form = match (event.kind, hit) {
            (
                MouseKind::Down(MouseButton::Left),
                Hit::Text {
                    window,
                    line,
                    column,
                },
            ) => {
                self.set_mouse_drag(Some(MouseDrag::Text { window, at: (x, y) }));
                // One press or the second of a pair: a terminal reports presses
                // and no more, and only the editor keeps the first. See
                // `Windows::register_click`.
                let command = match self.register_click(x, y) {
                    ClickCount::Single => "mouse-set-point",
                    ClickCount::Double => "mouse-start-selection",
                };
                Some(mouse_form(
                    command,
                    &[
                        window.0 as f64,
                        line as f64,
                        self.buffer_column(window, line, column) as f64,
                    ],
                ))
            }
            (MouseKind::Down(MouseButton::Left), Hit::Separator { path, orientation }) => {
                self.set_mouse_drag(Some(MouseDrag::Divider {
                    path,
                    orientation,
                    last: x,
                }));
                None
            }
            (MouseKind::Down(MouseButton::Left), Hit::ModeLine { window }) => {
                // A status line divides a stacked pair, and which pair is asked
                // of the tree here, once, not at every event of the drag.
                let divider = self.windows(|windows| windows.root().mode_line_divider(window));
                if let Some(path) = divider {
                    self.set_mouse_drag(Some(MouseDrag::Divider {
                        path,
                        orientation: Orientation::Horizontal,
                        last: y,
                    }));
                }
                None
            }
            (MouseKind::ScrollUp, Hit::Text { window, .. }) => {
                Some(mouse_form("mouse-scroll", &[window.0 as f64, -1.0]))
            }
            (MouseKind::ScrollDown, Hit::Text { window, .. }) => {
                Some(mouse_form("mouse-scroll", &[window.0 as f64, 1.0]))
            }
            // Everything else is swallowed on purpose: every click on a float,
            // and every button nothing is bound to.
            _ => None,
        };
        let Some(form) = form else {
            return false;
        };
        self.run_command_form(form, env);
        true
    }
}
