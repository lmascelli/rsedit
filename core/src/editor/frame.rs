//! What the renderer is told, and when it should ask again.
//!
//! [`EditorState::snapshot`] is the one place that holds more than one
//! compartment at a time, and so the one place that defines their order.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Ask the editor for a list of window to be rendered. Those are composed of a rect that tells
    /// where the window is placed and its size, the name of the buffer it represents, if it's
    /// focused, the relative cursor position in it, if it has a border and of course the line
    /// that it contains and that have to be drawn.
    /// Capture everything the UI needs to draw one frame.
    ///
    /// # Lock order
    ///
    /// This is the **canonical order** for the whole editor. It is currently
    /// the only code path that holds more than one of these at a time, so it
    /// gets to define the order; anything added later that needs two or more
    /// must take them in this sequence or risk a deadlock the moment a second
    /// thread writes:
    ///
    /// `echo_message` -> `windows` -> `buffers` -> an individual `Buffer`
    ///
    /// It used to be five links long. Three of them -- the focused id, the
    /// layout and the floating windows -- are one lock now, so the order they
    /// had to be taken in is not a rule anybody can get wrong any more.
    ///
    /// The Lisp environment is read first of all, before any of these, so that
    /// no editor lock is ever held while touching it.
    ///
    /// Cheap scalars first so the structural locks are held for as short a
    /// time as possible. Every lock is acquired exactly once and released
    /// before the caller sees the result, so no terminal I/O ever happens with
    /// a lock held.
    ///
    /// # Why one capture rather than field-by-field reads
    ///
    /// See [`FrameSnapshot`]. The short version: `BackgroundScheduler` and
    /// `(spawn ...)` already mutate this state from other threads, so a
    /// renderer that reads six locks at six different instants can compose a
    /// frame that never existed.
    ///
    /// # Note on `windows`
    ///
    /// A *write* lock, because `compute_tiled_views` adjusts each window's
    /// `scroll_x`/`scroll_y` to keep the cursor in view -- rendering mutates
    /// editor state. That works while one thread renders, and is the thing to
    /// untangle before the UI loop moves off the command thread: hoisting the
    /// scroll reconciliation into an explicit post-command step would let this
    /// take a read lock and let renders run concurrently.
    pub fn snapshot(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        screen_width: usize,
        screen_height: usize,
    ) -> FrameSnapshot {
        // Read *before* the first editor lock is taken. The environment has
        // locks of its own, and taking one while holding an editor lock would
        // add an edge to the ordering below that nothing else respects.
        let echo_timeout = echo_timeout(env);
        // The theme is read before the structural locks and never alongside
        // them, so it stays outside the ordering below rather than becoming
        // another link in it. It is a small `Copy` value, so this is a memcpy
        // and the lock is released immediately.
        let theme = self.theme();
        let pending_input = self.pending_input();
        let prompt = self.transient_message();
        let mode_line_format = mode_line_format(env);
        let separator_char = window_separator(env);
        // Asked before the structural locks, alongside the other independent
        // reads: it takes the buffers and the mode registry, and the registry
        // has no place in the ordering that begins below.
        let colouring_pending = self.colouring_pending();

        // An expired message is simply not reported. The state keeps it -- the
        // view is what forgets, so nothing has to run on a timer to tidy up.
        let echo_message = {
            let echo = self
                .echo_message
                .read()
                .expect("Failed to acquire read lock on echo_message");
            match echo.visible_for(echo_timeout) {
                Some(_) => echo.text.clone(),
                None => String::new(),
            }
        };

        // One acquisition for the tiled tree, the floats and the focused id.
        // The three used to be read separately, which is how a frame could
        // report a cursor in a window the layout had already removed.
        let (views, separators, focused_window_id) = self.windows_mut(|windows| {
            let buffers = self
                .buffers
                .read()
                .expect("Failed to acquire read lock on buffers");

            let focused_window_id = windows.focused();
            let mut views = Vec::new();
            let mut separator_rects = Vec::new();
            let focus = Focus {
                id: focused_window_id,
                tiled: windows.root().contains_window(focused_window_id),
            };
            windows.root_mut().compute_tiled_views(
                Rect {
                    x: 0,
                    y: 0,
                    width: screen_width,
                    // The bottom row belongs to the echo area, which is drawn over
                    // whatever is under it. Tiling into it would put a window's
                    // status line on the same row as a message, and one of the two
                    // would win at random.
                    height: screen_height.saturating_sub(1),
                },
                focus,
                &buffers,
                &mode_line_format,
                &mut views,
                &mut separator_rects,
            );

            let separators: Vec<Separator> = separator_rects
                .into_iter()
                .map(|rect| Separator {
                    rect,
                    ch: separator_char,
                    face: Face::WINDOW_SEPARATOR,
                })
                .collect();

            for float in windows.floating().iter() {
                let is_focused = float.window.id == focused_window_id;
                // Unlike a tiled window, a float is not auto-scrolled to follow the
                // cursor; its scroll offsets are whatever whoever opened it set.
                let cursor_rel_pos = is_focused
                    .then(|| buffers.handle(&float.window.buffer_name))
                    .flatten()
                    .map(|buf| {
                        let (c_line, c_col) = buf
                            .read()
                            .expect("Failed to acquire read lock on buffer")
                            .text
                            .cursor_pos();
                        (
                            c_col.saturating_sub(float.window.scroll_x),
                            c_line.saturating_sub(float.window.scroll_y),
                        )
                    });

                views.push(RenderableWindowView {
                    rect: float.rect,
                    buffer_name: float.window.buffer_name.clone(),
                    title: float.title.clone(),
                    is_focused,
                    cursor_rel_pos,
                    lines: extract_buffer_lines(&float.window, &float.rect, &buffers),
                    highlights: region_highlights(&float.window, &float.rect, &buffers),
                    // A float says what it is on its border, so a status line
                    // would be a second answer to the same question.
                    mode_line: None,
                    has_border: float.has_border,
                });
            }

            (views, separators, focused_window_id)
        });

        FrameSnapshot {
            views,
            echo_message,
            pending_input,
            prompt,
            theme,
            focused_window_id,
            width: screen_width,
            height: screen_height,
            colouring_pending,
            separators,
            clipboard: self.take_pending_clipboard(),
        }
    }

    pub fn resize(&self, env: Arc<Env<Self>>, new_screen_width: usize, new_screen_height: usize) {
        env.set_variable(
            "frame-width".into(),
            ELispExp::number(new_screen_width as f64),
        );
        env.set_variable(
            "frame-height".into(),
            ELispExp::number(new_screen_height as f64),
        );
        if let Some(callback_list) = env.get_variable("after-resize-hook") {
            for el in callback_list.iter() {
                let _command = self.begin_command();
                match &el {
                    ELispExp::Lambda(_) | ELispExp::Symbol(_) => {
                        if let Err(err) = eval(
                            &ELispExp::form(vec![
                                el.clone(),
                                ELispExp::number(new_screen_width as f64),
                                ELispExp::number(new_screen_height as f64),
                            ]),
                            env.clone(),
                            self,
                        ) {
                            self.log_diagnostic(&format!("[ERROR] resize: {:?}", err));
                        }
                    }
                    _ => {
                        self.log_diagnostic(&format!(
                            "[WARNING] not a valid lambda for after-resize-hook {:?}",
                            el
                        ));
                    }
                }
            }
        } else {
            self.log_diagnostic(
                "[WARNING] there is not after-resize-hook variable bound or it's not a list",
            );
        }
    }

    /// Return the editor echo string.
    ///
    /// The message as stored, whether or not it is still being shown: expiry
    /// is a question for the view, decided in [`Self::snapshot`], and state is
    /// not rewritten by the passage of time.
    pub fn get_echo_message(&self) -> String {
        self.echo_message
            .read()
            .expect("Failed to acquire read lock on echo_message")
            .text
            .clone()
    }

    /// How long until the current echo message stops being shown, or `None`
    /// when nothing is waiting to expire -- there is no message, no timeout is
    /// set, or the message has already expired.
    ///
    /// This is what a UI event loop needs in order to redraw when a message
    /// vanishes. Without it the loop blocks on the next key and the message
    /// stays on screen until the user happens to press one, which is not a
    /// timeout so much as a coincidence.
    pub fn echo_expiry_in(&self, env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
        let timeout = echo_timeout(env)?;
        let elapsed = self
            .echo_message
            .read()
            .expect("Failed to acquire read lock on echo_message")
            .visible_for(Some(timeout))?;
        timeout.checked_sub(elapsed).filter(|left| !left.is_zero())
    }

    /// How long a renderer may wait for input before the screen it has just
    /// drawn will want drawing again, or `None` when it may wait indefinitely.
    ///
    /// # Why the renderer asks rather than decides
    ///
    /// Almost everything on screen changes because the user did something, so a
    /// renderer can draw, block on input, and be right. Two things do not: an
    /// echo message that expires on a timer, and colour that arrives from the
    /// highlighter's thread. A renderer blocked on input sleeps through both.
    ///
    /// Which of those are outstanding, and how soon each matters, are questions
    /// about the editor, not about drawing -- so they are answered here and the
    /// renderer is handed a single duration. A second frontend gets the
    /// behaviour by asking the same question, rather than by remembering to
    /// reimplement two special cases.
    ///
    /// FRAME is the one just drawn, because the question is whether *that*
    /// frame goes stale. Taking it from the frame also means the colouring is
    /// not asked about twice per redraw, once to compose and once to wait.
    ///
    /// `None` is the ordinary answer, and it is the one that matters: with
    /// nothing being coloured and no message pending there is no wake-up at
    /// all, so an idle editor costs nothing.
    pub fn next_redraw_in(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        frame: &FrameSnapshot,
    ) -> Option<Duration> {
        // One more turn is the soonest new colour can appear, so it is the
        // longest this may sleep without being late for it.
        let colouring = frame.colouring_pending.then_some(TURN_INTERVAL);
        // Output from a running command arrives without anybody pressing a
        // key, so the renderer has to come back and look. Same interval as
        // colouring for the same reason: it is short enough to read as live
        // and long enough to cost nothing.
        let shell = (self.shell_commands_running() > 0).then_some(TURN_INTERVAL);
        [self.echo_expiry_in(env), colouring, shell]
            .into_iter()
            .flatten()
            .min()
    }

    /// Set the echo message to be MSG, and start its timeout running.
    pub fn set_echo_message(&self, msg: &str) {
        *self
            .echo_message
            .write()
            .expect("Failed to acquire write lock on echo_message") = EchoMessage::new(msg);
    }

    /// Bind FACE to STYLE for the whole editor.
    /// Put every face back to how the editor ships it.
    ///
    /// The compiled-in defaults rather than a snapshot taken at startup, so
    /// this means the same thing however many themes have been applied since.
    pub(crate) fn reset_theme(&self) {
        self.runtime_mut(|runtime| runtime.reset_theme());
    }

    pub(crate) fn set_face_style(&self, face: Face, style: Style) {
        // Copy-on-write: whatever frames are already holding keep the theme
        // they were composed under, and the next one picks this up.
        self.runtime_mut(|runtime| runtime.set_face_style(face, style));
    }

    pub(crate) fn face_style(&self, face: Face) -> Style {
        self.runtime(|runtime| runtime.face_style(face))
    }

    pub(crate) fn theme(&self) -> Arc<Theme> {
        self.runtime(|runtime| runtime.theme())
    }

    // ---------------------------------------------------------------
    // The kill ring
    // ---------------------------------------------------------------
}
