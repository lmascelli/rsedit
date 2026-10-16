//! The facade over the WorkManager: what the host, the primitives and the
//! tests ask of the editor's asynchronous work, the topics the editor emits
//! for it, and how the host hears that the work changed the screen.
//!
//! # The topics
//!
//! A topic carries no data: it says "go and look", and what changed is read
//! from the state, where it happened. So each is emitted where the change is
//! made, in the one place every change of its kind goes through -- a change
//! that does not pass there is one no job can be woken for.
//!
//! | Topic | Emitted by |
//! |---|---|
//! | [`BUFFER_CHANGED`] | `edits::changed` (both doors, undo, redo, `replace-back`), `Buffer::adopt_text`, a change of major mode, a buffer joining the table |
//! | [`SYNTAX_CHANGED`] | `set_mode`, `edit_grammar`, `edit_syntax_table` |
//!
//! Lisp emits topics of its own with `emit-event`.
//!
//! # Telling the host
//!
//! Almost everything on screen changes because the user did something, and a
//! host that draws after every event and then waits for the next one is
//! right about all of it -- except what a job changes. A file reloaded by the
//! watcher, a variable a Lisp job set, a buffer it filled: none of them comes
//! with an event, and a host blocked on input sleeps straight through them.
//!
//! So the WorkManager calls the editor after every step, and the editor tells
//! the host -- once, however many steps follow, until the host takes a
//! snapshot. What the step changed is not said, because it is not known: a
//! step is free to change anything, so the host looks at the whole frame.
//! See [`EditorState::on_background_change`].
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
    /// Whether this thread is part-way through a step of a Lisp job, or is a
    /// thread the editor started for work in the background.
    ///
    /// # What it is for
    ///
    /// Some things an editor can do only make sense on the thread the user is
    /// typing on. Opening a prompt is the clearest: a minibuffer put up by a
    /// job is a window nobody is typing into, over a command nobody
    /// started, and the keystroke that answers it goes to whatever the user
    /// was actually doing.
    ///
    /// A job *can* call `minibuffer-read` -- nothing in Rust stops a Lisp
    /// program from naming a function -- so the refusal has to be somewhere,
    /// and it has to be somewhere that knows which thread it is on. This is
    /// that.
    static IN_WORKER: Cell<bool> = const { Cell::new(false) };
}

/// Whether the caller is running in the background: inside a step of a Lisp
/// job, or on a thread of its own the editor started.
///
/// For the handful of primitives that are meaningless off the thread the user
/// is typing on. A primitive that merely *reads* editor state has no business
/// asking: it is as correct here as anywhere, and asking would make jobs a
/// second class of Lisp rather than the same Lisp somewhere else.
pub(crate) fn in_worker() -> bool {
    IN_WORKER.get()
}

/// Sets [`in_worker`] for as long as it is alive.
///
/// RAII rather than a pair of calls so that a step which ends by propagating
/// an error cannot leave the flag set -- which would make every later
/// primitive on this thread believe it was in the background.
///
/// # Who holds one
///
/// A Lisp job, around each of its steps: the executor's thread is shared with
/// the Rust jobs, which run no Lisp and so have nothing to refuse. And the
/// old scheduler, around every thread it starts for a shell command or a
/// search.
///
/// # Why it nests
///
/// A step driven by hand -- which is how the tests drive one -- may be inside
/// another holder. So this restores what it found rather than clearing: an
/// inner step ending inside an outer one must not announce that the thread
/// has become the command thread.
pub(crate) struct WorkerTurn(bool);

impl WorkerTurn {
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
    /// A step has ended. The host is told the first time since its last
    /// snapshot, and not again until it has taken another.
    pub(crate) fn raise(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            // Cloned out, so the host's code runs with no lock of ours held.
            let wake = self.wake.read().expect("background waker").clone();
            if let Some(wake) = wake {
                wake();
            }
        }
    }

    /// A snapshot is about to be taken: whatever the steps changed so far is
    /// in it. Lowered *before* the state is read, so that a step ending while
    /// it is being read raises the flag again and is drawn in the next frame.
    pub(crate) fn taken(&self) {
        self.pending.store(false, Ordering::Release);
        *self.snapshot_at.lock().expect("snapshot time") = Some(Instant::now());
    }

    /// How long until the screen should be drawn again for what the steps
    /// changed, or `None` when they have changed nothing since the last
    /// snapshot.
    ///
    /// Not at once: a job colouring a large file takes a step every
    /// millisecond or so, and a frame for each would be a terminal repainted
    /// continuously. So at most one frame per [`TURN_INTERVAL`], counted from
    /// the last snapshot, which reads as live and costs little -- the same
    /// rate colour has always been drawn at.
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
    /// Have WAKE called whenever work in the background may have changed
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

    /// Stop the editor's asynchronous work, giving it WITHIN to finish.
    ///
    /// Every job leaves with `Exit::Shutdown`, and every dedicated thread is
    /// asked to stop; what has not finished by then is left behind, named in
    /// the log. For the host to call once, as the editor exits. Nothing calls
    /// it on its own: the executor's thread holds a copy of the editor, which
    /// holds the executor, and `Drop` cannot break that cycle.
    pub fn shutdown(&self, within: Duration) {
        self.work.shutdown(self, within);
    }

    /// The WorkManager: for the primitives that start, stop and list work,
    /// and for the tests that drive it by hand.
    ///
    /// Handed out whole rather than forwarded a method at a time, unlike the
    /// compartments: it has no lock a caller could take out of order or hold
    /// across the interpreter. Every call answers at once.
    pub(crate) fn work(&self) -> &WorkManager<Self> {
        &self.work
    }
}
