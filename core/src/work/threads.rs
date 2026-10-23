//! Dedicated threads: work that may block, on a thread of its own, which the
//! WorkManager starts, keeps the handle of and lists beside its jobs. A thread
//! cannot be killed, so stopping one is a request: the [`Stop`].
//!
//! Design: `doc/architettura-editor.typ, <thread-dedicati>`.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::job::WorkId;

/// What unblocks a dedicated thread's body.
pub(crate) type Action = Box<dyn FnOnce() + Send>;

/// The request to stop, as a dedicated thread's body sees it: a signal the
/// body checks where it is safe to leave, and the action registered with
/// [`Stop::on_cancel`], which unblocks it where it waits and cannot check.
/// Cheap to clone: every clone is the same signal.
#[derive(Clone, Default)]
pub struct Stop(Arc<Signal>);

#[derive(Default)]
struct Signal {
    raised: AtomicBool,
    /// Behind the lock that raising the signal takes too. So an action
    /// registered just as the request arrives runs exactly once: handed to
    /// whoever raised the signal, or run at once by `on_cancel`.
    action: Mutex<Option<Action>>,
}

impl Stop {
    /// Returns whether the thread has been asked to stop: for the body to
    /// check where it is safe to leave, between two lines of output, between
    /// two files.
    pub fn is_cancelled(&self) -> bool {
        self.0.raised.load(Ordering::Acquire)
    }

    /// Has ACTION run when the thread is asked to stop: what unblocks the body
    /// where it cannot check, such as killing the process it reads from.
    ///
    /// It replaces the action registered before, if any, so a body that moves
    /// from one blocking thing to the next registers each in turn. It runs on
    /// the thread that asks for the stop -- a command, or `shutdown` -- so it
    /// must be quick: kill, close, drop. When the request has already come,
    /// ACTION runs at once, here.
    pub fn on_cancel(&self, action: impl FnOnce() + Send + 'static) {
        let mut slot = self.0.action.lock().expect("stop action");
        if self.is_cancelled() {
            drop(slot);
            action();
        } else {
            *slot = Some(Box::new(action));
        }
    }

    /// Raises the signal, and hands back the action to run. Only the first
    /// call finds one.
    pub(crate) fn raise(&self) -> Option<Action> {
        let mut slot = self.0.action.lock().expect("stop action");
        self.0.raised.store(true, Ordering::Release);
        slot.take()
    }
}

/// A dedicated thread, as the registry keeps it.
pub(crate) struct Dedicated {
    pub(crate) name: String,
    pub(crate) stop: Stop,
    pub(crate) handle: JoinHandle<()>,
    pub(crate) started: Instant,
    /// When it was asked to stop.
    pub(crate) asked: Option<Instant>,
    /// Whether it has been reported for not stopping.
    pub(crate) reported: bool,
}

/// A dedicated thread as `work-list` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadInfo {
    pub id: WorkId,
    pub name: String,
    /// Asked to stop, and not stopped yet.
    pub stopping: bool,
    pub running_for: Duration,
}

/// How many of a WorkManager's threads are alive: what `shutdown` waits on.
///
/// Each thread carries a [`Life`] from before it starts to its very end, so
/// the count reaches zero only when no thread the WorkManager started is
/// still running code of its own.
#[derive(Default)]
pub(crate) struct Lives {
    count: Mutex<usize>,
    changed: Condvar,
}

/// One thread's share of [`Lives`], given back when the thread ends, whether
/// it returns or unwinds.
pub(crate) struct Life(Arc<Lives>);

impl Lives {
    /// Returns a share for a thread about to start.
    pub(crate) fn enter(self: &Arc<Self>) -> Life {
        *self.count() += 1;
        Life(self.clone())
    }

    /// Waits until no thread is alive, or for WITHIN at most. True if none
    /// is. A span rather than a deadline, so that any span will do: an
    /// `Instant` that far in the future would overflow.
    pub(crate) fn wait(&self, within: Duration) -> bool {
        let started = Instant::now();
        let mut count = self.count();
        while *count > 0 {
            let left = within.saturating_sub(started.elapsed());
            if left.is_zero() {
                return false;
            }
            count = self
                .changed
                .wait_timeout(count, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Returns the count, locked. The lock is taken while a panicking thread
    /// unwinds, where a second panic would abort the process: so a poisoned
    /// lock is used as it is.
    /// What it guards is a number, changed by one at a time, which a panic
    /// cannot leave half-written.
    fn count(&self) -> std::sync::MutexGuard<'_, usize> {
        self.count.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Life {
    fn drop(&mut self) {
        *self.0.count() -= 1;
        self.0.changed.notify_all();
    }
}

/// Starts F on a thread called NAME, carrying LIFE until its very end.
///
/// The name is what a debugger and `top -H` show, cut to 15 bytes by Linux.
pub(crate) fn start_thread(
    name: &str,
    life: Life,
    f: impl FnOnce() + Send + 'static,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        // A NUL in a thread's name makes the standard library panic, and a
        // job's name may come from Lisp.
        .name(name.replace('\0', " "))
        .spawn(move || {
            f();
            // After everything F owned has gone with it.
            drop(life);
        })
}
