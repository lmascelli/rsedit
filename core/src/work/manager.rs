//! The WorkManager: where the editor's asynchronous work runs.
//!
//! One per editor, shared by every clone of it. It owns the scheduler -- the
//! [`Core`] and the executor that drives it -- and beside it the registry of
//! what is running, the options, the topics, the dedicated threads and the
//! watchdog. See `doc/workmanager.typ`, "L'architettura".
//!
//! # The watchdog
//!
//! A thread of its own, from [`WorkManager::start`] to the end, that wakes
//! every `watchdog-every` and looks at what nobody else can: the step the
//! executor is in, which the executor cannot report while it is in it. A step
//! running `stuck-step` is reported, and again each time it has run twice as
//! long. It joins the dedicated threads that have finished, and reports those
//! slow to stop. And it calls the host's [`WorkManager::after_watch`], with
//! which the editor writes its log file: so a write that blocks holds up the
//! watchdog and nobody else, and a report reaches the file even when what it
//! is about is stuck.
//!
//! # Two modes
//!
//! - **Threaded**, the editor's: an executor thread runs the core on the real
//!   clock, from [`WorkManager::start`] to [`WorkManager::shutdown`].
//! - **Manual**, the tests': nothing runs by itself. The core reads a
//!   [`ManualClock`], and work happens when the test calls
//!   [`WorkManager::run_until_idle`] or [`WorkManager::run_for`].
//!
//! The rest is the same in both, down to the way a job gets in. A call never
//! touches the core: it changes the registry, which answers at once, and
//! leaves a message in the executor's inbox, which the executor reads between
//! two steps. So `submit` from inside a step is as safe as from anywhere --
//! in manual mode too, where the test's own thread is the executor.
//!
//! # Why `shutdown` is explicit
//!
//! The executor holds a copy of the state, and in the editor the state holds
//! the WorkManager: a cycle of references, which `Drop` cannot break. The same
//! cycle kept the scheduler before this one alive for ever, one thread for
//! every editor ever built. So the host calls [`WorkManager::shutdown`] when
//! the editor exits, and a test that starts threads calls it too.
use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::clock::{Clock, ManualClock, SystemClock};
use super::core::{Core, Current, Place, Underway, WorkInfo, panic_message};
use super::job::{Job, Spec, Start, WorkId};
use super::options::Options;
use super::threads::{Dedicated, Lives, Stop, ThreadInfo, start_thread};
use super::topic::{Topic, Topics};

/// How often a busy executor brings the registry's measurements up to date.
/// An idle one does it before it sleeps.
const PUBLISH_EVERY: Duration = Duration::from_millis(100);

/// The most steps one call of the manual mode runs: a job that always answers
/// `Continue` must not hang a test.
const MANUAL_LIMIT: usize = 100_000;

/// The asynchronous work of one editor. Cheap to clone: every clone is the
/// same WorkManager.
pub struct WorkManager<C: 'static> {
    shared: Arc<Shared<C>>,
}

impl<C: 'static> Clone for WorkManager<C> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

/// What the WorkManager tells its executor.
///
/// All but [`Msg::Nudge`] are sent with the registry's lock held. So the
/// inbox holds them in the order the registry changed, and once
/// [`Msg::Shutdown`] is in, nothing else will be.
enum Msg<C: 'static> {
    Submit(WorkId, Spec, Box<dyn Job<C>>),
    Cancel(WorkId),
    /// A topic was emitted: look again. Sent from wherever the emission
    /// happened, and never more than one waiting in the inbox.
    Nudge,
    Shutdown,
}

/// Everything running, by number and by name: the WorkManager's answer to
/// what exists, given at once and without asking the executor.
#[derive(Default)]
struct Registry {
    /// Set by `shutdown`: nothing is taken after.
    closed: bool,
    /// One namespace for jobs and threads: a name means one thing, and
    /// stopping it by name stops whichever it is.
    names: HashMap<String, WorkId>,
    /// The newest work submitted under each name, until it is stopped: kept
    /// after it finishes, unlike `names`. See [`WorkManager::is_latest`].
    latest: HashMap<String, WorkId>,
    /// The jobs, with what the executor last published about them.
    jobs: BTreeMap<WorkId, WorkInfo>,
    threads: BTreeMap<WorkId, Dedicated>,
}

/// What taking work out of the registry leaves to do once the lock is given
/// back.
enum Withdrawn {
    /// There was nothing to take.
    Nothing,
    /// A job. The executor has been told.
    Job,
    /// A thread, by name, whose stop is to be raised.
    Thread(String, Stop),
}

impl Registry {
    /// Take the work numbered ID out of the registry. A job leaves at once and
    /// the executor is told; a thread stays listed, as stopping, until it has
    /// finished. After `shutdown` there is nothing to take: everything has
    /// been told to stop already, and the executor must hear nothing more.
    fn withdraw<C: 'static>(&mut self, id: WorkId, inbox: &Sender<Msg<C>>) -> Withdrawn {
        if self.closed {
            return Withdrawn::Nothing;
        }
        if let Some(job) = self.jobs.remove(&id) {
            self.forget_name(&job.name, id);
            self.silence(&job.name, id);
            let _ = inbox.send(Msg::Cancel(id));
            return Withdrawn::Job;
        }
        let Some(thread) = self.threads.get_mut(&id) else {
            return Withdrawn::Nothing;
        };
        thread.asked.get_or_insert_with(Instant::now);
        let (name, stop) = (thread.name.clone(), thread.stop.clone());
        self.forget_name(&name, id);
        self.silence(&name, id);
        Withdrawn::Thread(name, stop)
    }

    /// [`Self::withdraw`] the work called NAME.
    fn withdraw_named<C: 'static>(&mut self, name: &str, inbox: &Sender<Msg<C>>) -> Withdrawn {
        match self.names.get(name) {
            Some(&id) => self.withdraw(id, inbox),
            None => Withdrawn::Nothing,
        }
    }

    /// Free NAME, if it still names ID.
    fn forget_name(&mut self, name: &str, id: WorkId) {
        if self.names.get(name) == Some(&id) {
            self.names.remove(name);
        }
    }

    /// ID has been stopped: it is no longer the latest under NAME, if it was.
    fn silence(&mut self, name: &str, id: WorkId) {
        if self.latest.get(name) == Some(&id) {
            self.latest.remove(name);
        }
    }
}

/// What every clone of a WorkManager shares.
struct Shared<C: 'static> {
    /// The number in this WorkManager's thread names: `w{instance} executor`.
    instance: u64,
    options: Arc<Options>,
    topics: Arc<Topics>,
    /// Where the report lines go: in the editor, its log.
    log: Box<dyn Fn(&str) + Send + Sync>,
    /// Called after every step: see [`WorkManager::after_step`].
    after_step: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// Called after every turn of the watchdog: see
    /// [`WorkManager::after_watch`].
    after_watch: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// Set while a nudge is in the inbox, unread: what makes a burst of
    /// emissions one wake-up.
    nudged: Arc<AtomicBool>,
    inbox: Sender<Msg<C>>,
    registry: Mutex<Registry>,
    /// The executor, until a thread takes it -- never, in manual mode.
    executor: Mutex<Option<Executor<C>>>,
    /// The clock the core reads, which the watchdog times its steps by.
    clock: Arc<dyn Clock>,
    /// The same clock, in manual mode, where the test moves it.
    manual: Option<Arc<ManualClock>>,
    /// What the executor is running, as it says while it runs.
    underway: Arc<Underway>,
    /// What the watchdog has said about it.
    watching: Mutex<Watching>,
    lives: Arc<Lives>,
    executor_thread: Mutex<Option<JoinHandle<()>>>,
    watchdog: Mutex<Option<JoinHandle<()>>>,
}

/// What the watchdog has said about the step in progress, so that it says it
/// again only once the step has run twice as long.
#[derive(Default)]
struct Watching {
    /// Which step it was: [`Current::serial`].
    serial: Option<u64>,
    /// How long that step has to run before it is reported again.
    next: Duration,
}

impl<C: Send + 'static> WorkManager<C> {
    /// A WorkManager for the editor: an executor thread, from [`Self::start`]
    /// on, and the real clock. Report lines go to LOG.
    pub fn threaded(log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self::build(Arc::new(SystemClock), None, Box::new(log))
    }

    /// A WorkManager for tests: nothing runs until the test says so, on a
    /// clock that moves when it is told to. Report lines go to LOG.
    pub fn manual(log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        let clock = ManualClock::new();
        Self::build(clock.clone(), Some(clock), Box::new(log))
    }

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

    /// Start the executor's thread, on STATE, and the watchdog's.
    ///
    /// Called once the state exists, which in the editor is after the
    /// WorkManager: the state holds it. Work submitted before waits in the
    /// inbox. In manual mode there is no thread to start, and STATE is not
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

    /// Hand JOB to the scheduler, under the name in SPEC, and answer with its
    /// number at once: the job joins the queue at the executor's next turn.
    ///
    /// Work that already has the name -- a job or a thread -- is stopped. `None`
    /// after [`Self::shutdown`], and then JOB is not kept: its `exit` is not
    /// called, the caller knows it was refused.
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

    /// Start BODY on a dedicated thread called NAME, and answer with its
    /// number: work that may block, which no step may do.
    ///
    /// BODY is handed the [`Stop`] that [`Self::cancel`] raises, and its own
    /// number, as a job's step is: what it can tell its own writes by. Work
    /// that already has the name is stopped, as with [`Self::submit`]. `None`
    /// after [`Self::shutdown`], or when the system will not give a thread.
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

    /// Stop the work numbered ID.
    ///
    /// A job leaves at the end of its step, or before its next one, with
    /// `Exit::Cancelled`. A thread has its [`Stop`] raised and its action run,
    /// here, and is listed as stopping until it has. False when there is no
    /// such work: it has finished, or never was.
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

    /// [`Self::cancel`] the work called NAME.
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

    /// Have HOOK called after every step, on the thread that took it: how the
    /// host hears that a job may have changed something it shows, without
    /// asking. HOOK runs between two steps, so it must be quick and must not
    /// wait: in the editor it raises a flag and, the first time, sends the
    /// renderer a message.
    ///
    /// Set once, and before the work it is for starts; a second call is
    /// ignored and answers false.
    pub fn after_step(&self, hook: impl Fn() + Send + Sync + 'static) -> bool {
        self.shared.after_step.set(Box::new(hook)).is_ok()
    }

    /// Have HOOK called at the end of every turn of the watchdog, on its
    /// thread, and once more at the end of [`Self::shutdown`], on the caller's,
    /// for the lines `shutdown` itself writes.
    ///
    /// What the editor writes its log file with. On the watchdog's thread a
    /// write that blocks -- a full disk, a home directory on the network --
    /// holds up nothing else, and what the watchdog reports reaches the file
    /// even when the thread it reports on is stuck.
    ///
    /// Set once, and before the work it is for starts; a second call is
    /// ignored and answers false.
    pub fn after_watch(&self, hook: impl Fn() + Send + Sync + 'static) -> bool {
        self.shared.after_watch.set(Box::new(hook)).is_ok()
    }

    /// Whether work -- a job or a thread -- is running under NAME.
    ///
    /// For whoever names work after something else that must be unique too: a
    /// shell command takes the name of the buffer it writes into, and that
    /// name has to be free here as well, or starting the command would stop
    /// whatever already had it.
    pub fn is_named(&self, name: &str) -> bool {
        let registry = self.shared.registry.lock().expect("work registry");
        registry.names.contains_key(name)
    }

    /// Whether ID is the newest work submitted under NAME, and has not been
    /// stopped -- which stays true after it has finished.
    ///
    /// What the editor asks before it delivers a callback a job queued: the
    /// job may have finished since, and its ON-DONE is exactly the call that
    /// must still be made, but a newer job under the name, or a stop, silences
    /// it. Answered by the registry, which changes in one step with the work
    /// it describes, so that two submissions under one name, or a submission
    /// and a stop, can never leave it naming the wrong one.
    pub fn is_latest(&self, name: &str, id: WorkId) -> bool {
        let registry = self.shared.registry.lock().expect("work registry");
        registry.latest.get(name) == Some(&id)
    }

    /// The topic called NAME, made on first use. See [`Topics::get`].
    pub fn topic(&self, name: &str) -> Topic {
        self.shared.topics.get(name)
    }

    /// The scheduler's options, which take effect from the next step.
    pub fn options(&self) -> &Options {
        &self.shared.options
    }

    /// Every job, in the order they were submitted, with what the executor
    /// last published about them. It publishes when it runs out of work -- in
    /// manual mode, at the end of each drive -- and every [`PUBLISH_EVERY`]
    /// while it is busy. A job it has not seen yet is listed as it was
    /// submitted: ready, or sleeping for a delayed start, with no steps.
    ///
    /// The job in the middle of a step is listed as running, from what the
    /// executor says as it starts the step rather than from what it last
    /// published: a step that does not end is a step after which nothing is
    /// published again.
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

    /// Every dedicated thread still running, in the order they were started.
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

    /// Stop everything, giving it WITHIN to finish.
    ///
    /// Nothing is taken from here on. Every job leaves with `Exit::Shutdown`,
    /// and every dedicated thread is asked to stop. Whatever has not finished
    /// by the deadline -- a thread that does not stop, a step that does not
    /// end -- is left behind and named in the log. STATE is what the jobs'
    /// `exit` is handed when no executor thread is there to hand its own: in
    /// manual mode, or before [`Self::start`]. A second call does nothing.
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

    /// Manual mode: turns until no job is ready, applying what each one
    /// submits on the way, at most [`MANUAL_LIMIT`] steps. The number of
    /// steps that ran.
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

    /// Manual mode: SPAN of scheduling, the clock moved from one wake-up to
    /// the next instead of waited for. The number of steps that ran. The
    /// clock ends SPAN later than it started, whatever ran.
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

    /// Manual mode: move the clock forward by BY, running nothing.
    ///
    /// # Panics
    ///
    /// On a threaded WorkManager, which reads the real clock.
    pub fn advance(&self, by: Duration) {
        self.manual_clock().advance(by);
    }

    /// Manual mode: one turn of the watchdog, here and now -- what its thread
    /// does every `watchdog-every`. The dedicated threads that have finished
    /// are joined, a step that has run too long is reported, and the hook set
    /// with [`Self::after_watch`] is called.
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

    /// The number in this WorkManager's thread names: `w{instance} ...`.
    pub(crate) fn instance(&self) -> u64 {
        self.shared.instance
    }

    fn manual_clock(&self) -> &Arc<ManualClock> {
        self.shared
            .manual
            .as_ref()
            .expect("only a manual WorkManager is driven by hand: this one has an executor thread")
    }

    fn thread_name(&self, role: &str) -> String {
        format!("w{} {role}", self.shared.instance)
    }

    /// Start the watchdog, if it is not running: with the executor, or with
    /// the first dedicated thread if that comes first. In manual mode there is
    /// none: the threads are reaped whenever the test drives the WorkManager,
    /// and the test takes the watchdog's turns itself.
    ///
    /// Called with the registry's lock held, so a failure comes back as the
    /// line to log once the lock is given back.
    fn watch(&self) -> Option<String> {
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
    /// Do what WITHDRAWN left to do, with no lock held: raise a thread's
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

    /// Raise STOP, and run the action it hands back: here, on the caller's
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

    /// Call the hook set with [`WorkManager::after_step`], if there is one:
    /// guarded like a step, since it is the host's code on the executor.
    fn stepped(&self) {
        if let Some(hook) = self.after_step.get()
            && let Err(payload) = catch_unwind(AssertUnwindSafe(hook))
        {
            (self.log)(&format!(
                "[BKG] panic: the after-step hook: {}",
                panic_message(&*payload)
            ));
        }
    }

    /// Call the hook set with [`WorkManager::after_watch`], if there is one:
    /// guarded like a step, since it is the host's code on the watchdog.
    fn watched(&self) {
        if let Some(hook) = self.after_watch.get()
            && let Err(payload) = catch_unwind(AssertUnwindSafe(hook))
        {
            (self.log)(&format!(
                "[BKG] panic: the after-watch hook: {}",
                panic_message(&*payload)
            ));
        }
    }

    /// One turn of the watchdog: join the threads that have finished, report
    /// a step that has run too long, and hand over to the host's hook last, so
    /// that what the turn reported is in what the hook writes.
    fn watch_turn(&self) {
        self.reap();
        self.watch_step();
        self.watched();
    }

    /// Report the step in progress once it has run `stuck-step`, and again
    /// each time it has run twice as long as when it was last reported:
    /// 1 s, 2 s, 4 s, with the default. A turn that comes late reports once,
    /// for how long the step has run, and the next mark is past that.
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

    /// Join the dedicated threads that have finished, and report those still
    /// running `stuck-step` after they were asked to stop.
    fn reap(&self) {
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

    /// Join HANDLE, a thread that has finished: a panic in it goes to the log.
    fn join(&self, name: String, handle: JoinHandle<()>) {
        if let Err(payload) = handle.join() {
            (self.log)(&format!(
                "[BKG] panic: thread {name}: {}",
                panic_message(&*payload)
            ));
        }
    }

    /// The end of `shutdown`, after the wait: join what has finished, and
    /// leave behind, named in the log, what has not. ALL_FINISHED says that
    /// every thread is done -- possibly a moment before it can be joined, so
    /// it is joined without asking.
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

/// The core and the far end of its inbox: what an executor runs. Driven by
/// the executor's thread or, in manual mode, by the test.
struct Executor<C: 'static> {
    core: Core<C>,
    inbox: Receiver<Msg<C>>,
    /// [`Msg::Shutdown`] has been read: no step runs after it.
    shut: bool,
}

impl<C: 'static> Executor<C> {
    /// Apply every message waiting.
    fn drain(&mut self) {
        while let Ok(msg) = self.inbox.try_recv() {
            self.apply(msg);
        }
    }

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

    /// One turn of the executor's loop: the messages, then at most one step.
    /// True when a step ran.
    fn turn(&mut self, shared: &Shared<C>, state: &C) -> bool {
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

    /// Pass on what the turn left: report lines to the log, and the jobs that
    /// left to the registry.
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

    /// Bring the registry's measurements up to date. A job the registry no
    /// longer lists -- cancelled, replaced -- is not put back.
    fn publish(&self, shared: &Shared<C>) {
        let infos = self.core.info();
        let mut registry = shared.registry.lock().expect("work registry");
        for info in infos {
            if let Some(entry) = registry.jobs.get_mut(&info.id) {
                *entry = info;
            }
        }
    }

    /// The end: what is still in the inbox is applied, then every job leaves,
    /// with `Exit::Shutdown`.
    fn close(&mut self, shared: &Shared<C>, state: &C) {
        self.drain();
        self.core.shutdown(state);
        self.settle(shared);
    }
}

/// The executor's thread, from `start` to `shutdown`: the loop in
/// `doc/workmanager.typ`, "Il ciclo dell'esecutore".
fn executor_main<C: 'static>(shared: Arc<Shared<C>>, mut executor: Executor<C>, state: C) {
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

/// The watchdog's thread: a turn every `watchdog-every`, until `shutdown`.
/// See the module documentation, and [`Shared::watch_turn`] for the turn.
/// Once it has stopped, `shutdown` calls the hook one last time itself, after
/// the lines it writes.
fn watchdog_main<C: 'static>(shared: Arc<Shared<C>>) {
    loop {
        std::thread::park_timeout(shared.options.watchdog_every());
        if shared.registry.lock().expect("work registry").closed {
            break;
        }
        shared.watch_turn();
    }
}

/// CURRENT, running for RUNNING, as a report says it:
/// `highlighter running for 2.0 s`, and `... in its exit`.
fn running_for(current: &Current, running: Duration) -> String {
    let exit = if current.exit { " in its exit" } else { "" };
    format!(
        "{} running for {:.1} s{exit}",
        current.name,
        running.as_secs_f64()
    )
}
