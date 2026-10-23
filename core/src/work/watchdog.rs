//! The watchdog: a thread of its own that looks at what the executor cannot
//! report while it is in it, the step in progress. It also joins the dedicated
//! threads that have finished, and calls the host's after-watch hook.
//!
//! Design: `doc/architettura-editor.typ, <watchdog>`.
use std::sync::{Arc, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::core::{Current, panic_message};
use super::job::WorkId;
use super::manager::{Shared, WorkManager};
use super::threads::{Dedicated, start_thread};

/// What the watchdog has said about the step in progress, so that it says it
/// again only once the step has run twice as long.
#[derive(Default)]
pub(super) struct Watching {
    /// Which step it was: [`Current::serial`].
    serial: Option<u64>,
    /// How long that step has to run before it is reported again.
    next: Duration,
}

impl<C: Send + 'static> WorkManager<C> {
    /// Starts the watchdog, if it is not running: with the executor, or with
    /// the first dedicated thread if that comes first. In manual mode there is
    /// none: the threads are reaped whenever the test drives the WorkManager,
    /// and the test takes the watchdog's turns itself.
    ///
    /// Called with the registry's lock held, so a failure comes back as the
    /// line to log once the lock is given back.
    pub(super) fn watch(&self) -> Option<String> {
        if self.shared.manual.is_some() {
            return None;
        }
        let mut watchdog = self.shared.watchdog.lock().expect("watchdog");
        if watchdog.is_some() {
            return None;
        }
        let shared = self.shared.clone();
        let started = start_thread(
            &self.thread_name("watchdog"),
            self.shared.lives.enter(),
            move || watchdog_main(shared),
        );
        match started {
            Ok(handle) => {
                *watchdog = Some(handle);
                None
            }
            Err(err) => Some(format!("[BKG] error: the watchdog did not start: {err}")),
        }
    }
}

impl<C: 'static> Shared<C> {
    /// Runs one turn of the watchdog: joins the threads that have finished,
    /// reports a step that has run too long, and hands over to the host's hook
    /// last, so that what the turn reported is in what the hook writes.
    pub(super) fn watch_turn(&self) {
        self.reap();
        self.watch_step();
        self.watched();
    }

    /// Reports the step in progress once it has run `stuck-step`, and again
    /// each time it has run twice as long as when it was last reported: 1 s,
    /// 2 s, 4 s, with the default. A turn that comes late reports once, for
    /// how long the step has run, and the next mark is past that.
    fn watch_step(&self) {
        // Never poisoned: what runs under it cannot panic. But this is the
        // watchdog, which must not be the one to find out otherwise.
        let mut watching = self.watching.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(current) = self.underway.current() else {
            *watching = Watching::default();
            return;
        };
        if watching.serial != Some(current.serial) {
            *watching = Watching {
                serial: Some(current.serial),
                next: self.options.stuck_step(),
            };
        }
        let running = self.clock.now().saturating_duration_since(current.started);
        if running < watching.next {
            return;
        }
        (self.log)(&format!(
            "[BKG] stuck step: {}",
            running_for(&current, running)
        ));
        let mut next = watching.next.saturating_mul(2);
        while next <= running {
            next = next.saturating_mul(2);
        }
        watching.next = next;
    }

    /// Joins the dedicated threads that have finished, and reports those
    /// still running `stuck-step` after they were asked to stop.
    pub(super) fn reap(&self) {
        let now = Instant::now();
        let stuck = self.options.stuck_step();
        let mut lines = Vec::new();
        let finished: Vec<Dedicated> = {
            let mut registry = self.registry.lock().expect("work registry");
            let done: Vec<WorkId> = registry
                .threads
                .iter()
                .filter(|(_, thread)| thread.handle.is_finished())
                .map(|(id, _)| *id)
                .collect();
            let finished = done
                .into_iter()
                .filter_map(|id| {
                    let thread = registry.threads.remove(&id)?;
                    registry.forget_name(&thread.name, id);
                    Some(thread)
                })
                .collect();
            for thread in registry.threads.values_mut() {
                if let Some(asked) = thread.asked
                    && !thread.reported
                    && now.saturating_duration_since(asked) >= stuck
                {
                    thread.reported = true;
                    lines.push(format!(
                        "[BKG] thread: {} still running {} ms after it was asked to stop",
                        thread.name,
                        now.saturating_duration_since(asked).as_millis()
                    ));
                }
            }
            finished
        };
        for thread in finished {
            self.join(thread.name, thread.handle);
        }
        for line in lines {
            (self.log)(&line);
        }
    }

    /// Joins HANDLE, a thread that has finished: a panic in it goes to the
    /// log.
    pub(super) fn join(&self, name: String, handle: JoinHandle<()>) {
        if let Err(payload) = handle.join() {
            (self.log)(&format!(
                "[BKG] panic: thread {name}: {}",
                panic_message(&*payload)
            ));
        }
    }
}

/// Runs the watchdog's thread: a turn every `watchdog-every`, until
/// `shutdown`. Once it has stopped, `shutdown` calls the after-watch hook one
/// last time itself, after the lines it writes.
fn watchdog_main<C: 'static>(shared: Arc<Shared<C>>) {
    loop {
        std::thread::park_timeout(shared.options.watchdog_every());
        if shared.registry.lock().expect("work registry").closed {
            break;
        }
        shared.watch_turn();
    }
}

/// Returns CURRENT, running for RUNNING, as a report says it:
/// `highlighter running for 2.0 s`, and `... in its exit`.
pub(super) fn running_for(current: &Current, running: Duration) -> String {
    let exit = if current.exit { " in its exit" } else { "" };
    format!(
        "{} running for {:.1} s{exit}",
        current.name,
        running.as_secs_f64()
    )
}
