//! The windows' half of the facade: splitting, closing, focusing and scrolling,
//! with what each needs of a buffer -- a scroll needs a line count, a move of
//! focus makes a buffer current -- and what the mouse asks of the windows.
//!
//! Design: `doc/architettura-editor.typ, <coppie>`.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Splits the focused window, the halves divided by DIVISION, and returns
    /// the new window's id. See [`Windows::split_focused`].
    ///
    /// `Division::Ratio(0.5)` is `C-x 2` and `C-x 3`. A window that keeps its
    /// size -- a listing beside the file it lists -- is split with
    /// `Division::FirstFixed`, the first half being the window split.
    pub(crate) fn split_focused_window(
        &self,
        orientation: Orientation,
        division: Division,
    ) -> Option<WindowId> {
        self.windows_mut(|windows| windows.split_focused(orientation, division))
    }

    /// Opens a window as wide as the frame and HEIGHT rows high at its bottom,
    /// showing BUFFER, and returns its id. Focus stays where it is: see
    /// [`Windows::open_bottom`].
    pub(crate) fn open_bottom_window(&self, buffer: &str, height: usize) -> WindowId {
        self.windows_mut(|windows| windows.open_bottom(buffer, height))
    }

    /// Scrolls the focused window so that point's line sits WHERE_TO of the
    /// way down it, without moving point; false when nothing moved. See
    /// [`Windows::recenter_focused`].
    ///
    /// The buffer is read first, and the compartment is told the two numbers
    /// it needs: the two locks are never held together.
    pub(crate) fn recenter_focused_window(&self, where_to: f64) -> bool {
        let Some(name) = self.focused_window_buffer() else {
            return false;
        };
        let Some((line_count, point_line)) = self.with_buffer(&name, |buf| {
            (buf.text.line_count(), buf.text.cursor_pos().0)
        }) else {
            return false;
        };
        self.windows_mut(|windows| windows.recenter_focused(where_to, line_count, point_line))
    }

    /// Moves the focused window's view by AMOUNT screens, forward when AMOUNT
    /// is positive, and takes point along when it would be left outside. See
    /// [`Windows::scroll_focused`].
    ///
    /// False when the view could not move -- at the end going forward, at the
    /// start going back -- so that the caller can say so.
    pub(crate) fn scroll_focused_window(&self, amount: isize) -> bool {
        let Some(name) = self.focused_window_buffer() else {
            return false;
        };
        let Some((line_count, point_line, point_column)) = self.with_buffer(&name, |buf| {
            let (line, column) = buf.text.cursor_pos();
            (buf.text.line_count(), line, column)
        }) else {
            return false;
        };
        // The windows are let go before point moves: moving point changes a
        // buffer, and the compartment, which holds none, only names the line.
        let Scrolled::Yes { drag_point_to } =
            self.windows_mut(|windows| windows.scroll_focused(amount, line_count, point_line))
        else {
            return false;
        };
        if let Some(line) = drag_point_to {
            self.with_buffer_mut(&name, |buf| buf.text.cursor_move(line, point_column));
        }
        true
    }

    /// Closes the window ID, whichever window has focus; false when there is
    /// no such window, or it is the only one.
    ///
    /// For a strip, which whatever opened it closes while working in another
    /// window: focusing the strip first would make its buffer current.
    pub(crate) fn delete_window_by_id(&self, id: WindowId) -> bool {
        // `Refocus` means the compartment has moved focus. What is left is the
        // buffer's half of the move, which needs the lock given back first.
        match self.windows_mut(|windows| windows.remove(id)) {
            WindowRemoved::No => false,
            WindowRemoved::Yes => true,
            WindowRemoved::Refocus(survivor) => {
                self.follow_focus(survivor);
                true
            }
        }
    }

    /// Closes the focused window; false when it is the only one.
    pub(crate) fn delete_focused_window(&self) -> bool {
        self.delete_window_by_id(self.get_focused_window_id())
    }

    /// Closes every window but the focused one.
    pub(crate) fn delete_other_windows(&self) -> bool {
        self.windows_mut(|windows| windows.delete_others())
    }

    /// Moves focus COUNT windows on, round the end; a negative count goes the
    /// other way, so that one command serves `C-x o` both ways.
    pub(crate) fn focus_other_window(&self, count: isize) -> bool {
        // As in `delete_window_by_id`: the compartment moves focus, and the
        // buffer's half of the move comes once the lock is back.
        let Some(landed) = self.windows_mut(|windows| windows.focus_other(count)) else {
            return false;
        };
        self.follow_focus(landed);
        true
    }

    /// Returns how many tiled windows the frame holds.
    pub(crate) fn window_count(&self) -> usize {
        self.windows(|windows| windows.count())
    }

    /// Returns the buffer the focused window shows. Not always the current
    /// one: a prompt takes the current buffer without taking a tiled window's
    /// place.
    pub(crate) fn focused_window_buffer(&self) -> Option<String> {
        self.windows(|windows| windows.focused_buffer())
    }

    /// Makes a buffer BUF_NAME, in MODE or `fundamental-mode`, shows it in a
    /// bordered floating window at (X, Y), WIDTH by HEIGHT and titled TITLE,
    /// gives that window focus, and returns its id. Closing the window gives
    /// focus back to the one that had it.
    ///
    /// The `make-floating-window` primitive and the minibuffer both open their
    /// windows here.
    pub(crate) fn open_floating_window(
        &self,
        buf_name: &str,
        x: isize,
        y: isize,
        width: usize,
        height: usize,
        title: Option<String>,
        mode: Option<String>,
        env: &Arc<Env<EditorState<B>>>,
    ) -> WindowId {
        let previous_focused_window_id = self.get_focused_window_id();
        self.new_buffer(buf_name, None, mode, env);

        // The id and the push in one acquisition: no other thread sees an id
        // handed out and not yet used.
        let new_id = self.windows_mut(|windows| {
            let new_id = windows.next_id();
            windows.floating_mut().push(FloatingWindow {
                window: Window::new(new_id, buf_name),
                rect: Rect {
                    x,
                    y,
                    width,
                    height,
                },
                has_border: true,
                title,
                previous_focused_window_id,
            });
            new_id
        });

        // The float is in the list before focus moves, so following the focus
        // makes its buffer current: setting it here too would be a second
        // answer to "what window N shows".
        self.set_focused_window_id(new_id);
        new_id
    }

    /// Scrolls WINDOW by LINES, toward the end of the buffer when LINES is
    /// positive, without moving focus or point; false when there is no such
    /// window, or the view was already as far as it goes.
    ///
    /// The two locks are taken one after the other, never together: the line
    /// count from the buffer, then the scroll into the window.
    pub(crate) fn scroll_window_by(&self, window: WindowId, lines: isize) -> bool {
        let Some(name) = self.windows(|windows| windows.buffer_of(window)) else {
            return false;
        };
        let Some((line_count, point_line, point_column)) = self.with_buffer(&name, |buf| {
            let (line, column) = buf.text.cursor_pos();
            (buf.text.line_count(), line, column)
        }) else {
            return false;
        };
        // As in `scroll_focused_window`: the windows are let go before point
        // moves.
        let Scrolled::Yes { drag_point_to } =
            self.windows_mut(|windows| windows.scroll_by(window, lines, line_count, point_line))
        else {
            return false;
        };
        if let Some(line) = drag_point_to {
            self.with_buffer_mut(&name, |buf| buf.text.cursor_move(line, point_column));
        }
        true
    }

    /// Returns the id of the window with focus.
    pub fn get_focused_window_id(&self) -> WindowId {
        self.windows(|windows| windows.focused())
    }

    /// Gives window ID focus, and makes what it shows the current buffer;
    /// false when there is no such window, which a window remembered earlier
    /// may have become.
    pub(crate) fn select_window(&self, id: WindowId) -> bool {
        // Tiled or floating: a prompt is a float, and selecting one works as
        // selecting a tiled window does.
        let exists = self.windows(|windows| windows.buffer_of(id).is_some());
        if exists {
            self.set_focused_window_id(id);
        }
        exists
    }

    /// Gives window ID focus, makes what it shows the current buffer, and
    /// puts point back where that window left it.
    pub(crate) fn set_focused_window_id(&self, id: WindowId) {
        self.windows_mut(|windows| windows.set_focused(id));
        self.follow_focus(id);
    }

    /// The buffer's half of a move of focus: makes what window ID shows the
    /// current buffer, and puts point back where that window left it.
    ///
    /// Apart from [`EditorState::set_focused_window_id`] because
    /// `Windows::focus_other` and `Windows::remove` move focus themselves,
    /// holding the list of survivors, and cannot touch a buffer. Every move of
    /// focus ends here, with no lock held.
    fn follow_focus(&self, id: WindowId) {
        if let Some(name) = self.window_buffer(id) {
            self.set_current_buffer_name(&name);
            self.restore_window_point(id, &name);
        }
    }

    /// Puts point in BUFFER_NAME back where window ID last had it. Only a
    /// tiled window remembers one: a float answers `None`, and a prompt's point
    /// is the prompt's own.
    fn restore_window_point(&self, id: WindowId, buffer_name: &str) {
        let remembered = self.windows(|windows| windows.root().window(id).and_then(|w| w.point));
        let Some(offset) = remembered else {
            return;
        };
        // The windows are let go before the buffer is written. Holding both
        // would keep to the order, windows before buffers, but every other
        // path here copies out and lets go.
        self.with_buffer_mut(buffer_name, |buf| goto_offset(&mut buf.text, offset));
    }

    /// Returns what window ID shows, tiled or floating: a prompt is a float,
    /// and focusing one makes its buffer current as with any window.
    fn window_buffer(&self, id: WindowId) -> Option<String> {
        self.windows(|windows| windows.buffer_of(id))
    }

    // ------------------------------------------------------------------
    // What the mouse asks of the windows
    // ------------------------------------------------------------------

    /// Returns what the pointer at (X, Y) is over, in cells of the last frame;
    /// `None` over the echo area, or a gap no window claims.
    pub(crate) fn hit_test(&self, x: isize, y: isize) -> Option<Hit> {
        self.windows(|windows| windows.hit_test(x, y))
    }

    /// Records a left button going down at (X, Y), and returns whether it is
    /// the second of a pair.
    pub(super) fn register_click(&self, x: isize, y: isize) -> ClickCount {
        self.windows_mut(|windows| windows.register_click(x, y))
    }

    /// Ends the drag in progress, and returns it.
    pub(super) fn take_mouse_drag(&self) -> Option<MouseDrag> {
        self.windows_mut(|windows| windows.take_drag())
    }

    /// Moves the boundary of the split at PATH by DELTA cells; false when it
    /// could not move.
    pub(crate) fn resize_dragged_split(&self, path: &[Side], delta: isize) -> bool {
        self.windows_mut(|windows| windows.resize_split(path, delta))
    }

    /// Records DRAG as the drag in progress, or that there is none.
    pub(super) fn set_mouse_drag(&self, drag: Option<MouseDrag>) {
        self.windows_mut(|windows| windows.set_drag(drag));
    }

    /// Returns the drag in progress, if any.
    pub(crate) fn mouse_drag(&self) -> Option<MouseDrag> {
        self.windows(|windows| windows.drag())
    }

    /// Returns which way a drag that has left WINDOW, at row Y, wants the view
    /// to move: up or down only, since off a side the clamping already selects
    /// to the end of the lines.
    pub(super) fn drag_scroll_step(&self, window: WindowId, y: isize) -> Option<isize> {
        self.windows(|windows| windows.drag_scroll_step(window, y))
    }

    /// Returns how long until a drag that has left its window should scroll
    /// again, or `None` when none is waiting to. A pointer held still outside
    /// a window sends nothing, and the selection must go on growing all the
    /// same: the host waits this long, then calls [`Self::tick`].
    pub fn drag_scroll_in(&self) -> Option<Duration> {
        self.windows(|windows| windows.drag_scroll_in())
    }
}
