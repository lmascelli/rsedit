//! The executor: the core and the far end of its inbox, and the loop that
//! drives them. On a thread of its own in the editor; driven by the test in
//! manual mode.
//!
//! Design: `doc/architettura-editor.typ, <esecutore>`.
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::core::Core;
use super::job::{Job, Spec, WorkId};
use super::manager::Shared;

/// How often a busy executor brings the registry's measurements up to date.
/// An idle one does it before it sleeps.
pub(super) const PUBLISH_EVERY: Duration = Duration::from_millis(100);

/// What the WorkManager tells its executor.
///
/// All but [`Msg::Nudge`] are sent with the registry's lock held. So the
/// inbox holds them in the order the registry changed, and once
/// [`Msg::Shutdown`] is in, nothing else will be.
pub(super) enum Msg<C: 'static> {
    Submit(WorkId, Spec, Box<dyn Job<C>>),
    Cancel(WorkId),
    /// A topic was emitted: look again. Sent from wherever the emission
    /// happened, and never more than one waiting in the inbox.
    Nudge,
    Shutdown,
}

/// The core and the far end of its inbox: what an executor runs.
pub(super) struct Executor<C: 'static> {
    pub(super) core: Core<C>,
    pub(super) inbox: Receiver<Msg<C>>,
    /// [`Msg::Shutdown`] has been read: no step runs after it.
    pub(super) shut: bool,
}

impl<C: 'static> Executor<C> {
    /// Applies every message waiting.
    fn drain(&mut self) {
        while let Ok(msg) = self.inbox.try_recv() {
            self.apply(msg);
        }
    }

    /// Applies MSG to the core.
    fn apply(&mut self, msg: Msg<C>) {
        match msg {
            // Refused only once the core has shut down, which happens after
            // the last message has been read: see `Msg`.
            Msg::Submit(id, spec, job) => {
                self.core.submit_as(id, spec, job);
            }
            Msg::Cancel(id) => {
                self.core.cancel(id);
            }
            Msg::Nudge => {}
            Msg::Shutdown => self.shut = true,
        }
    }

    /// Runs one turn of the executor's loop: the messages, then at most one
    /// step. True when a step ran.
    pub(super) fn turn(&mut self, shared: &Shared<C>, state: &C) -> bool {
        self.drain();
        if self.shut {
            return false;
        }
        // Cleared before the core looks at the topics, and with a swap, which
        // the emission's own swap is ordered against: an emission this look
        // will miss finds the flag clear and sends a nudge, and one it finds
        // set happened before the look and is seen by it. So none can fall
        // between the look and the sleep that may follow it.
        shared.nudged.swap(false, Ordering::AcqRel);
        let stepped = self.core.run_once(state);
        self.settle(shared);
        // After the registry has heard who left, so that whoever the hook
        // wakes finds a job that has finished listed as gone.
        if stepped {
            shared.stepped();
        }
        stepped
    }

    /// Passes on what the turn left: report lines to the log, and the jobs
    /// that left to the registry.
    fn settle(&mut self, shared: &Shared<C>) {
        for line in self.core.take_reports() {
            (shared.log)(&line);
        }
        let departed = self.core.take_departed();
        if !departed.is_empty() {
            let mut registry = shared.registry.lock().expect("work registry");
            for id in departed {
                if let Some(job) = registry.jobs.remove(&id) {
                    registry.forget_name(&job.name, id);
                }
            }
        }
    }

    /// Brings the registry's measurements up to date. A job the registry no
    /// longer lists -- cancelled, replaced -- is not put back.
    pub(super) fn publish(&self, shared: &Shared<C>) {
        let infos = self.core.info();
        let mut registry = shared.registry.lock().expect("work registry");
        for info in infos {
            if let Some(entry) = registry.jobs.get_mut(&info.id) {
                *entry = info;
            }
        }
    }

    /// Closes the executor: what is still in the inbox is applied, then every
    /// job leaves, with [`Exit::Shutdown`](super::job::Exit::Shutdown).
    pub(super) fn close(&mut self, shared: &Shared<C>, state: &C) {
        self.drain();
        self.core.shutdown(state);
        self.settle(shared);
    }
}

/// Runs the executor's thread, from `start` to `shutdown`: turns while a job
/// is ready, and a sleep until the next deadline or message when none is.
pub(super) fn executor_main<C: 'static>(
    shared: Arc<Shared<C>>,
    mut executor: Executor<C>,
    state: C,
) {
    let mut published = Instant::now();
    loop {
        if executor.turn(&shared, &state) {
            if published.elapsed() >= PUBLISH_EVERY {
                executor.publish(&shared);
                published = Instant::now();
            }
            continue;
        }
        if executor.shut {
            break;
        }
        executor.publish(&shared);
        published = Instant::now();
        // Until the next deadline, or the next message: whichever is first.
        let message = match executor.core.next_wakeup() {
            Some(at) => executor
                .inbox
                .recv_timeout(at.saturating_duration_since(Instant::now())),
            None => executor
                .inbox
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
        };
        match message {
            Ok(message) => executor.apply(message),
            Err(RecvTimeoutError::Timeout) => {}
            // Not while the topics hold a sender to the inbox -- and if it
            // did happen, nothing could reach this executor again.
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    executor.close(&shared, &state);
}
