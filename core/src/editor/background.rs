//! What background jobs owe the thread that runs commands, and when it pays: a
//! job does not call Lisp from its own thread, it leaves the call owed, and the
//! command thread makes it between one command and the next.
//!
//! Design: `doc/architettura-editor.typ, <callback-dovute>`.
use super::*;

/// One call a job has asked for, waiting for the command thread.
pub(crate) struct OwedCallback<B: BufferTrait> {
    /// The job that asked, by its name and number: the call of one replaced
    /// or stopped since is dropped.
    name: String,
    id: WorkId,
    function: ELispExp<B>,
    args: Vec<ELispExp<B>>,
}

impl<B: BufferTrait> EditorState<B> {
    /// Records that the job ID, under NAME, wants FUNCTION called with ARGS.
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
            .expect("write lock on owed callbacks")
            .push(OwedCallback {
                name: name.to_string(),
                id,
                function,
                args,
            });
    }

    /// Returns whether a call is owed.
    ///
    /// [`Self::next_redraw_in`] asks. A job that has owed its last call has
    /// finished, and nothing else would tell the host that something is still
    /// to be done for it.
    pub fn callbacks_owed(&self) -> bool {
        !self
            .owed
            .read()
            .expect("read lock on owed callbacks")
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
        let owed: Vec<OwedCallback<B>> =
            std::mem::take(&mut *self.owed.write().expect("write lock on owed callbacks"));
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
    /// drag that has left its window, and makes the calls owed. Returns
    /// whether the screen is now behind.
    ///
    /// One call, so that a host need not know what the editor was waiting
    /// for, as [`Self::next_redraw_in`] answers with one duration.
    pub fn tick(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        // Both, always: `||` would skip the second whenever the first had
        // work, which is when a job is likeliest to have left some too.
        let scrolled = self.drag_scroll_tick(env);
        let ran = self.run_owed_callbacks(env);
        scrolled || ran
    }
}
