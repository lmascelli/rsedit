//! Turning pointer events into commands.
//!
//! Working out *where* a click landed needs the layout and every window's
//! scroll, so it happens here. Deciding what a click *means* goes through the
//! command machinery, so a binding can be replaced from Lisp.

use super::*;

/// A command form with numeric arguments, built rather than parsed.
///
/// Built, because the arguments are numbers the editor just worked out: going
/// through the parser would mean formatting them into text for it to read back.
fn mouse_form<B: BufferTrait>(name: &str, args: &[f64]) -> ELispExp<B> {
    let mut items = vec![ELispExp::symbol(name.into())];
    items.extend(args.iter().copied().map(ELispExp::number));
    ELispExp::form(items)
}

impl<B: BufferTrait> EditorState<B> {
    /// What the pointer at (X, Y) is over, in cells of the last frame.
    ///
    /// `None` when it is over nothing -- the echo area, or a gap no window
    /// claims.
    pub(crate) fn hit_test(&self, x: isize, y: isize) -> Option<Hit> {
        self.windows(|windows| windows.hit_test(x, y))
    }

    pub(crate) fn take_mouse_drag(&self) -> Option<MouseDrag> {
        self.windows_mut(|windows| windows.take_drag())
    }

    /// Move the boundary of the split at PATH by DELTA cells.
    pub(crate) fn resize_dragged_split(&self, path: &[Side], delta: isize) -> bool {
        self.windows_mut(|windows| windows.resize_split(path, delta))
    }

    fn set_mouse_drag(&self, drag: Option<MouseDrag>) {
        self.windows_mut(|windows| windows.set_drag(drag));
    }

    pub(crate) fn mouse_drag(&self) -> Option<MouseDrag> {
        self.windows(|windows| windows.drag())
    }

    /// Where in WINDOW's buffer the cell (X, Y) is, clamped to what the window
    /// is showing.
    ///
    /// Clamped rather than refused because this answers a *drag*, and a drag
    /// that leaves the window is a perfectly ordinary way to select to its
    /// edge. Dragging beyond an edge selects to that edge and stops; it does
    /// not scroll the window after the pointer, which is a separate feature
    /// and a worse one to get subtly wrong.
    fn position_in(&self, window: WindowId, x: isize, y: isize) -> Option<(usize, usize)> {
        self.windows(|windows| windows.position_in(window, x, y))
    }

    /// Which way a drag that has left its window wants the view to move, if it
    /// has left it at all.
    ///
    /// Vertically only. Dragging off the side of a window is a request to
    /// select to the end of the lines you are over, which clamping already
    /// gives; dragging off the top or bottom is a request for lines that are
    /// not on screen, which nothing but scrolling can answer.
    fn drag_scroll_step(&self, window: WindowId, y: isize) -> Option<isize> {
        self.windows(|windows| windows.drag_scroll_step(window, y))
    }

    /// How long until a drag that has left its window should scroll again, or
    /// `None` when none is waiting to.
    ///
    /// # Why this is a timer and not an event
    ///
    /// A pointer held still outside a window sends nothing at all, and that is
    /// exactly the position somebody selecting a long passage leaves it in.
    /// Scrolling only on movement would mean the selection stopped the moment
    /// they stopped jiggling the mouse, which reads as the editor having lost
    /// interest.
    pub fn drag_scroll_in(&self) -> Option<Duration> {
        self.windows(|windows| windows.drag_scroll_in())
    }

    /// Scroll a drag that has left its window by one line, and take the
    /// selection with it. False when there was nothing to do.
    ///
    /// One line a turn rather than a distance that grows with how far outside
    /// the pointer is: a selection running away faster the further the hand
    /// strays is hard to stop where you meant to, and the turn is short enough
    /// that holding the pointer out is smooth anyway.
    pub fn drag_scroll_tick(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        let Some(MouseDrag::Text { window, at: (x, y) }) = self.mouse_drag() else {
            return false;
        };
        let Some(step) = self.drag_scroll_step(window, y) else {
            return false;
        };
        if !self.scroll_window_by(window, step) {
            // Already as far as the buffer goes. The selection is already at
            // that end, so there is nothing left to extend either.
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

    /// One more event of a drag already in progress.
    fn continue_drag(&self, x: isize, y: isize, env: &Arc<Env<EditorState<B>>>) -> bool {
        let form = match self.mouse_drag() {
            Some(MouseDrag::Text { window, .. }) => {
                self.set_mouse_drag(Some(MouseDrag::Text { window, at: (x, y) }));
                // Against the window the drag began in, not whatever is under
                // the pointer now: a selection that changed buffers halfway
                // through is nobody's idea of a selection.
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
                // axis the boundary moves in. Kept as a running position
                // rather than compared with where the drag started, so a
                // boundary that could not move as far as the pointer did does
                // not then lag behind it for the rest of the drag.
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

    /// Act on a mouse event from the frontend.
    ///
    /// # Why this resolves and then dispatches
    ///
    /// Working out *where* a click landed needs the layout and every window's
    /// scroll, which only the editor has -- so it happens here. Deciding what
    /// a click *means* is a different kind of question, and it goes through
    /// the command machinery exactly as `handle_paste` does: one undo step,
    /// one `post-command-hook`, a name that `M-x` and the logs can see, and a
    /// binding that can be replaced later without touching this function.
    ///
    /// Answers whether the event was acted on. A terminal that is reporting
    /// the mouse sends motion and drag events by the dozen per second, and
    /// nearly all of them mean nothing here -- so the frontend needs to know
    /// which ones are worth a redraw, or it repaints the screen continuously
    /// for frames identical to the last.
    pub fn handle_mouse_event(&self, event: MouseEvent, env: &Arc<Env<EditorState<B>>>) -> bool {
        // Checked here rather than only where the terminal switches capture
        // on, so the setting means the same thing to every frontend: a GUI
        // that always delivers mouse events has to obey it too.
        if !mouse_mode(env) {
            return false;
        }
        let (x, y) = (event.column as isize, event.row as isize);

        // Continuing or ending a drag is answered before asking what is under
        // the pointer, because a drag is about where it *began*. By the time
        // one is a few rows long the pointer is often over another window or
        // off the frame entirely, where the hit test answers nothing -- and a
        // selection that stopped growing the moment you overshot the window
        // would be a selection you could not make.
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
                Some(mouse_form(
                    "mouse-set-point",
                    &[window.0 as f64, line as f64, column as f64],
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
                // A status line divides a stacked pair, and which pair is a
                // question about the tree -- asked here, once, rather than on
                // every event of the drag that follows.
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
            // Everything else is swallowed on purpose -- every click on a
            // float, and every button nothing is bound to yet.
            _ => None,
        };
        let Some(form) = form else {
            return false;
        };
        self.run_command_form(form, env);
        true
    }
}
