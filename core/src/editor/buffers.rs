//! The buffers' half of the facade: making a buffer, showing one, closing one
//! -- with the windows repointed and the mode's hooks run -- and reaching into
//! one under its own lock.
//!
//! Design: `doc/architettura-editor.typ, <coppie>`.

use super::*;

impl<B: BufferTrait> EditorState<B> {
    /// Makes a buffer NAME in START_MODE: empty, or holding the text of the
    /// file at PATH, and then shown in the focused window, in the mode its name
    /// picks unless START_MODE says. Returns NAME, or `None` when the file
    /// could not be read.
    pub(crate) fn new_buffer(
        &self,
        name: &str,
        path: Option<&str>,
        start_mode: Option<String>,
        env: &Arc<Env<EditorState<B>>>,
    ) -> Option<String> {
        if let Some(file_path) = path {
            match std::fs::read_to_string(file_path) {
                Ok(content) => {
                    let mut new_buf = Buffer::from_text(name, &content);
                    // A mode asked for wins; otherwise the file's name decides,
                    // which is how a language's mode is ever chosen.
                    new_buf.current_mode = start_mode
                        .or_else(|| self.auto_mode_for(file_path))
                        .unwrap_or_else(|| "fundamental-mode".into());
                    new_buf.file_path = Some(file_path.to_string());
                    // Taken after the read, so that a file rewritten while it
                    // was read is noticed at the next look, not stamped as
                    // seen.
                    new_buf.file_stamp = FileStamp::of(file_path);

                    self.buffers_mut(|buffers| buffers.insert(name, new_buf));
                    self.show_in_focused_window(name);
                    self.initialize_buffer(name, true, env);

                    Some(name.to_string())
                }
                Err(e) => {
                    // TODO(FIL-3): a file that is not UTF-8 fails here, and
                    // only the log says so.
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
            self.initialize_buffer(name, false, env);
            Some(name.to_string())
        }
    }

    /// Makes an empty buffer NAME that saves to PATH, for a file that does not
    /// exist yet, shows it in the focused window, and returns NAME.
    ///
    /// Not modified: nothing has been written, so leaving it loses nothing and
    /// asks nothing, and `C-x C-s` creates the file, since saving writes
    /// whether or not anything was typed.
    pub(crate) fn new_file_buffer(
        &self,
        name: &str,
        path: &str,
        start_mode: Option<String>,
        env: &Arc<Env<EditorState<B>>>,
    ) -> String {
        let mut new_buf = Buffer::new(name);
        new_buf.current_mode = start_mode
            .or_else(|| self.auto_mode_for(path))
            .unwrap_or_else(|| "fundamental-mode".into());
        new_buf.file_path = Some(path.to_string());
        self.buffers_mut(|buffers| buffers.insert(name, new_buf));
        self.show_in_focused_window(name);
        self.initialize_buffer(name, true, env);
        name.to_string()
    }

    /// Runs the hooks of the buffer NAME, just made, in its mode and with it
    /// current, changing no window: `buffer-created-hook`, `major-mode-hook`,
    /// `after-change-major-mode-hook`, and `find-file-hook` when VISITING_FILE.
    /// Once per buffer made, never when one is selected; no lock is held while
    /// they run.
    pub(crate) fn initialize_buffer(
        &self,
        name: &str,
        visiting_file: bool,
        env: &Arc<Env<EditorState<B>>>,
    ) {
        self.run_buffer_hook(name, "buffer-created-hook", env);
        self.run_buffer_hook(name, "major-mode-hook", env);
        self.run_buffer_hook(name, "after-change-major-mode-hook", env);
        if visiting_file {
            self.run_buffer_hook(name, "find-file-hook", env);
        }
    }

    /// Runs the hook HOOK of the buffer NAME's mode with NAME current, and
    /// then makes current again the buffer that was.
    pub(crate) fn run_buffer_hook(&self, name: &str, hook: &str, env: &Arc<Env<EditorState<B>>>) {
        let Some(previous) = self.set_current_buffer(name) else {
            return; // An earlier callback may have closed the new buffer.
        };
        let mode = self.with_current_buffer(|buf| buf.current_mode.clone());
        self.run_hook(&mode, hook, env);
        self.set_current_buffer(&previous);
    }

    /// Renames the buffer OLD to NEW wherever it is known: in the table, in the
    /// buffer, in the windows. False when there is no OLD, or NEW belongs to
    /// another buffer.
    ///
    /// The table moves the entry and what is current, and answers; the windows
    /// follow before anything else can run, so that none names a buffer gone
    /// from under that name.
    pub(crate) fn rename_buffer(&self, old: &str, new: &str) -> bool {
        if self.buffers_mut(|buffers| buffers.rename(old, new)) == BufferRenamed::No {
            return false;
        }
        // The buffer's own idea of its name, which the table left alone: a
        // buffer's lock taken under the table's would put the table in the way
        // of whatever is writing to the buffer.
        self.with_buffer_mut(new, |buf| buf.name = new.to_string());
        self.windows_mut(|windows| windows.rename_buffer(old, new));
        true
    }

    /// Shows the buffer NAME in the focused window, and makes it current;
    /// false, with a line in the log, when there is no such buffer. For
    /// `switch-to-buffer`, and for the minibuffer's cleanup.
    pub(crate) fn switch_to_buffer(&self, name: &str) -> bool {
        if !self.has_buffer(name) {
            self.log_diagnostic(&format!("[LOG] buffer {} does not exist.", name));
            return false;
        }
        self.show_in_focused_window(name);
        true
    }

    /// Runs F on the buffer NAME, under its read lock; `None` when there is no
    /// such buffer.
    ///
    /// The table's lock is given back before the buffer's is taken: the
    /// highlighter and the prescanner walk the table from their own threads,
    /// and must not wait for whoever is typing. F gets the buffer locked, and
    /// cannot keep it.
    pub(crate) fn with_buffer<R>(&self, name: &str, f: impl FnOnce(&Buffer<B>) -> R) -> Option<R> {
        let handle = self.buffers(|buffers| buffers.handle(name))?;
        let guard = handle.read().expect("read lock on buffer");
        Some(f(&guard))
    }

    /// Runs F on the buffer NAME, under its write lock; `None` when there is no
    /// such buffer.
    pub(crate) fn with_buffer_mut<R>(
        &self,
        name: &str,
        f: impl FnOnce(&mut Buffer<B>) -> R,
    ) -> Option<R> {
        let handle = self.buffers(|buffers| buffers.handle(name))?;
        let mut guard = handle.write().expect("write lock on buffer");
        Some(f(&mut guard))
    }

    /// Runs F on the current buffer, under its read lock. There is always one:
    /// [`Buffers`] makes sure of it.
    pub(crate) fn with_current_buffer<R>(&self, f: impl FnOnce(&Buffer<B>) -> R) -> R {
        let handle = self.buffers(|buffers| buffers.current_handle());
        let guard = handle.read().expect("read lock on buffer");
        f(&guard)
    }

    /// Runs F on the current buffer, under its write lock.
    pub(crate) fn with_current_buffer_mut<R>(&self, f: impl FnOnce(&mut Buffer<B>) -> R) -> R {
        let handle = self.buffers(|buffers| buffers.current_handle());
        let mut guard = handle.write().expect("write lock on buffer");
        f(&mut guard)
    }

    /// Returns whether there is a buffer NAME.
    pub(crate) fn has_buffer(&self, name: &str) -> bool {
        self.buffers(|buffers| buffers.contains(name))
    }

    /// Appends TEXT to the buffer NAME, read-only or not; false when there is
    /// no such buffer, or the write was refused.
    ///
    /// A transcript is read-only so that nobody types into it. Its flag goes
    /// down and back up inside one write lock, so that it is never writable
    /// while anybody else can see it.
    pub(crate) fn append_to_buffer(&self, name: &str, text: &str) -> bool {
        self.with_buffer_mut(name, |buf| append(buf, text)) == Some(true)
    }

    /// Appends TEXT to the buffer NAME as [`Self::append_to_buffer`] does, for
    /// the work WRITER, and only while the buffer is still the one it writes
    /// into.
    ///
    /// A name is given out again: close a command's output and start another,
    /// and the new buffer may take the old one's name while the old command,
    /// stopping, has a last line to write. Asking under the write's own lock
    /// keeps that line out of the new output.
    pub(crate) fn append_as_writer(&self, name: &str, writer: WorkId, text: &str) -> bool {
        self.with_buffer_mut(name, |buf| buf.writer == Some(writer) && append(buf, text))
            == Some(true)
    }

    /// Records that the work WRITER has stopped writing into NAME, if it was.
    pub(crate) fn release_writer(&self, name: &str, writer: WorkId) {
        self.with_buffer_mut(name, |buf| {
            if buf.writer == Some(writer) {
                buf.writer = None;
            }
        });
    }

    /// Returns STEM, or `STEM<2>`, `STEM<3>`, the first that no buffer has. For
    /// a buffer that work will write into, see [`Self::free_name_for_work`].
    pub(crate) fn free_buffer_name(&self, stem: &str) -> String {
        // TODO(DOC-5): nothing calls it.
        if !self.has_buffer(stem) {
            return stem.to_string();
        }
        for n in 2.. {
            let candidate = format!("{stem}<{n}>");
            if !self.has_buffer(&candidate) {
                return candidate;
            }
        }
        unreachable!("the loop above returns")
    }

    /// Shows the buffer NAME in the focused window, and makes it current.
    ///
    /// The mirror of [`Self::set_focused_window_id`], which moves focus and
    /// brings the current buffer along: between them they are every way the
    /// focused window and the current buffer change, and they keep the two in
    /// step.
    fn show_in_focused_window(&self, name: &str) {
        self.windows_mut(|windows| windows.show_in_focused(name));
        // Once the windows are let go: a door's closure does not call back
        // into the editor.
        self.set_current_buffer_name(name);
    }

    /// Closes the buffer NAME, and returns false when there is no such buffer.
    ///
    /// A float showing it closes, giving focus back to the window that had it.
    /// Every tiled window showing it shows instead the buffer most recently
    /// current, or `*scratch*`, and the buffer leaves the table, a fresh
    /// `*scratch*` taking the place of the last one. The work writing into it
    /// stops, and its mode's `after-close-hook` runs. Its auto-save file is
    /// deleted, unless it has unsaved changes.
    pub fn close_buffer(&self, name: &str, env: &Arc<Env<EditorState<B>>>) -> bool {
        let Some((closing_mode, writer)) =
            self.with_buffer(name, |buf| (buf.current_mode.clone(), buf.writer))
        else {
            return false;
        };

        // A buffer with unsaved changes keeps its copy. This asks nothing, and
        // neither do `close-buffer` and `kill-buffer-without-saving`, which
        // call it: the copy may be all that is left of the work. It goes when
        // what it held is in the file -- on a save, and here when nothing is
        // outstanding.
        let unsaved = self
            .with_buffer(name, |buf| buf.is_modified && buf.file_path.is_some())
            .unwrap_or(false);
        if !unsaved {
            self.discard_auto_save(name, auto_save_directory(env).as_deref());
        }

        // Found and removed in one acquisition: between the two, another
        // thread opening or closing a float would shift the indices. Every
        // float showing it goes, since one left would name a buffer that is
        // gone; focus returns to where the first of them took it from, which
        // was opened before the others and so is none of them.
        let restore_id = self.windows_mut(|windows| {
            let mut restore = None;
            windows.floating_mut().retain(|f| {
                if f.window.buffer_name != name {
                    true
                } else {
                    restore.get_or_insert(f.previous_focused_window_id);
                    false
                }
            });
            restore
        });
        
        // Outside the closure: moving focus makes a buffer current, which takes
        // the windows' lock again.
        if let Some(restore_id) = restore_id {
            self.set_focused_window_id(restore_id);
        }

        // Every tiled window showing it, not only the focused one: a window
        // left naming a closed buffer would make it current again once
        // focused. The replacement is found before the windows are taken: a
        // door's closure does not call back into the editor.
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

        // Removing it and settling what is current in one acquisition: between
        // two, another thread could read a current name naming a buffer that
        // had just gone.
        let last_buffer = self.buffers(|buffers| buffers.names().len() == 1);
        if let BufferRemoved::Current(successor) = self.buffers_mut(|buffers| buffers.remove(name))
        {
            // The table picked the most recent survivor. What is current should
            // be what the windows were just pointed at, when that is still
            // there.
            if replacement != successor {
                self.set_current_buffer_name(&replacement);
            }
        }

        if last_buffer {
            self.initialize_buffer("*scratch*", false, env);
        }

        // The work writing into it from a thread of its own stops with it:
        // output with nowhere to go is a process nobody can see or stop. Asked
        // once the buffer has left the table, so that nothing written from then
        // on can land.
        if let Some(writer) = writer {
            self.work.cancel(writer);
        }

        self.run_hook(&closing_mode, "after-close-hook", env);
        true
    }

    /// Returns whether a minibuffer prompt is open.
    pub(crate) fn minibuffer_is_open(&self) -> bool {
        self.has_buffer(crate::feature::minibuffer::PROMPT_BUFFER)
    }

    /// Returns every buffer's name, sorted, for completing a buffer's name.
    pub(crate) fn buffer_names(&self) -> Vec<String> {
        self.buffers(|buffers| buffers.names())
    }

    /// Returns every buffer's name, the focused window's first: the background
    /// walkers reach what is on screen before what is not.
    ///
    /// The focused window is asked before the table is taken: one compartment
    /// at a time.
    pub(crate) fn buffer_names_focused_first(&self) -> Vec<String> {
        let focused = self.focused_window_buffer();
        self.buffers(|buffers| buffers.names_with_first(focused.as_deref()))
    }

    /// Returns the current buffer's name.
    pub(crate) fn get_current_buffer_name(&self) -> String {
        self.current_buffer_name_shared().to_string()
    }

    /// Returns the current buffer's name, without copying it.
    fn current_buffer_name_shared(&self) -> Arc<str> {
        self.buffers(|buffers| buffers.current_name())
    }

    /// Makes NAME the current buffer; nothing when there is no such buffer,
    /// the refusal that keeps the current name naming a buffer of the table.
    pub(crate) fn set_current_buffer_name(&self, name: &str) {
        self.buffers_mut(|buffers| buffers.make_current(name));
    }

    /// Returns the buffer most recently current that is not EXCEPT; `None`
    /// when there is none. Falling back to `*scratch*` is the caller's choice.
    pub(crate) fn most_recent_buffer(&self, except: &str) -> Option<String> {
        self.buffers(|buffers| buffers.most_recent(except))
    }

    /// Makes NAME the current buffer without showing it, and returns the one
    /// current before; `None`, changing nothing, when there is no such buffer.
    ///
    /// Emacs' `set-buffer`. `switch-to-buffer` also points the focused window
    /// at the buffer, which is right when somebody asked to see it and wrong
    /// when Lisp only means to act on it.
    pub(crate) fn set_current_buffer(&self, name: &str) -> Option<Arc<str>> {
        self.buffers_mut(|buffers| buffers.bind_current(name))
    }
}

/// Appends TEXT to BUF, read-only or not, and returns whether it was written.
/// The flag goes down and back up under the caller's lock: see
/// [`EditorState::append_to_buffer`].
fn append<B: BufferTrait>(buf: &mut Buffer<B>, text: &str) -> bool {
    let was_read_only = buf.read_only;
    buf.read_only = false;
    let at = buf.text.len();
    // `insert_text` refuses only a read-only buffer, and the flag is down under
    // this same lock. Checked all the same: a new reason to refuse is reported
    // rather than the text going missing.
    let written = crate::primitives::edits::insert_text(buf, at, text);
    buf.read_only = was_read_only;
    written
}
