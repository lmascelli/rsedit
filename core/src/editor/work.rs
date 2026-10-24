//! The facade over the WorkManager: what the host, the primitives and the tests
//! ask of the editor's asynchronous work, the topics the editor emits, which
//! say "go and look" where each change of their kind is made, and how the host
//! hears that the work has changed the screen.
//!
//! Design: `doc/architettura-editor.typ, <utenti-sfondo>`.
use super::*;
use std::cell::Cell;
use std::sync::Mutex;

/// A buffer's text changed, or a buffer came in: a colouring or a scan may be
/// out of date. Which buffer is for the job to find out, from the buffers'
/// versions.
pub(crate) const BUFFER_CHANGED: &str = "buffer-changed";

/// A grammar or a syntax table changed: what was coloured, or scanned, under
/// the old one may be out of date.
pub(crate) const SYNTAX_CHANGED: &str = "syntax-changed";

thread_local! {
    /// Whether this thread is in a step of a Lisp job, or is a thread the
    /// editor started for work in the background.
    ///
    /// What lets a primitive that makes sense only on the thread the user
    /// types on refuse to run anywhere else: a prompt opened by a job would be
    /// a window nobody types into, and the key that answered it would go to
    /// whatever the user was doing.
    static IN_WORKER: Cell<bool> = const { Cell::new(false) };
}

/// Returns whether the caller runs in the background: in a step of a Lisp
/// job, or on a thread of its own that the editor started.
///
/// For the few primitives that mean nothing off the thread the user types on.
/// One that only reads the editor's state has no business asking: asking
/// would make a job's Lisp a second kind of Lisp.
pub(crate) fn in_worker() -> bool {
    IN_WORKER.get()
}

/// Sets [`in_worker`] for as long as it lives, so that a step that ends by
/// raising an error cannot leave it set.
///
/// A Lisp job holds one around each step -- the executor's thread is shared
/// with the Rust jobs, which run no Lisp -- and [`EditorState::spawn_work`]
/// around the whole life of each thread it starts. It puts back what it found
/// rather than clearing: a step driven by hand, as the tests drive one, may be
/// inside another holder.
pub(crate) struct WorkerTurn(bool);

impl WorkerTurn {
    /// Marks this thread as in the background, until the turn is dropped.
    pub(crate) fn begin() -> Self {
        WorkerTurn(IN_WORKER.replace(true))
    }
}

impl Drop for WorkerTurn {
    fn drop(&mut self) {
        IN_WORKER.set(self.0);
    }
}

/// Whether work in the background has changed the screen since the host last
/// looked, and how to tell the host. See [`EditorState::on_background_change`].
#[derive(Default)]
pub(crate) struct BackgroundChange {
    /// Raised by every step, lowered by every snapshot: what makes a burst of
    /// steps one message to the host.
    pending: AtomicBool,
    /// When the last snapshot was taken: what spaces the frames the work
    /// causes.
    snapshot_at: Mutex<Option<Instant>>,
    /// What the host asked to have called.
    wake: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl BackgroundChange {
    /// Records that a step has ended, and tells the host the first time since
    /// its last snapshot, and not again until it has taken another.
    pub(crate) fn raise(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            // Cloned out, so the host's code runs with no lock of ours held.
            let wake = self.wake.read().expect("background waker").clone();
            if let Some(wake) = wake {
                wake();
            }
        }
    }

    /// Records that a snapshot is about to be taken, which holds whatever the
    /// steps have changed so far. Lowered before the state is read, so that a
    /// step ending while it is read raises the flag again, and is drawn in the
    /// next frame.
    pub(crate) fn taken(&self) {
        self.pending.store(false, Ordering::Release);
        *self.snapshot_at.lock().expect("snapshot time") = Some(Instant::now());
    }

    /// Returns how long until the screen should be drawn again for what the
    /// steps changed, or `None` when they have changed nothing since the last
    /// snapshot.
    ///
    /// Not at once: a job colouring a large file takes a step every millisecond
    /// or so, and a frame for each would repaint the terminal without pause.
    /// One frame per [`TURN_INTERVAL`] at most, counted from the last
    /// snapshot, which reads as live and costs little.
    pub(crate) fn due_in(&self) -> Option<Duration> {
        if !self.pending.load(Ordering::Acquire) {
            return None;
        }
        let since = self
            .snapshot_at
            .lock()
            .expect("snapshot time")
            .map(|at| at.elapsed());
        Some(match since {
            Some(since) => TURN_INTERVAL.saturating_sub(since),
            None => Duration::ZERO,
        })
    }
}

impl<B: BufferTrait> EditorState<B> {
    /// Registers WAKE, called whenever work in the background may have changed
    /// what is on screen: a job's step has ended since the last snapshot.
    ///
    /// For a host whose renderer waits for input: without this, a file the
    /// watcher reloads, or a buffer a Lisp job fills, stays as it was on
    /// screen until a key is pressed. WAKE is called on whatever thread the
    /// step ran on, once per snapshot however many steps end, so it must only
    /// make the host come back: send it a message, unpark it. It must not
    /// draw, take a snapshot, or wait for anything.
    ///
    /// Once woken, the host asks [`Self::next_redraw_in`] as usual: the
    /// answer then includes the change, spaced so that a busy job does not
    /// make the screen redraw at every step. A second call replaces WAKE.
    pub fn on_background_change(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self
            .background_change
            .wake
            .write()
            .expect("background waker") = Some(Arc::new(wake));
    }

    /// Stops the editor's asynchronous work, giving it WITHIN to finish.
    ///
    /// Every job leaves with `Exit::Shutdown`, and every dedicated thread is
    /// asked to stop; what has not finished by then is left behind, named in
    /// the log. Last, whatever the log holds that is not in its file yet is
    /// written there: the watchdog that writes it every turn has stopped.
    ///
    /// For the host to call once, as the editor exits -- also when it exits
    /// by a panic, or the log loses its last lines, which are the ones that
    /// say why. Nothing calls it on its own: the executor's thread holds a
    /// copy of the editor, which holds the executor, and `Drop` cannot break
    /// that cycle.
    pub fn shutdown(&self, within: Duration) {
        self.work.shutdown(self, within);
    }

    /// Returns the WorkManager: for the primitives that start, stop and list
    /// work, and for the tests that drive it by hand.
    ///
    /// Handed out whole rather than forwarded a method at a time, unlike the
    /// compartments: it has no lock a caller could take out of order or hold
    /// across the interpreter. Every call answers at once.
    pub(crate) fn work(&self) -> &WorkManager<Self> {
        &self.work
    }

    /// Starts BODY on a thread of its own called NAME: work that may block,
    /// which no step may do -- reading a process's output, walking a tree.
    ///
    /// BODY is handed the editor, the [`Stop`] that `stop-worker` raises and
    /// its own number. The thread is off the command thread for its whole
    /// life, so a prompt cannot be put up from it, and when BODY returns the
    /// host is told to look: whatever it changed last is drawn. `None` when
    /// the thread could not be started -- the editor is shutting down, or
    /// the system will not give one.
    pub(crate) fn spawn_work(
        &self,
        name: &str,
        body: impl FnOnce(&EditorState<B>, &Stop, WorkId) + Send + 'static,
    ) -> Option<WorkId> {
        let state = self.clone();
        self.work.spawn(name, move |stop, id| {
            let _off_the_command_thread = WorkerTurn::begin();
            body(&state, &stop, id);
            state.changed_in_background();
        })
    }

    /// Starts BODY on a thread of its own that writes into BUFFER, and is known
    /// by its name: a shell command, a search. `stop-worker BUFFER` stops it,
    /// and so does closing the buffer.
    ///
    /// The buffer is told who writes into it before BODY can write a line:
    /// [`Self::append_as_writer`] refuses every other writer, and a line
    /// written in the moment before the buffer knew would be lost. When BODY
    /// returns, the buffer has no writer again. A buffer gone before it could
    /// be told -- closed from another thread in that moment -- stops the work
    /// at once, which BODY then finds out as it would from `stop-worker`.
    pub(crate) fn spawn_writer(
        &self,
        buffer: &str,
        body: impl FnOnce(&EditorState<B>, &Stop, WorkId) + Send + 'static,
    ) -> Option<WorkId> {
        let (go, gate) = std::sync::mpsc::sync_channel::<()>(1);
        let name = buffer.to_string();
        let id = self.spawn_work(buffer, move |state, stop, id| {
            // Whatever the answer: a gate that closed without one is a
            // spawner that went away, and the stop is what says so.
            let _ = gate.recv();
            body(state, stop, id);
            state.release_writer(&name, id);
        })?;
        let told = self
            .with_buffer_mut(buffer, |buf| buf.writer = Some(id))
            .is_some();
        if !told {
            self.work.cancel(id);
        }
        let _ = go.send(());
        Some(id)
    }

    /// Tells the host that a thread of the editor's own has changed the screen
    /// -- output appended, a result found -- as the end of a step does: see
    /// [`Self::on_background_change`]. Cheap enough for every line, since only
    /// the first since the last snapshot does more than set a flag.
    pub(crate) fn changed_in_background(&self) {
        self.background_change.raise();
    }

    /// Returns STEM, or `STEM<2>`, `STEM<3>`, the first that no buffer and no
    /// work has: the name of a buffer that work started now will write into,
    /// and be known by.
    ///
    /// Free in both, since the two share it: a name only the buffers had given
    /// up -- a command's buffer renamed while it runs -- would stop that
    /// command as soon as a new one took the name.
    pub(crate) fn free_name_for_work(&self, stem: &str) -> String {
        let taken = |name: &str| self.has_buffer(name) || self.work.is_named(name);
        if !taken(stem) {
            return stem.to_string();
        }
        (2..)
            .map(|n| format!("{stem}<{n}>"))
            .find(|candidate| !taken(candidate))
            .expect("the names run out only with the integers")
    }
}
