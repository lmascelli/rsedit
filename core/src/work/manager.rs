//! The WorkManager: one per editor, shared by every clone of it. It owns the
//! scheduler's core and the executor that drives it, the registry of what is
//! running, the options, the topics, the dedicated threads and the watchdog.
//!
//! Design: `doc/architettura-editor.typ, <work-manager>`.
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::clock::{Clock, ManualClock, SystemClock};
use super::core::{Core, Place, Underway, WorkInfo, panic_message};
use super::executor::{Executor, Msg, executor_main};
use super::job::{Job, Spec, Start, WorkId};
use super::options::Options;
use super::registry::{Registry, Withdrawn};
use super::threads::{Dedicated, Lives, Stop, ThreadInfo, start_thread};
use super::topic::{Topic, Topics};
use super::watchdog::{Watching, running_for};

/// The most steps one drive of the manual mode runs: a job that always answers
/// `Continue` must not hang a test.
const MANUAL_LIMIT: usize = 100_000;

/// The asynchronous work of one editor. Cheap to clone: every clone is the
/// same WorkManager.
///
/// A call never touches the core: it changes the registry, which answers at
/// once, and leaves a message in the executor's inbox, read between two steps.
/// So `submit` from inside a step is as safe as from anywhere.
pub struct WorkManager<C: 'static> {
    pub(super) shared: Arc<Shared<C>>,
}

impl<C: 'static> Clone for WorkManager<C> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

/// What every clone of a WorkManager shares.
pub(super) struct Shared<C: 'static> {
    /// The number in this WorkManager's thread names: `w{instance} executor`.
    pub(super) instance: u64,
    pub(super) options: Arc<Options>,
    pub(super) topics: Arc<Topics>,
    /// Where the report lines go: in the editor, its log.
    pub(super) log: Box<dyn Fn(&str) + Send + Sync>,
    /// Called after every step: see [`WorkManager::after_step`].
    pub(super) after_step: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// Called after every turn of the watchdog: see
    /// [`WorkManager::after_watch`].
    pub(super) after_watch: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// Set while a nudge is in the inbox, unread: what makes a burst of
    /// emissions one wake-up.
    pub(super) nudged: Arc<AtomicBool>,
    pub(super) inbox: Sender<Msg<C>>,
    pub(super) registry: Mutex<Registry>,
    /// The executor, until a thread takes it -- never, in manual mode.
    pub(super) executor: Mutex<Option<Executor<C>>>,
    /// The clock the core reads, which the watchdog times its steps by.
    pub(super) clock: Arc<dyn Clock>,
    /// The same clock, in manual mode, where the test moves it.
    pub(super) manual: Option<Arc<ManualClock>>,
    /// What the executor is running, as it says while it runs.
    pub(super) underway: Arc<Underway>,
    /// What the watchdog has said about it.
    pub(super) watching: Mutex<Watching>,
    pub(super) lives: Arc<Lives>,
    pub(super) executor_thread: Mutex<Option<JoinHandle<()>>>,
    pub(super) watchdog: Mutex<Option<JoinHandle<()>>>,
}

impl<C: Send + 'static> WorkManager<C> {
    /// Returns a WorkManager for the editor: an executor thread from
    /// [`Self::start`] on, and the real clock. Report lines go to LOG.
    pub fn threaded(log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self::build(Arc::new(SystemClock), None, Box::new(log))
    }

    /// Returns a WorkManager for tests: nothing runs until the test drives it,
    /// on a clock that moves when it is told to. Report lines go to LOG.
    pub fn manual(log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        let clock = ManualClock::new();
        Self::build(clock.clone(), Some(clock), Box::new(log))
    }

    /// Builds a WorkManager on CLOCK, which is MANUAL when the tests move it.
    fn build(
        clock: Arc<dyn Clock>,
        manual: Option<Arc<ManualClock>>,
        log: Box<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        static INSTANCES: AtomicU64 = AtomicU64::new(1);
        let (inbox, receiver) = mpsc::channel();
        let nudged = Arc::new(AtomicBool::new(false));
        let topics = {
            let (nudged, inbox) = (nudged.clone(), inbox.clone());
            Arc::new(Topics::with_nudge(move || {
                if !nudged.swap(true, Ordering::AcqRel) {
                    let _ = inbox.send(Msg::Nudge);
                }
            }))
        };
        let options = Arc::new(Options::default());
        let core = Core::new(options.clone(), topics.clone(), clock.clone());
        let underway = core.underway();
        Self {
            shared: Arc::new(Shared {
                instance: INSTANCES.fetch_add(1, Ordering::Relaxed),
                options,
                topics,
                log,
                after_step: OnceLock::new(),
                after_watch: OnceLock::new(),
                nudged,
                inbox,
                registry: Mutex::default(),
                executor: Mutex::new(Some(Executor {
                    core,
                    inbox: receiver,
                    shut: false,
                })),
                clock,
                manual,
                underway,
                watching: Mutex::default(),
                lives: Arc::default(),
                executor_thread: Mutex::new(None),
                watchdog: Mutex::new(None),
            }),
        }
    }

    /// Starts the executor's thread, on STATE, and the watchdog's.
    ///
    /// Called once the state exists, which in the editor is after the
    /// WorkManager, since the state holds it: work submitted before waits in
    /// the inbox. In manual mode there is no thread to start, and STATE is not
    /// kept: the test hands one to every call that runs work, and runs the
    /// watchdog's turns with [`Self::watch_once`].
    pub fn start(&self, state: C) {
        if self.shared.manual.is_some() {
            return;
        }
        let unwatched = {
            let registry = self.shared.registry.lock().expect("work registry");
            if registry.closed {
                return;
            }
            self.watch()
        };
        if let Some(line) = unwatched {
            (self.shared.log)(&line);
        }
        let Some(executor) = self.shared.executor.lock().expect("executor").take() else {
            return;
        };
        let shared = self.shared.clone();
        let started = start_thread(
            &self.thread_name("executor"),
            self.shared.lives.enter(),
            move || executor_main(shared, executor, state),
        );
        match started {
            Ok(handle) => {
                *self.shared.executor_thread.lock().expect("executor thread") = Some(handle);
            }
            // The inbox went with the thread that never ran, so from here on
            // every `submit` is refused.
            Err(err) => {
                (self.shared.log)(&format!("[BKG] error: the executor did not start: {err}"))
            }
        }
    }

    /// Hands JOB to the scheduler under the name in SPEC, and returns its
    /// number at once: the job joins its queue at the executor's next turn.
    ///
    /// Work that already has the name -- a job or a thread -- is stopped.
    /// `None` after [`Self::shutdown`], or when the executor never started,
    /// and then JOB is not kept: its `exit` is not called, and the caller
    /// knows it was refused.
    pub fn submit(&self, spec: Spec, job: Box<dyn Job<C>>) -> Option<WorkId> {
        let shared = &self.shared;
        let id = WorkId::next();
        let (taken, replaced) = {
            let mut registry = shared.registry.lock().expect("work registry");
            if registry.closed {
                return None;
            }
            let replaced = registry.withdraw_named(&spec.name, &shared.inbox);
            let info = WorkInfo {
                id,
                name: spec.name.clone(),
                class: spec.class,
                place: match spec.start {
                    Start::Now => Place::Ready,
                    Start::After(_) => Place::Sleeping,
                },
                steps: 0,
                total: Duration::ZERO,
                longest: Duration::ZERO,
            };
            // Refused only when the executor has gone without a shutdown: it
            // did not start, and nothing will ever read the inbox.
            let taken = shared.inbox.send(Msg::Submit(id, spec, job)).is_ok();
            if taken {
                registry.names.insert(info.name.clone(), id);
                registry.latest.insert(info.name.clone(), id);
                registry.jobs.insert(id, info);
            }
            (taken, replaced)
        };
        shared.finish(replaced);
        taken.then_some(id)
    }

    /// Starts BODY on a dedicated thread called NAME, and returns its number:
    /// work that may block, which no step may do.
    ///
    /// BODY is handed the [`Stop`] that [`Self::cancel`] raises, and its own
    /// number, as a job's step is. Work that already has the name is stopped,
    /// as with [`Self::submit`]. `None` after [`Self::shutdown`], or when the
    /// system will not give a thread.
    pub fn spawn(
        &self,
        name: &str,
        body: impl FnOnce(Stop, WorkId) + Send + 'static,
    ) -> Option<WorkId> {
        let shared = &self.shared;
        let id = WorkId::next();
        let stop = Stop::default();
        let (started, replaced) = {
            let mut registry = shared.registry.lock().expect("work registry");
            if registry.closed {
                return None;
            }
            let replaced = registry.withdraw_named(name, &shared.inbox);
            let handed = stop.clone();
            let started = start_thread(&self.thread_name(name), shared.lives.enter(), move || {
                body(handed, id)
            });
            let started = started.map(|handle| {
                registry.names.insert(name.to_string(), id);
                registry.latest.insert(name.to_string(), id);
                registry.threads.insert(
                    id,
                    Dedicated {
                        name: name.to_string(),
                        stop,
                        handle,
                        started: Instant::now(),
                        asked: None,
                        reported: false,
                    },
                );
                // Under the registry's lock, like the thread it is for: so
                // either `shutdown` has not closed the registry yet and will
                // find the watchdog, or this thread was never started.
                self.watch()
            });
            (started, replaced)
        };
        shared.finish(replaced);
        match started {
            Ok(unwatched) => {
                if let Some(line) = unwatched {
                    (shared.log)(&line);
                }
                Some(id)
            }
            Err(err) => {
                (shared.log)(&format!("[BKG] error: thread {name} did not start: {err}"));
                None
            }
        }
    }

    /// Stops the work numbered ID.
    ///
    /// A job leaves at the end of its step, or before its next one, with
    /// [`Exit::Cancelled`](super::job::Exit::Cancelled). A thread has its
    /// [`Stop`] raised and its action run, here, and is listed as stopping
    /// until it has. False when there is no such work: it has finished, or
    /// never was.
    pub fn cancel(&self, id: WorkId) -> bool {
        let shared = &self.shared;
        // A thread that has finished is not there to stop, though it stays
        // listed until it is joined: so it is joined first.
        shared.reap();
        let withdrawn = shared
            .registry
            .lock()
            .expect("work registry")
            .withdraw(id, &shared.inbox);
        shared.finish(withdrawn)
    }

    /// Stops the work called NAME, as [`Self::cancel`] does.
    pub fn cancel_named(&self, name: &str) -> bool {
        let shared = &self.shared;
        shared.reap();
        let withdrawn = shared
            .registry
            .lock()
            .expect("work registry")
            .withdraw_named(name, &shared.inbox);
        shared.finish(withdrawn)
    }

    /// Has HOOK called after every step, on the thread that took it: how the
    /// host hears that a job may have changed something it shows, without
    /// asking. HOOK runs between two steps, so it must be quick and must not
    /// wait.
    ///
    /// Set once, and before the work it is for starts; a second call is
    /// ignored and answers false.
    pub fn after_step(&self, hook: impl Fn() + Send + Sync + 'static) -> bool {
        self.shared.after_step.set(Box::new(hook)).is_ok()
    }

    /// Has HOOK called at the end of every turn of the watchdog, on its
    /// thread, and once more at the end of [`Self::shutdown`], on the caller's,
    /// for the lines `shutdown` itself writes.
    ///
    /// On the watchdog's thread a write that blocks -- a full disk, a home
    /// directory on the network -- holds up nothing else, and what the
    /// watchdog reports reaches the log even when what it reports on is stuck:
    /// the editor writes its log file here. Set once, and before the work it
    /// is for starts; a second call is ignored and answers false.
    pub fn after_watch(&self, hook: impl Fn() + Send + Sync + 'static) -> bool {
        self.shared.after_watch.set(Box::new(hook)).is_ok()
    }

    /// Returns whether work -- a job or a thread -- is running under NAME.
    ///
    /// For whoever names work after something else that must be unique too:
    /// a shell command takes the name of the buffer it writes into, and that
    /// name has to be free here as well, or starting the command would stop
    /// whatever already had it.
    pub fn is_named(&self, name: &str) -> bool {
        let registry = self.shared.registry.lock().expect("work registry");
        registry.names.contains_key(name)
    }

    /// Returns whether ID is the newest work submitted under NAME and has not
    /// been stopped -- which stays true after it has finished.
    ///
    /// What the editor asks before it delivers a callback a job queued: the
    /// job may have finished since, and its ON-DONE is exactly the call that
    /// must still be made, but a newer job under the name, or a stop, silences
    /// it. The registry changes in one step with the work it describes, so two
    /// submissions under one name, or a submission and a stop, never leave it
    /// naming the wrong one.
    pub fn is_latest(&self, name: &str, id: WorkId) -> bool {
        let registry = self.shared.registry.lock().expect("work registry");
        registry.latest.get(name) == Some(&id)
    }

    /// Returns the topic called NAME, made on first use. See [`Topics::get`].
    pub fn topic(&self, name: &str) -> Topic {
        self.shared.topics.get(name)
    }

    /// Returns the scheduler's options, which take effect from the next step.
    pub fn options(&self) -> &Options {
        &self.shared.options
    }

    /// Returns every job, in the order they were submitted, with what the
    /// executor last published about them.
    ///
    /// It publishes when it runs out of work -- in manual mode, at the end of
    /// each drive -- and every
    /// [`PUBLISH_EVERY`](super::executor::PUBLISH_EVERY) while it is busy; a
    /// job it has not seen yet is listed as it was submitted. The job in the
    /// middle of a step is listed as running from what the executor says as it
    /// starts the step, rather than from what it last published: a step that
    /// does not end is one after which nothing is published again.
    pub fn jobs(&self) -> Vec<WorkInfo> {
        let stepping = self
            .shared
            .underway
            .current()
            .filter(|current| !current.exit)
            .map(|current| current.id);
        let registry = self.shared.registry.lock().expect("work registry");
        registry
            .jobs
            .values()
            .cloned()
            .map(|mut job| {
                if stepping == Some(job.id) {
                    job.place = Place::Running;
                }
                job
            })
            .collect()
    }

    /// Returns every dedicated thread still running, in the order they were
    /// started.
    pub fn threads(&self) -> Vec<ThreadInfo> {
        self.shared.reap();
        let now = Instant::now();
        let registry = self.shared.registry.lock().expect("work registry");
        registry
            .threads
            .iter()
            .map(|(id, thread)| ThreadInfo {
                id: *id,
                name: thread.name.clone(),
                stopping: thread.asked.is_some(),
                running_for: now.saturating_duration_since(thread.started),
            })
            .collect()
    }

    /// Stops everything, giving it WITHIN to finish.
    ///
    /// Nothing is taken from here on. Every job leaves with
    /// [`Exit::Shutdown`](super::job::Exit::Shutdown), and every dedicated
    /// thread is asked to stop. Whatever has not finished by the deadline -- a
    /// thread that does not stop, a step that does not end -- is left behind
    /// and named in the log. STATE is what the jobs' `exit` is handed when no
    /// executor thread is there to hand its own: in manual mode, or before
    /// [`Self::start`]. A second call does nothing.
    pub fn shutdown(&self, state: &C, within: Duration) {
        let started = Instant::now();
        let shared = &self.shared;
        let stops: Vec<(String, Stop)> = {
            let mut registry = shared.registry.lock().expect("work registry");
            if registry.closed {
                return;
            }
            registry.closed = true;
            // Under the lock: every job taken is in the inbox before this.
            let _ = shared.inbox.send(Msg::Shutdown);
            let asked = Instant::now();
            registry
                .threads
                .values_mut()
                .map(|thread| {
                    thread.asked.get_or_insert(asked);
                    (thread.name.clone(), thread.stop.clone())
                })
                .collect()
        };
        for (name, stop) in stops {
            shared.raise(&name, &stop);
        }
        // An executor that no thread took closes here, on this thread. One
        // with a thread of its own closes there, when it reads the message.
        let unstarted = shared.executor.lock().expect("executor").take();
        if let Some(mut executor) = unstarted {
            executor.close(shared, state);
        }
        if let Some(watchdog) = &*shared.watchdog.lock().expect("watchdog") {
            watchdog.thread().unpark();
        }
        let all_finished = shared.lives.wait(within.saturating_sub(started.elapsed()));
        shared.bury(all_finished);
        shared.watched();
    }

    /// Runs turns until no job is ready, in manual mode, applying what each one
    /// submits on the way, at most [`MANUAL_LIMIT`] steps. Returns the number
    /// of steps that ran.
    ///
    /// # Panics
    ///
    /// On a threaded WorkManager, whose executor runs by itself.
    pub fn run_until_idle(&self, state: &C) -> usize {
        self.manual_clock();
        let steps = {
            let mut slot = self.shared.executor.lock().expect("executor");
            let Some(executor) = slot.as_mut() else {
                return 0;
            };
            let mut steps = 0;
            while steps < MANUAL_LIMIT && executor.turn(&self.shared, state) {
                steps += 1;
            }
            executor.publish(&self.shared);
            steps
        };
        self.shared.reap();
        steps
    }

    /// Runs SPAN of scheduling in manual mode, the clock moved from one wake-up
    /// to the next instead of waited for. Returns the number of steps that
    /// ran. The clock ends SPAN later than it started, whatever ran.
    ///
    /// # Panics
    ///
    /// On a threaded WorkManager, whose executor runs by itself.
    pub fn run_for(&self, state: &C, span: Duration) -> usize {
        let clock = self.manual_clock();
        let end = clock.now() + span;
        let steps = {
            let mut slot = self.shared.executor.lock().expect("executor");
            let mut steps = 0;
            if let Some(executor) = slot.as_mut() {
                while steps < MANUAL_LIMIT {
                    if executor.turn(&self.shared, state) {
                        steps += 1;
                        continue;
                    }
                    match executor.core.next_wakeup() {
                        // A wake-up that is not in the future, with nothing
                        // ready, would be a turn that changes nothing.
                        Some(at) if at <= end && at > clock.now() => clock.advance_to(at),
                        _ => break,
                    }
                }
                executor.publish(&self.shared);
            }
            steps
        };
        clock.advance_to(end);
        self.shared.reap();
        steps
    }

    /// Moves the clock forward by BY in manual mode, running nothing.
    ///
    /// # Panics
    ///
    /// On a threaded WorkManager, which reads the real clock.
    pub fn advance(&self, by: Duration) {
        self.manual_clock().advance(by);
    }

    /// Runs one turn of the watchdog here and now, in manual mode: what its
    /// thread does every `watchdog-every`. The dedicated threads that have
    /// finished are joined, a step that has run too long is reported, and the
    /// hook set with [`Self::after_watch`] is called.
    ///
    /// The test's own thread is the executor in manual mode, so a test that
    /// wants a step to be watched takes it on another: the step runs there,
    /// the clock is moved and the watchdog looks from here.
    ///
    /// # Panics
    ///
    /// On a threaded WorkManager, whose watchdog runs by itself.
    pub fn watch_once(&self) {
        self.manual_clock();
        self.shared.watch_turn();
    }

    /// Returns the number in this WorkManager's thread names:
    /// `w{instance} ...`.
    pub(crate) fn instance(&self) -> u64 {
        self.shared.instance
    }

    /// Returns the clock the tests move, which only a manual WorkManager has.
    fn manual_clock(&self) -> &Arc<ManualClock> {
        self.shared
            .manual
            .as_ref()
            .expect("only a manual WorkManager is driven by hand: this one has an executor thread")
    }

    /// Returns the name of this WorkManager's thread for ROLE:
    /// `w{instance} {role}`.
    pub(super) fn thread_name(&self, role: &str) -> String {
        format!("w{} {role}", self.shared.instance)
    }
}

impl<C: 'static> Shared<C> {
    /// Does what WITHDRAWN left to do, with no lock held: raises a thread's
    /// stop. True when there was work to withdraw.
    fn finish(&self, withdrawn: Withdrawn) -> bool {
        match withdrawn {
            Withdrawn::Nothing => false,
            Withdrawn::Job => true,
            Withdrawn::Thread(name, stop) => {
                self.raise(&name, &stop);
                true
            }
        }
    }

    /// Raises STOP and runs the action it hands back: here, on the caller's
    /// thread, guarded like a step.
    fn raise(&self, name: &str, stop: &Stop) {
        if let Some(action) = stop.raise()
            && let Err(payload) = catch_unwind(AssertUnwindSafe(action))
        {
            (self.log)(&format!(
                "[BKG] panic: thread {name}: its stop action panicked: {}",
                panic_message(&*payload)
            ));
        }
    }

    /// Calls the hook set with [`WorkManager::after_step`], if there is one,
    /// guarded like a step: it is the host's code on the executor.
    pub(super) fn stepped(&self) {
        if let Some(hook) = self.after_step.get()
            && let Err(payload) = catch_unwind(AssertUnwindSafe(hook))
        {
            (self.log)(&format!(
                "[BKG] panic: the after-step hook: {}",
                panic_message(&*payload)
            ));
        }
    }

    /// Calls the hook set with [`WorkManager::after_watch`], if there is one,
    /// guarded like a step: it is the host's code on the watchdog.
    pub(super) fn watched(&self) {
        if let Some(hook) = self.after_watch.get()
            && let Err(payload) = catch_unwind(AssertUnwindSafe(hook))
        {
            (self.log)(&format!(
                "[BKG] panic: the after-watch hook: {}",
                panic_message(&*payload)
            ));
        }
    }

    /// Ends `shutdown`, after the wait: joins what has finished, and leaves
    /// behind, named in the log, what has not. ALL_FINISHED says that every
    /// thread is done -- possibly a moment before it can be joined, so it is
    /// joined without asking.
    fn bury(&self, all_finished: bool) {
        let now = Instant::now();
        let threads = {
            let mut registry = self.registry.lock().expect("work registry");
            registry.names.clear();
            // Empty unless the executor is stuck in a step: then nothing will
            // publish them again.
            registry.jobs.clear();
            std::mem::take(&mut registry.threads)
        };
        for thread in threads.into_values() {
            if all_finished || thread.handle.is_finished() {
                self.join(thread.name, thread.handle);
            } else {
                let asked = thread.asked.unwrap_or(now);
                (self.log)(&format!(
                    "[BKG] thread: {} abandoned at shutdown, still running {} ms after it was asked to stop",
                    thread.name,
                    now.saturating_duration_since(asked).as_millis()
                ));
            }
        }
        let executor = self.executor_thread.lock().expect("executor thread").take();
        if let Some(handle) = executor {
            if all_finished || handle.is_finished() {
                self.join("executor".to_string(), handle);
            } else {
                let line = match self.underway.current() {
                    Some(current) => {
                        let running = self.clock.now().saturating_duration_since(current.started);
                        format!(
                            "[BKG] executor abandoned at shutdown: {}",
                            running_for(&current, running)
                        )
                    }
                    // Between two steps: what it was doing is not one of
                    // them, and is not known.
                    None => "[BKG] executor abandoned at shutdown, between two steps".to_string(),
                };
                (self.log)(&line);
            }
        }
        let watchdog = self.watchdog.lock().expect("watchdog").take();
        if let Some(handle) = watchdog
            && (all_finished || handle.is_finished())
        {
            self.join("watchdog".to_string(), handle);
        }
    }
}
