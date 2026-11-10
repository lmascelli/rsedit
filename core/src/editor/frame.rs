//! What the host is told: the snapshot of a frame, the echo area, the size of
//! the screen, and how long the host may wait before it draws again; and what
//! a frame draws for the moment, the delimiters paired at point and the
//! matches of a search.
//!
//! Design: `doc/architettura-editor.typ, <snapshot>`, and
//! `doc/architettura-editor.typ, <evidenziazioni>` for what a frame draws for
//! the moment.

use super::*;
use crate::modes::sexp;

/// The name of the Lisp variable that turns pairing the delimiter at point
/// off.
pub(crate) const SHOW_PAREN_MODE: &str = "show-paren-mode";

/// The name of the Lisp variable that turns drawing every match of a search
/// or a replace off.
pub(crate) const HIGHLIGHT_ALL_MATCHES: &str = "highlight-all-matches";

/// How many characters the delimiter at point is looked for over, on either
/// side of point: the reach of Emacs' `blink-matching-paren-distance`, so
/// that a frame never scans a whole large file.
pub(crate) const SHOW_PAREN_REACH: usize = 102_400;

/// Returns whether the delimiter at point is paired, as Lisp has it now;
/// unset counts as on.
fn showing_parens<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(SHOW_PAREN_MODE, true)
}

/// Returns whether every match of a search or a replace is drawn, as Lisp has
/// it now; unset counts as on.
fn highlighting_matches<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(HIGHLIGHT_ALL_MATCHES, true)
}

/// What the focused window's delimiter is paired with: its buffer, the mode
/// read when the table was, and the table. Read before the frame's locks, and
/// used under them only while the buffer is still in that mode.
struct ParenTable {
    buffer: String,
    mode: String,
    table: SyntaxTable,
}

/// Returns the delimiters to draw in the focused window, FOCUSED, from
/// PARENS: the one next to point and the one it pairs with, in
/// [`Face::SHOW_PAREN_MATCH`], or in [`Face::SHOW_PAREN_MISMATCH`] when the
/// two are of different kinds or there is no partner. Under the frame's
/// locks, from BUFFERS.
fn paren_spots<B: BufferTrait>(
    parens: Option<&ParenTable>,
    focused: WindowId,
    focused_buffer: Option<&str>,
    buffers: &Buffers<B>,
) -> Option<SpotMarks> {
    let parens = parens.filter(|parens| focused_buffer == Some(parens.buffer.as_str()))?;
    let buf = buffers.handle(&parens.buffer)?;
    let buf = buf
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if buf.current_mode != parens.mode {
        return None;
    }
    let point = buf.text.cursor_pos_1d();
    let pair = sexp::pairing(
        &buf.text,
        &parens.table,
        buf.scan_resume(point, &parens.table),
        point,
        SHOW_PAREN_REACH,
    )?;
    let face = if pair.matched {
        Face::SHOW_PAREN_MATCH
    } else {
        Face::SHOW_PAREN_MISMATCH
    };
    let mut spots = vec![(pair.at, face)];
    spots.extend(pair.partner.map(|partner| (partner, face)));
    Some(SpotMarks {
        window: focused,
        spots,
    })
}

/// How often the host draws again while something changes the screen behind
/// its back -- colour arriving, a shell command's output, a background job --
/// and how long a Lisp worker's `(yield)` waits when it names no interval.
///
/// Short enough that colour follows typing without a visible lag, long enough
/// that a burst of keystrokes is one frame's work rather than twenty.
pub const TURN_INTERVAL: Duration = Duration::from_millis(40);

impl<B: BufferTrait> EditorState<B> {
    /// Returns everything the host needs to draw one frame of SCREEN_WIDTH by
    /// SCREEN_HEIGHT cells: each window's rectangle, text, highlights, cursor
    /// and status line, the separators, the echo area and what is being typed,
    /// the theme, and the text owed to the system clipboard.
    ///
    /// Taken at once, holding the locks and doing no I/O, so that a frame is a
    /// state the editor was in: the jobs, the threads of the shell commands and
    /// those of `spawn` change it while the host draws. The Lisp variables are
    /// read before any lock, the small fields one at a time, and then the
    /// windows and the buffers together, the windows for writing: composing a
    /// window scrolls it to keep point in view.
    pub fn snapshot(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        screen_width: usize,
        screen_height: usize,
    ) -> FrameSnapshot {
        // Before anything is read: what the jobs changed up to here is in this
        // frame, and a step that ends while it is composed raises the flag
        // again, for the next.
        self.background_change.taken();
        // Before the first of the editor's locks: the environment has locks of
        // its own, and taking one under an editor's lock would add an edge to
        // the order that nothing else keeps.
        let echo_timeout = echo_timeout(env);
        // Before the windows and never with them: an `Arc`, cloned out, and the
        // lock given straight back.
        let theme = self.theme();
        let pending_input = self.pending_input();
        let prompt = self.transient_message();
        let mode_line_format = mode_line_format(env);
        let separator_char = window_separator(env);
        let gutter = line_numbers(env);
        let scroll_conservatively = scroll_conservatively(env);
        // The names out of the registry and its lock given back, then each
        // mode's property read from the environment: no lock of the editor is
        // held while the environment's is taken.
        let mode_names = self.modes(|modes| modes.names().cloned().collect::<Vec<_>>());
        let tabs = tab_widths(env, mode_names.clone());
        let trailing = trailing_spaces(env, mode_names);
        // With the reads before the windows: it takes the buffers and the
        // registry of the modes, which has no place in the order below.
        let colouring_pending = self.colouring_pending();
        // The same: the focused buffer's mode, and then the registry.
        let parens = showing_parens(env).then(|| self.paren_table()).flatten();
        let matches = highlighting_matches(env)
            .then(|| self.search_marks())
            .flatten();

        // A message that has expired is not reported. The state keeps it, and
        // the view forgets it: nothing runs on a timer to tidy up.
        let echo_message = {
            let echo = self
                .echo_message
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match echo.visible_for(echo_timeout) {
                Some(_) => echo.text.clone(),
                None => String::new(),
            }
        };

        // The tiled tree, the floats and the focus in one acquisition, with
        // the buffers they show.
        let (views, separators, focused_window_id) =
            self.windows_mut_with_buffers(|windows, buffers| {
                let focused_window_id = windows.focused();
                let mut views = Vec::new();
                let mut separator_rects = Vec::new();
                let focus = Focus {
                    id: focused_window_id,
                    // The minibuffer's split counts as a float here: while
                    // it is typed in, every other window follows its point,
                    // as during an incremental search.
                    tiled: windows
                        .root()
                        .window(focused_window_id)
                        .is_some_and(|window| !window.is_prompt()),
                };
                let spots = paren_spots(
                    parens.as_ref(),
                    focused_window_id,
                    windows.focused_buffer().as_deref(),
                    buffers,
                );
                let settings = ComposeSettings {
                    mode_line_format: &mode_line_format,
                    gutter,
                    tabs: &tabs,
                    trailing: &trailing,
                    matches: matches.as_ref(),
                    spots: spots.as_ref(),
                    scroll_conservatively,
                };
                windows.root_mut().compute_tiled_views(
                    Rect {
                        x: 0,
                        y: 0,
                        width: screen_width,
                        // The bottom row is the echo area's, drawn over what is
                        // under it: a window's status line there would share
                        // the row with a message.
                        height: screen_height.saturating_sub(1),
                    },
                    focus,
                    buffers,
                    settings,
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
                    // A float is not scrolled to follow the cursor: its offsets
                    // are what whoever opened it set. Its rows are composed as
                    // a tiled window's are, since a prompt can carry virtual
                    // text as a file can.
                    let layout = compose_layout(&float.window, &float.rect, buffers, &tabs);
                    let scroll_x = if layout.is_wrapped() {
                        0
                    } else {
                        float.window.scroll_x
                    };
                    let cursor_rel_pos = is_focused
                        .then(|| buffers.handle(&float.window.buffer_name))
                        .flatten()
                        .and_then(|buf| {
                            let (c_line, c_col) = buf
                                .read()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .text
                                .cursor_pos();
                            layout.row_of_position(c_line, c_col).map(|(index, row)| {
                                (row.to_screen(c_col).saturating_sub(scroll_x), index)
                            })
                        });
                    // What a tiled window draws, in the same order, and then
                    // the virtual text's own faces.
                    let in_buffer = window_highlights(
                        &float.window,
                        &float.rect,
                        buffers,
                        settings,
                        is_focused,
                    );
                    let mut highlights =
                        layout::place(&layout, in_buffer, scroll_x, float.rect.width);
                    highlights.extend(layout.virtual_highlights(scroll_x, float.rect.width));

                    views.push(RenderableWindowView {
                        rect: float.rect,
                        buffer_name: float.window.buffer_name.clone(),
                        title: float.title.clone(),
                        is_focused,
                        cursor_rel_pos,
                        lines: layout.visible_text(scroll_x, float.rect.width),
                        highlights,
                        // A float says what it is on its border, and a status
                        // line would say it again.
                        mode_line: None,
                        has_border: float.has_border,
                        // No gutter on a float: a prompt or a strip of choices
                        // is a few rows read and dismissed, whose numbers say
                        // nothing, in a window only as wide as what it shows.
                        gutter: Vec::new(),
                        gutter_width: 0,
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

    /// Tells the editor that the screen is now NEW_SCREEN_WIDTH by
    /// NEW_SCREEN_HEIGHT cells: sets `frame-width` and `frame-height`, and
    /// calls each function of `after-resize-hook` with the two, each with a
    /// budget of its own (`begin_command`). One that fails is logged.
    pub fn resize(&self, env: Arc<Env<Self>>, new_screen_width: usize, new_screen_height: usize) {
        env.set_variable(
            "frame-width".into(),
            ELispExp::number(new_screen_width as f64),
        );
        env.set_variable(
            "frame-height".into(),
            ELispExp::number(new_screen_height as f64),
        );
        if let Some(callback_list) = env.get_variable(AFTER_RESIZE_HOOK) {
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

    /// Returns the echo area's text as stored, whether or not it is still
    /// shown: when it stops being shown is a question for the view, which
    /// [`Self::snapshot`] answers.
    pub fn get_echo_message(&self) -> String {
        self.echo_message
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .text
            .clone()
    }

    /// Returns how long until the echo area's message stops being shown, or
    /// `None` when nothing is waiting to expire: no message, no timeout, or a
    /// message already expired.
    ///
    /// A host waiting for input asks, so as to redraw when the message goes,
    /// and not at whatever key comes next.
    pub fn echo_expiry_in(&self, env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
        let timeout = echo_timeout(env)?;
        let elapsed = self
            .echo_message
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .visible_for(Some(timeout))?;
        timeout.checked_sub(elapsed).filter(|left| !left.is_zero())
    }

    /// Returns how long the host may wait for input before FRAME, the frame it
    /// has just drawn, wants drawing again, or `None` when it may wait for
    /// ever: the soonest of the echo message expiring, the colouring going on,
    /// a change made in the background, and a callback owed.
    ///
    /// The editor knows which of these are pending and how soon each matters,
    /// so the host asks rather than knowing them; an idle editor answers
    /// `None`, and costs nothing. A host that waits for input also registers
    /// [`Self::on_background_change`], which wakes it when a job has changed
    /// something, and asks again here.
    pub fn next_redraw_in(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        frame: &FrameSnapshot,
    ) -> Option<Duration> {
        // Colour arrives a step of the highlighter at a time: coming back
        // every interval shows it as it lands.
        let colouring = frame.colouring_pending.then_some(TURN_INTERVAL);
        // Something has changed in the background since the frame was composed
        // -- a job's step, a line of a command's output -- and is drawn a turn
        // after the last frame at the soonest.
        let stepped = self.background_change.due_in();
        // A callback owed is work to do now, not something to wait for: zero,
        // and the host calls `tick`.
        let owed = self.callbacks_owed().then_some(Duration::ZERO);
        [self.echo_expiry_in(env), colouring, stepped, owed]
            .into_iter()
            .flatten()
            .min()
    }

    /// Returns what the focused window's delimiter is paired with: its
    /// buffer, its mode and the mode's table, read one lock at a time.
    fn paren_table(&self) -> Option<ParenTable> {
        let buffer = self.focused_window_buffer()?;
        let mode = self.with_buffer(&buffer, |buf| buf.current_mode.clone())?;
        let table = self.syntax_table(&mode);
        Some(ParenTable {
            buffer,
            mode,
            table,
        })
    }

    /// Returns the matches to draw: those of the search in progress, in the
    /// whole of its buffer, or else those of the replace in progress, in what
    /// is left of its region; `None` when neither has a pattern.
    fn search_marks(&self) -> Option<SearchMarks> {
        self.runtime(|runtime| {
            if let Some(search) = runtime.isearch() {
                return search.pattern.clone().map(|pattern| SearchMarks {
                    buffer: search.buffer.clone(),
                    pattern,
                    from: 0,
                    to: usize::MAX,
                });
            }
            let replace = runtime.replace().filter(|replace| !replace.done)?;
            Some(SearchMarks {
                buffer: replace.buffer.clone(),
                pattern: replace.pattern.clone(),
                from: replace.from,
                to: replace.limit,
            })
        })
    }

    /// Shows MSG in the echo area, and starts its timeout.
    pub fn set_echo_message(&self, msg: &str) {
        *self
            .echo_message
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = EchoMessage::new(msg);
    }
}
