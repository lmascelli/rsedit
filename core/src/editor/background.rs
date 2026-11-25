//! What background jobs owe the thread that runs commands, and when it pays: a
//! job does not call Lisp from its own thread, it leaves the call owed, and the
//! command thread makes it between one command and the next.
//!
//! Design: `doc/architettura-editor.typ, <callback-dovute>`.
use super::*;

/// How much text, in characters, the buffers closed must have freed before
/// the allocator is asked to give back what it holds free: enough that closing
/// a prompt asks nothing, little enough that closing a file does.
pub(crate) const GIVE_BACK_AFTER: usize = 64 * 1024;

/// The buffers closed and not yet freed, each with its length, and how much
/// text those freed since the allocator was last asked.
pub(crate) struct Released<B: BufferTrait> {
    closing: Vec<(std::sync::Weak<RwLock<Buffer<B>>>, usize)>,
    freed: usize,
}

impl<B: BufferTrait> Default for Released<B> {
    fn default() -> Self {
        Self {
            closing: Vec::new(),
            freed: 0,
        }
    }
}

/// Gives the system back the pages the allocator holds free: glibc's
/// `malloc_trim`, which goes through every arena, the threads' as well as the
/// main one, and not only the top of the heap.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn trim_heap() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> std::ffi::c_int;
    }
    // SAFETY: it is handed no pointer, and takes each arena's lock itself.
    unsafe {
        malloc_trim(0);
    }
}

/// Elsewhere there is no such call, and the allocator is left as it is.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn trim_heap() {}

/// One call a job has asked for, waiting for the command thread.
pub(crate) struct OwedCallback<B: BufferTrait> {
    /// The job that asked, by its name and number: the call of one replaced
    /// or stopped since is dropped.
    name: String,
    id: WorkId,
    function: ELispExp<B>,
    args: Vec<ELispExp<B>>,
    /// The directory the thread that owed it was scoped to: the call runs in
    /// it.
    directory: Option<String>,
}

impl<B: BufferTrait> EditorState<B> {
    /// Records that the job ID, under NAME, wants FUNCTION called with ARGS,
    /// in the directory the calling thread is scoped to.
    ///
    /// Called from the executor. It calls nothing, takes no lock that is held
    /// for long, and wakes nobody: the step it is called from wakes the host
    /// as it ends, and [`Self::callbacks_owed`] then tells the host that there
    /// is something to do.
    pub(crate) fn owe_callback(
        &self,
        name: &str,
        id: WorkId,
        function: ELispExp<B>,
        args: Vec<ELispExp<B>>,
    ) {
        self.owed
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(OwedCallback {
                name: name.to_string(),
                id,
                function,
                args,
                directory: super::scoped_directory(),
            });
    }

    /// Returns whether a call is owed: a job's, a file operation's, or a
    /// process's output waiting for its function; or whether a buffer closed
    /// has been freed, and its memory may be given back.
    ///
    /// [`Self::next_redraw_in`] asks. A job that has owed its last call has
    /// finished, and nothing else would tell the host that something is still
    /// to be done for it.
    pub fn callbacks_owed(&self) -> bool {
        self.file_io_ready()
            || self.process_output_waiting()
            || self.released_freed()
            || !self
                .owed
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
    }

    /// Makes every call owed, and returns whether any was made.
    ///
    /// Where a job's results become visible, and the only place: a call runs
    /// here with the locks free and the invariants holding, as for a key. It
    /// is not a command -- no `last-command`, no repeat key, no
    /// `post-command-hook` -- since the editor owed it and the user did not do
    /// it. The list is taken whole: what is owed while it runs waits for the
    /// next pass, so that a job owing faster than this pays cannot keep the
    /// thread the user types on.
    pub fn run_owed_callbacks(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        let owed: Vec<OwedCallback<B>> = std::mem::take(
            &mut *self
                .owed
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        if owed.is_empty() {
            return false;
        }
        let mut ran = false;
        for callback in owed {
            // Dropped, not made, when a newer job has the name or the job was
            // stopped: it would tell a mode that the job it now waits for had
            // finished, with the old job's half-built state for evidence. The
            // question is whether the job still speaks for its name, not
            // whether it runs: a finished job is the one whose call is owed.
            if !self.work.is_latest(&callback.name, callback.id) {
                continue;
            }
            ran = true;
            // A budget of its own, as for a hook: an entry into the
            // interpreter, where a loop is stopped as a command's is.
            let _command = self.begin_command();
            let _scope = super::DirectoryScope::enter(callback.directory);
            if let Err(error) = call_callable(&callback.function, &callback.args, env.clone(), self)
            {
                self.log_diagnostic(&format!(
                    "[ERROR] the callback of background job {} failed: {error:?}",
                    callback.name
                ));
            }
        }
        ran
    }

    /// Does what the editor owes when the host's wait has run out: scrolls a
    /// drag that has left its window, makes the calls owed, and delivers some
    /// of the processes' output. Returns whether the screen is now behind.
    ///
    /// One call, so that a host need not know what the editor was waiting
    /// for, as [`Self::next_redraw_in`] answers with one duration.
    pub fn tick(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        // Both, always: `||` would skip the second whenever the first had
        // work, which is when a job is likeliest to have left some too.
        let scrolled = self.drag_scroll_tick(env);
        let ran = self.run_owed_callbacks(env);
        let files = self.run_file_io(env);
        let output = self.run_process_output(env);
        // After the file operations: the last to hold a buffer closed.
        self.give_back_memory();
        scrolled || ran || files || output
    }

    /// Records that the buffer NAME is closing. Its memory is freed once the
    /// last holder lets it go, which may be after the close: a file operation
    /// it started still holds it.
    pub(crate) fn note_released(&self, name: &str) {
        let Some(handle) = self.file_handle(name) else {
            return;
        };
        let length = handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .text
            .len();
        self.released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closing
            .push((Arc::downgrade(&handle), length));
    }

    /// Returns whether a buffer closed has been freed since the last
    /// [`Self::give_back_memory`].
    fn released_freed(&self) -> bool {
        self.released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closing
            .iter()
            .any(|(buffer, _)| buffer.strong_count() == 0)
    }

    /// Asks the allocator to give the system back the memory it holds free,
    /// once the buffers closed have freed [`GIVE_BACK_AFTER`] characters of
    /// text since it was last asked; and returns whether it was asked.
    ///
    /// What is freed the allocator keeps for what comes next, so closing a
    /// buffer frees its memory and the editor's footprint stays. Between
    /// commands rather than in [`Self::close_buffer`]: what the close leaves
    /// running may hold the buffer a while yet, and asking holds each arena
    /// of the allocator for a time that grows with the heap.
    pub(crate) fn give_back_memory(&self) -> bool {
        {
            let mut released = self
                .released
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut freed = 0;
            released.closing.retain(|(buffer, length)| {
                let held = buffer.strong_count() > 0;
                if !held {
                    freed += length;
                }
                held
            });
            released.freed += freed;
            if released.freed < GIVE_BACK_AFTER {
                return false;
            }
            released.freed = 0;
        }
        trim_heap();
        true
    }
}
