//! What background jobs owe the thread that runs commands, and when it pays.
//!
//! # Why there is a queue at all
//!
//! A job runs on the worker thread, and the thing it was started for -- a
//! completion list, a listing, an index -- belongs to a mode. Letting the job
//! call the mode directly would be letting Lisp written for the command
//! thread run at an arbitrary instant on another one: half-way through an
//! insertion, inside a hook that is walking a list it is also changing,
//! between the two halves of something that is one operation to everybody
//! except the thread that interrupted it.
//!
//! No care in the callback fixes that, because the callback is not what is
//! wrong. So a job does not call; it leaves a note saying what should be
//! called, and the note is read where a keystroke would be read. See
//! [`crate::worker::BackgroundJob`] for the rule this is the machinery for.
//!
//! # Why it is a field and not a compartment
//!
//! Every compartment in [`crate::managers`] knows nothing of Lisp, and this
//! queue is full of it. Putting it behind one would either break that rule or
//! require a second representation of a function call, invented for the
//! purpose of not admitting what it is. It sits beside `current_results` as
//! what it is: state the facade holds directly.
use super::*;

/// One call a job has asked for, waiting for the command thread.
pub(crate) struct OwedCallback<B: BufferTrait> {
    /// The job that asked, so that a job superseded since can be dropped.
    name: String,
    generation: u64,
    function: ELispExp<B>,
    args: Vec<ELispExp<B>>,
}

impl<B: BufferTrait> EditorState<B> {
    /// Note that the job born as GENERATION of NAME wants FUNCTION called
    /// with ARGS.
    ///
    /// Called from the worker thread. It does not call anything, take a lock
    /// anyone is holding, or wake anybody: the renderer is already awake
    /// while a job is running, and [`Self::callbacks_owed`] keeps it awake
    /// for the moment between the last turn and the drain.
    pub(crate) fn owe_callback(
        &self,
        name: &str,
        generation: u64,
        function: ELispExp<B>,
        args: Vec<ELispExp<B>>,
    ) {
        self.owed
            .write()
            .expect("write lock on owed callbacks")
            .push(OwedCallback {
                name: name.to_string(),
                generation,
                function,
                args,
            });
    }

    /// Whether anything is waiting to be run.
    ///
    /// Asked by [`Self::next_redraw_in`]. A job that has queued its last
    /// callback and then finished has taken the editor's count of running
    /// work to zero, and without this the renderer would decide there was
    /// nothing left to wait for and sleep on the note.
    pub fn callbacks_owed(&self) -> bool {
        !self
            .owed
            .read()
            .expect("read lock on owed callbacks")
            .is_empty()
    }

    /// Run everything owed, and say whether anything ran.
    ///
    /// # What this is and is not
    ///
    /// It is the point in the loop where a background job's results become
    /// visible, and the only one. A callback runs here with the same locks
    /// free and the same invariants holding as a keystroke, which is what
    /// makes "the mode keeps itself consistent" a rule a mode can keep.
    ///
    /// It is not a command. Nothing here sets `last-command`, offers a repeat
    /// key or runs `post-command-hook`: a callback is something the editor
    /// owed, not something the user did, and a mode reading `last-command`
    /// must not be told that an index finishing was the last thing that
    /// happened.
    ///
    /// The queue is taken whole rather than drained until empty, which bounds
    /// the pass: a job on the other thread that queues callbacks faster than
    /// they can be run would otherwise keep this loop going, and the thread
    /// it would be keeping is the one the user is typing on. Whatever arrives
    /// while this is running is run next time, a frame later.
    pub fn run_owed_callbacks(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        let owed: Vec<OwedCallback<B>> =
            std::mem::take(&mut *self.owed.write().expect("write lock on owed callbacks"));
        if owed.is_empty() {
            return false;
        }
        let mut ran = false;
        for callback in owed {
            // Dropped rather than run when a newer job has taken the name.
            // Running it would tell a mode that the job it is now waiting for
            // had finished, with the previous job's half-built state as the
            // evidence.
            //
            // The question is "is this still the newest", not "is this still
            // running": a job that has finished is precisely the one whose
            // callback has to be delivered.
            if !self
                .runtime(|runtime| runtime.worker_is_latest(&callback.name, callback.generation))
            {
                continue;
            }
            ran = true;
            // A budget of its own, like a hook and for the same reason: this
            // is a top-level entry into the interpreter, and a callback that
            // loops must be stopped by the same thing that stops a command
            // that loops.
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

    /// Everything the editor owes when its timer comes due, in one call.
    ///
    /// The renderer asks [`Self::next_redraw_in`] how long it may sleep and
    /// calls this when that time is up. One call rather than a list of them
    /// so that a second front end does not have to know what the editor was
    /// waiting for -- which is the same reason `next_redraw_in` answers with
    /// one duration rather than four.
    ///
    /// True when something changed and the screen is now behind.
    pub fn tick(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        // Both, always: `||` would skip the second whenever the first had
        // something to do, which is exactly when a job is most likely to have
        // left something too.
        let scrolled = self.drag_scroll_tick(env);
        let ran = self.run_owed_callbacks(env);
        scrolled || ran
    }
}
