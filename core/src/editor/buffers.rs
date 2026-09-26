//! The buffer half of the facade: making buffers, showing them, closing them,
//! and reaching into one.
//!
//! The compartment is [`crate::managers::Buffers`]. What lives here is the
//! coordination it is not allowed to do -- repointing windows at a
//! replacement, running a mode's `after-close-hook`, keeping the current
//! buffer and the focused window in step.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Open a new empty buffer or load a file into a new buffer if a path is
    /// provided.
    pub(crate) fn new_buffer(
        &self,
        name: &str,
        path: Option<&str>,
        start_mode: Option<String>,
    ) -> Option<String> {
        if let Some(file_path) = path {
            match std::fs::read_to_string(file_path) {
                Ok(content) => {
                    let mut new_buf = Buffer::from_text(name, &content);
                    // An explicit mode wins; otherwise the file's own name
                    // decides, which is how a language module ever gets used.
                    new_buf.current_mode = start_mode
                        .or_else(|| self.auto_mode_for(file_path))
                        .unwrap_or_else(|| "fundamental-mode".into());
                    new_buf.file_path = Some(file_path.to_string());

                    self.buffers_mut(|buffers| buffers.insert(name, new_buf));
                    self.show_in_focused_window(name);

                    Some(name.to_string())
                }
                Err(e) => {
                    self.log_diagnostic(&format!("Error reading file: {}", e));
                    None
                }
            }
        } else {
            let mut new_buf = Buffer::new(name);
            if let Some(mode_name) = start_mode {
                new_buf.current_mode = mode_name;
            }
            self.buffers_mut(|buffers| buffers.insert(name, new_buf));
            Some(name.to_string())
        }
    }

    /// An empty buffer that will be saved to PATH, for a file that is not
    /// there yet.
    ///
    /// # Why this is not `new_buffer` with a path
    ///
    /// That one reads the file, and reports failure when it cannot. A file
    /// that does not exist is not a failure here -- it is the ordinary way a
    /// file gets created, by opening it and typing. So this makes the empty
    /// buffer and remembers where it goes.
    ///
    /// The buffer is *not* modified. An empty buffer with no changes is the
    /// truth: nothing has been written, so leaving without saving loses
    /// nothing and should ask nothing. `C-x C-s` creates the file, because
    /// saving writes `file_path` whether or not anything was typed.
    pub(crate) fn new_file_buffer(
        &self,
        name: &str,
        path: &str,
        start_mode: Option<String>,
    ) -> String {
        let mut new_buf = Buffer::new(name);
        new_buf.current_mode = start_mode
            .or_else(|| self.auto_mode_for(path))
            .unwrap_or_else(|| "fundamental-mode".into());
        new_buf.file_path = Some(path.to_string());
        self.buffers_mut(|buffers| buffers.insert(name, new_buf));
        self.show_in_focused_window(name);
        name.to_string()
    }

    /// Make the buffer named NAME the one shown in the focused window and
    /// the current buffer. Returns `false` (logging a diagnostic) if no
    /// buffer named NAME exists, `true` otherwise. Shared by the
    /// `switch-to-buffer` primitive and the built-in minibuffer's cleanup.
    pub(crate) fn switch_to_buffer(&self, name: &str) -> bool {
        if !self.has_buffer(name) {
            self.log_diagnostic(&format!("[LOG] buffer {} does not exist.", name));
            return false;
        }
        self.show_in_focused_window(name);
        true
    }

    // ---------------------------------------------------------------
    // The window compartment
    // ---------------------------------------------------------------

    /// Read the buffer named NAME. `None` when there is no such buffer.
    ///
    /// # Why the table's lock is not held while F runs
    ///
    /// The handle is cloned out and the table's lock given back *before* the
    /// buffer's is taken. Holding both would make the table a bottleneck on
    /// every keystroke: the highlighter and the prescanner walk it from their
    /// own threads, and a scan that had to wait for whoever was typing -- or a
    /// keystroke that had to wait for a scan -- is exactly what those threads
    /// exist to avoid.
    ///
    /// The closure gets a locked buffer and cannot keep it. That is the whole
    /// difference from the `get_buffer` this replaced, which handed back an
    /// `Arc` and left every caller to remember which lock to take and when to
    /// let it go.
    pub(crate) fn with_buffer<R>(&self, name: &str, f: impl FnOnce(&Buffer<B>) -> R) -> Option<R> {
        let handle = self.buffers(|buffers| buffers.handle(name))?;
        let guard = handle.read().expect("read lock on buffer");
        Some(f(&guard))
    }

    /// Change the buffer named NAME. `None` when there is no such buffer.
    pub(crate) fn with_buffer_mut<R>(
        &self,
        name: &str,
        f: impl FnOnce(&mut Buffer<B>) -> R,
    ) -> Option<R> {
        let handle = self.buffers(|buffers| buffers.handle(name))?;
        let mut guard = handle.write().expect("write lock on buffer");
        Some(f(&mut guard))
    }

    /// Read the current buffer.
    ///
    /// Infallible, unlike [`EditorState::with_buffer`]: there is always a
    /// current buffer, and [`Buffers`] is what makes that true rather than
    /// hopeful.
    pub(crate) fn with_current_buffer<R>(&self, f: impl FnOnce(&Buffer<B>) -> R) -> R {
        let handle = self.buffers(|buffers| buffers.current_handle());
        let guard = handle.read().expect("read lock on buffer");
        f(&guard)
    }

    /// Change the current buffer.
    pub(crate) fn with_current_buffer_mut<R>(&self, f: impl FnOnce(&mut Buffer<B>) -> R) -> R {
        let handle = self.buffers(|buffers| buffers.current_handle());
        let mut guard = handle.write().expect("write lock on buffer");
        f(&mut guard)
    }

    /// Whether a buffer named NAME exists.
    pub(crate) fn has_buffer(&self, name: &str) -> bool {
        self.buffers(|buffers| buffers.contains(name))
    }

    /// Show the buffer named NAME in the focused window, and make it current.
    ///
    /// # Why this is a method and not five lines at each call site
    ///
    /// It is the mirror of [`EditorState::set_focused_window_id`]. That one
    /// moves focus and brings the current buffer along; this one changes the
    /// buffer and leaves focus where it is. Between them they are every way
    /// the pair (focused window, current buffer) is allowed to change, and
    /// keeping them in step is the whole job -- let them drift and the editor
    /// draws a cursor in one window and types into another.
    ///
    /// It was five lines at each of three call sites, and all three named the
    /// focused window and then edited whichever window the lookup handed back
    /// -- which, until [`LayoutNode::window_mut`] was fixed, was the leftmost
    /// one. Three copies is also three places for the next such bug to be
    /// fixed in only two of.
    fn show_in_focused_window(&self, name: &str) {
        self.windows_mut(|windows| windows.show_in_focused(name));
        // After the window lock is released: the current buffer name sits
        // before the windows in the canonical order, so taking it while
        // holding them would invert the two.
        self.set_current_buffer_name(name);
    }

    // ---------------------------------------------------------------
    // Splitting, closing and cycling through windows
    // ---------------------------------------------------------------

    /// Close the buffer named NAME: detach it from whatever window is
    /// showing it (a floating window is removed outright and focus
    /// returns to whatever had it before that floating window opened;
    /// a tiled window falls back to `*scratch*`, since there's no
    /// per-window buffer history to fall back to yet), run that
    /// buffer's major mode's `after-close-hook`, then remove it from
    /// the buffer table. If NAME was the last remaining buffer, a fresh
    /// empty `*scratch*` is created so the editor is never left with
    /// none. Returns `false` if no buffer named NAME exists, `true`
    /// otherwise.
    pub fn close_buffer(&self, name: &str, env: &Arc<Env<EditorState<B>>>) -> bool {
        let Some(closing_mode) = self.with_buffer(name, |buf| buf.current_mode.clone()) else {
            return false;
        };

        // Detach NAME from wherever it's currently displayed.
        //
        // Finding the float and removing it are now one acquisition rather
        // than a read followed by a write. They were never safe apart: between
        // the two, another thread opening or closing a float shifts the index,
        // and what came back was a *different* window than the one looked for.
        let restore_id = self.windows_mut(|windows| {
            let idx = windows
                .floating()
                .iter()
                .position(|f| f.window.buffer_name == name)?;
            Some(
                windows
                    .floating_mut()
                    .remove(idx)
                    .previous_focused_window_id,
            )
        });
        // Outside the closure, because moving focus makes a buffer current and
        // takes the window lock again to find out which.
        if let Some(restore_id) = restore_id {
            self.set_focused_window_id(restore_id);
        }

        // *Every* tiled window showing it, not only the focused one.
        //
        // # The bug this fixes
        //
        // This used to repoint the focused window alone. Open one buffer in
        // two windows, kill it, and the other window was left naming a buffer
        // that no longer existed. Nothing complained until focus moved there
        // -- at which point `current_buffer_name` became that dead name and
        // the next `get_current_buffer` hit its "Corruption in the hashmap of
        // buffers" panic, taking the editor down with whatever was unsaved in
        // the other windows.
        //
        // Worked out before the window lock is taken: `most_recent_buffer`
        // reads the buffer compartment, and taking that while holding the
        // windows would invert the canonical lock order.
        let replacement = self
            .most_recent_buffer(name)
            .unwrap_or_else(|| "*scratch*".to_string());
        self.windows_mut(|windows| {
            windows.root_mut().each_window_mut(&mut |window| {
                if window.buffer_name == name {
                    window.show(&replacement);
                }
            })
        });

        // Removing it and settling what is current are one acquisition. They
        // used to be four -- remove, re-add `*scratch*` if that emptied the
        // table, read the current name, then set it -- and between any two of
        // them the current name could be read by another thread while it
        // named the buffer that had just gone. That read is the panic
        // described above.
        if let BufferRemoved::Current(successor) = self.buffers_mut(|buffers| buffers.remove(name))
        {
            // The compartment picked the most recent survivor. Prefer the one
            // the windows were just repointed at, when it is still there, so
            // that what is current and what is on screen agree -- them
            // disagreeing is how the panic above was reached in the first
            // place.
            if replacement != successor {
                self.set_current_buffer_name(&replacement);
            }
        }

        self.run_hook(&closing_mode, "after-close-hook", env);
        true
    }

    /// Whether a minibuffer prompt is currently open.
    pub(crate) fn minibuffer_is_open(&self) -> bool {
        self.has_buffer("*Minibuffer*")
    }

    // ---------------------------------------------------------------
    // What the previous command was, and where vertical movement is aiming
    // ---------------------------------------------------------------

    /// Every live buffer's name, sorted. Used for buffer-name completion.
    pub(crate) fn buffer_names(&self) -> Vec<String> {
        self.buffers(|buffers| buffers.names())
    }

    /// Every live buffer's name with the focused window's at the head.
    ///
    /// For the background walkers, which both want to reach what is on screen
    /// before what is not. The focused window is asked *before* the table is
    /// taken: two compartments, never held together.
    pub(crate) fn buffer_names_focused_first(&self) -> Vec<String> {
        let focused = self.focused_window_buffer();
        self.buffers(|buffers| buffers.names_with_first(focused.as_deref()))
    }

    /// Get the name of the current buffer
    pub(crate) fn get_current_buffer_name(&self) -> String {
        self.current_buffer_name_shared().to_string()
    }

    /// The current buffer's name without copying it.
    pub(crate) fn current_buffer_name_shared(&self) -> Arc<str> {
        self.buffers(|buffers| buffers.current_name())
    }

    /// Set the name of the current buffer.
    ///
    /// Silently does nothing when there is no such buffer, which is the
    /// refusal that keeps the invariant: it is no longer possible from
    /// anywhere in the editor to leave the current name pointing at a buffer
    /// the table does not hold.
    pub(crate) fn set_current_buffer_name(&self, name: &str) {
        self.buffers_mut(|buffers| buffers.make_current(name));
    }

    /// The most recently current buffer that still exists and is not EXCEPT.
    ///
    /// `None` when there is no such buffer. The caller answers that for
    /// itself -- there is always `*scratch*`, but falling back to it is a
    /// policy this does not get to make.
    pub(crate) fn most_recent_buffer(&self, except: &str) -> Option<String> {
        self.buffers(|buffers| buffers.most_recent(except))
    }

    /// Make NAME the current buffer without showing it, and give back whatever
    /// was current before.
    ///
    /// This is Emacs' `set-buffer`, and the difference from `switch-to-buffer`
    /// is the whole reason it exists: that one also points the focused window
    /// at the buffer, which is right when a person asked to see it and wrong
    /// when a piece of Lisp merely wants to *act* on it. Rebinding the window
    /// for the duration of a computation and putting it back would be a window
    /// doing something nobody asked for.
    ///
    /// Returns `None` when there is no such buffer, having changed nothing.
    pub(crate) fn set_current_buffer(&self, name: &str) -> Option<Arc<str>> {
        self.buffers_mut(|buffers| buffers.make_current(name))
    }
}
