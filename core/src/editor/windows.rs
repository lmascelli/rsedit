//! The window half of the facade: splitting, closing, focusing and scrolling.
//!
//! The compartment is [`crate::managers::Windows`]. What is here is the part
//! that needs a buffer: a scroll needs a line count, moving focus makes a
//! buffer current, and closing a window may hand focus to another.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Split the focused window, giving the two halves DIVISION, and return
    /// the new window's id. See [`Windows::split_focused`].
    ///
    /// `Division::Ratio(0.5)` is the ordinary `C-x 2`/`C-x 3`. A caller that
    /// wants to keep a particular size -- a directory listing that should stay
    /// a fixed width while the file beside it takes the rest -- passes
    /// `Division::FirstFixed` instead, the first child being the window that
    /// was split.
    pub(crate) fn split_focused_window(
        &self,
        orientation: Orientation,
        division: Division,
    ) -> Option<WindowId> {
        self.windows_mut(|windows| windows.split_focused(orientation, division))
    }

    /// Open a full-width window of exactly HEIGHT rows at the bottom of the
    /// frame, showing BUFFER, and return its id.
    ///
    /// Focus does not move, which is the property everything else rests on.
    /// See [`Windows::open_bottom`] for why.
    pub(crate) fn open_bottom_window(&self, buffer: &str, height: usize) -> WindowId {
        self.windows_mut(|windows| windows.open_bottom(buffer, height))
    }

    /// Scroll the focused window so that the line point is on sits `where_to`
    /// of the way down it, without moving point. See
    /// [`Windows::recenter_focused`].
    ///
    /// The coordination here is reading the buffer first: the compartment is
    /// told the two numbers it needs rather than being handed the buffer, so
    /// the two locks are never held together.
    ///
    /// False when nothing changed, so a caller can tell a no-op from a scroll.
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

    /// Move the focused window's view by AMOUNT screenfuls, forwards when
    /// AMOUNT is positive, and drag point along if it would otherwise be left
    /// outside. See [`Windows::scroll_focused`].
    ///
    /// Returns false when the view could not move at all -- already showing
    /// the end and asked to go forward, or the beginning and asked to go back
    /// -- so the caller can say so rather than leaving the key looking broken.
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
        // The window lock is let go before point is touched. Moving point is
        // a change to a *buffer*, which is why the compartment names a line
        // rather than making the move itself: it has no business holding a
        // buffer, and this way it never does.
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

    /// Close the window with ID, whoever has focus.
    ///
    /// Separate from `delete_focused_window` because a strip is closed by
    /// whatever put it there, which by then is running in a different window
    /// entirely -- giving the strip focus first, just to be able to close it,
    /// would make its buffer current and defeat the point of never focusing it.
    ///
    /// Returns false when there is no such window, or when it is the only one:
    /// a frame with no windows has nowhere to draw a cursor.
    pub(crate) fn delete_window_by_id(&self, id: WindowId) -> bool {
        // `Refocus` means the compartment has already moved focus. What is
        // left is the editor's half of a focus change -- making that window's
        // buffer current and putting point back where it was -- and it is done
        // out here, after the lock is given back, because both touch buffers.
        match self.windows_mut(|windows| windows.remove(id)) {
            WindowRemoved::No => false,
            WindowRemoved::Yes => true,
            WindowRemoved::Refocus(survivor) => {
                self.follow_focus(survivor);
                true
            }
        }
    }

    /// Close the focused window. False when it is the only one.
    ///
    /// The same operation as `delete_window_by_id` and now written as one: the
    /// two used to differ in whether focus moved afterwards, which was never a
    /// difference between them but a difference between *which* window went.
    /// [`Windows::remove`] answers that, so there is one rule rather than two
    /// copies of it that could drift.
    pub(crate) fn delete_focused_window(&self) -> bool {
        self.delete_window_by_id(self.get_focused_window_id())
    }

    /// Close every window but the focused one.
    pub(crate) fn delete_other_windows(&self) -> bool {
        self.windows_mut(|windows| windows.delete_others())
    }

    /// Move focus COUNT windows on, wrapping round.
    ///
    /// A negative count goes the other way, which is what lets one command
    /// serve `C-x o` and a reversed `C-x o` alike.
    pub(crate) fn focus_other_window(&self, count: isize) -> bool {
        // As in `delete_window_by_id`: the compartment moves focus, and the
        // buffer half of the move happens after the lock is back.
        let Some(landed) = self.windows_mut(|windows| windows.focus_other(count)) else {
            return false;
        };
        self.follow_focus(landed);
        true
    }

    /// How many tiled windows the frame holds.
    pub(crate) fn window_count(&self) -> usize {
        self.windows(|windows| windows.count())
    }

    /// The buffer shown by the focused window, which is not always the current
    /// buffer: a floating prompt takes the current buffer without taking a
    /// tiled window's place.
    pub(crate) fn focused_window_buffer(&self) -> Option<String> {
        self.windows(|windows| windows.focused_buffer())
    }

    /// Create a new buffer named BUF_NAME (in major mode MODE, defaulting
    /// to fundamental-mode) and open it in a new bordered floating window
    /// at (X, Y) with the given WIDTH/HEIGHT and optional TITLE, giving
    /// that window focus. Closing the floating window (`close-buffer` or
    /// `close-floating-window`) restores focus to whatever window was
    /// focused before this call. Shared by the `make-floating-window`
    /// primitive and the built-in minibuffer, so both open a floating
    /// window exactly the same way.
    pub(crate) fn open_floating_window(
        &self,
        buf_name: &str,
        x: isize,
        y: isize,
        width: usize,
        height: usize,
        title: Option<String>,
        mode: Option<String>,
    ) {
        let previous_focused_window_id = self.get_focused_window_id();
        self.new_buffer(buf_name, None, mode);

        // The id and the push are one acquisition rather than two, which is
        // what a single lock over the five old fields buys: no other thread
        // can see an id handed out and not yet used.
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

        // No `set_current_buffer_name` here: the float is in the list before
        // focus moves, so the chokepoint finds it and makes it current. Setting
        // it a second time would work today and rot the moment the two
        // disagree about what "the buffer of window N" means.
        self.set_focused_window_id(new_id);
    }

    /// Scroll WINDOW by LINES, towards the end of the buffer when positive.
    ///
    /// Neither focus nor point moves. False when there is no such window or
    /// the view was already as far as it goes.
    ///
    /// The two locks are taken one at a time and given straight back rather
    /// than nested: the line count has to come from the buffer and the scroll
    /// has to be written to the window, and holding both would put an edge in
    /// the ordering for the sake of an operation that does not need one.
    pub(crate) fn scroll_window_by(&self, window: WindowId, lines: isize) -> bool {
        let Some(name) = self.windows(|windows| windows.buffer_of(window)) else {
            return false;
        };
        let Some(line_count) = self.with_buffer(&name, |buf| buf.text.line_count()) else {
            return false;
        };
        self.windows_mut(|windows| windows.scroll_by(window, lines, line_count))
    }

    /// Get the ID of the current focused window
    pub fn get_focused_window_id(&self) -> WindowId {
        self.windows(|windows| windows.focused())
    }

    /// Move focus to window ID, and make what it shows the current buffer.
    ///
    /// False when there is not, which is the useful answer rather than a
    /// failure: a window remembered earlier may have been closed since, and
    /// the caller wants to open a new one rather than be stopped.
    pub(crate) fn select_window(&self, id: WindowId) -> bool {
        // Tiled or floating: a prompt is a float, and selecting one has to
        // work exactly as selecting a tiled window does.
        let exists = self.windows(|windows| windows.buffer_of(id).is_some());
        if exists {
            self.set_focused_window_id(id);
        }
        exists
    }

    pub(crate) fn set_focused_window_id(&self, id: WindowId) {
        self.windows_mut(|windows| windows.set_focused(id));
        self.follow_focus(id);
    }

    /// The buffer half of a focus change: make what the newly focused window
    /// shows the current buffer, and put point back where that window left it.
    ///
    /// Separate from [`EditorState::set_focused_window_id`] because focus does
    /// not always move from out here. `Windows::focus_other` and
    /// `Windows::remove` move it themselves -- they have the lock open and the
    /// list of survivors in hand -- and what they cannot do is touch a buffer.
    /// This is the other half, and every path that moves focus ends in it.
    ///
    /// Called with no lock held, deliberately. It takes the windows to ask
    /// what the window shows, and then the buffers to move point.
    fn follow_focus(&self, id: WindowId) {
        if let Some(name) = self.window_buffer(id) {
            self.set_current_buffer_name(&name);
            self.restore_window_point(id, &name);
        }
    }

    /// Put point back where the window taking focus last had it.
    ///
    /// Only tiled windows: a floating one is not in the layout, so it answers
    /// `None` and nothing happens -- which is right, since a prompt's point
    /// belongs to the prompt.
    fn restore_window_point(&self, id: WindowId, buffer_name: &str) {
        let remembered = self.windows(|windows| windows.root().window(id).and_then(|w| w.point));
        let Some(offset) = remembered else {
            return;
        };
        // The window lock is let go above rather than held across the write
        // below. Windows come before buffers in the ordering, so holding both
        // would be legal -- but every other path here copies out and lets go,
        // and the one that does not is the one that eventually deadlocks.
        self.with_buffer_mut(buffer_name, |buf| goto_offset(&mut buf.text, offset));
    }

    /// What window ID is showing, whether it is tiled or floating.
    ///
    /// Both, because a minibuffer prompt is a floating window and giving it
    /// focus has to make its buffer current in exactly the same way -- that is
    /// what every prompt in the editor depends on.
    fn window_buffer(&self, id: WindowId) -> Option<String> {
        self.windows(|windows| windows.buffer_of(id))
    }
}
