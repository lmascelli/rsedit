//! The scheduler's core: the jobs, their queues and deadlines, the rule that
//! picks the next step, and the measurements. It has no thread and no clock of
//! its own: a caller drives it a step at a time, on the [`Clock`] it was given.
//!
//! Design: `doc/architettura-editor.typ, <nucleo>`.
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::clock::{Clock, ManualClock};
use super::job::{
    Class, Exit, Job, JobError, Spec, Start, Step, StepContext, Wake, WorkId, Worked,
};
use super::options::Options;
use super::report::{Reporter, Rule};
use super::topic::{Topic, Topics};

/// The furthest ahead a deadline is ever set: a year. A job asleep for longer
/// is asleep for good, as far as anybody can tell, and an `Instant` much
/// further out may not exist -- adding `Duration::MAX` to one panics, and here
/// that would happen on the executor's thread, outside any step.
const FURTHEST: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// Returns SPAN after NOW, but no further than [`FURTHEST`].
fn later(now: Instant, span: Duration) -> Instant {
    now + span.min(FURTHEST)
}

/// Where a job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// In the queue of its class.
    Ready,
    /// Taking a step.
    Running,
    /// Until a deadline.
    Sleeping,
    /// For one of its topics, and perhaps a deadline.
    Waiting,
}

/// A job as the core sees it: what `work-list` shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkInfo {
    pub id: WorkId,
    pub name: String,
    pub class: Class,
    pub place: Place,
    pub steps: u64,
    pub total: Duration,
    pub longest: Duration,
}

/// A topic a job is subscribed to, and the count it last saw.
struct Subscription {
    topic: Topic,
    seen: u64,
}

/// What ran while a job was ready: who to name when it starts late.
#[derive(Default)]
struct Behind {
    steps: u64,
    longest: Option<(String, Duration)>,
}

/// A job that has left, owed its `exit`.
struct Leaving<C: 'static> {
    id: WorkId,
    name: String,
    job: Box<dyn Job<C>>,
    why: Exit,
}

/// One job and everything the core keeps about it.
struct Record<C> {
    spec: Spec,
    /// `None` only while the job is taking a step: the job is out of the core
    /// then, which is what makes it impossible for it to run twice at once.
    job: Option<Box<dyn Job<C>>>,
    place: Place,
    subscriptions: Vec<Subscription>,
    /// When a sleeping or waiting job's time is up.
    deadline: Option<Instant>,
    /// When a waiting job's unseen events mature: `event-delay` after the core
    /// first noticed them.
    event_due: Option<Instant>,
    /// Why its next step happens.
    woken_by: Wake,
    /// When it became ready, or was due to.
    due_since: Instant,
    behind: Behind,
    steps: u64,
    total: Duration,
    longest: Duration,
}

impl<C> Record<C> {
    fn has_unseen_events(&self) -> bool {
        self.subscriptions
            .iter()
            .any(|subscription| subscription.topic.count() > subscription.seen)
    }
}

/// What a core is running at this moment, for other threads to look at while
/// it runs -- which is when it cannot answer for itself.
///
/// The watchdog reads it to report a step that does not end, and the registry
/// to list the job taking it as running. Written twice a step, by moves under
/// a lock nothing else holds for longer than a copy.
#[derive(Default)]
pub(crate) struct Underway(Mutex<Option<Current>>);

/// A step, or a job's `exit`, that a core is in the middle of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Current {
    /// Counted from the core's first, steps and exits together: what tells
    /// one step of a job from its next.
    pub(crate) serial: u64,
    pub(crate) id: WorkId,
    pub(crate) name: String,
    /// The job's `exit`, rather than one of its steps.
    pub(crate) exit: bool,
    pub(crate) started: Instant,
}

impl Underway {
    /// Returns what is running now, if anything.
    pub(crate) fn current(&self) -> Option<Current> {
        self.slot().clone()
    }

    fn begin(&self, current: Current) {
        *self.slot() = Some(current);
    }

    fn end(&self) {
        self.slot().take();
    }

    /// Returns the slot, locked. Nothing that can panic runs under this lock,
    /// but the watchdog reads it and must not be the thread that finds out
    /// otherwise.
    fn slot(&self) -> MutexGuard<'_, Option<Current>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The steps of the last second, for the busy-executor rule.
///
/// A sliding second, measured exactly: the share is the part of each step
/// that falls inside `[now - 1 s, now]`. A window that opened at the start of
/// one step and closed at the end of another would count one step too many,
/// and call a job that is busy 47% of the time busy for 52% of it.
#[derive(Default)]
struct Load {
    steps: VecDeque<(Instant, Instant, String)>,
    checked: Option<Instant>,
}

impl Load {
    const WINDOW: Duration = Duration::from_secs(1);

    fn record(&mut self, started: Instant, ended: Instant, name: &str) {
        self.steps.push_back((started, ended, name.to_string()));
    }

    /// Returns how busy the last second was, in percent, and with which jobs,
    /// busiest first -- at most once a second, `None` in between.
    fn check(&mut self, now: Instant) -> Option<(u64, Vec<(String, u64)>)> {
        let from = now.checked_sub(Self::WINDOW)?;
        while self
            .steps
            .front()
            .is_some_and(|(_, ended, _)| *ended <= from)
        {
            self.steps.pop_front();
        }
        if self
            .checked
            .is_some_and(|at| now.saturating_duration_since(at) < Self::WINDOW)
        {
            return None;
        }
        self.checked = Some(now);
        let mut busy: HashMap<&str, Duration> = HashMap::new();
        for (started, ended, name) in &self.steps {
            let inside = (*ended)
                .min(now)
                .saturating_duration_since((*started).max(from));
            *busy.entry(name.as_str()).or_default() += inside;
        }
        let percent = |time: Duration| (time.as_millis() * 100 / Self::WINDOW.as_millis()) as u64;
        let total = percent(busy.values().sum());
        let mut by_job: Vec<(String, u64)> = busy
            .into_iter()
            .map(|(name, time)| (name.to_string(), percent(time)))
            .collect();
        by_job.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Some((total, by_job))
    }
}

/// The core of a scheduler, driven by one thread at a time.
pub struct Core<C: 'static> {
    options: Arc<Options>,
    topics: Arc<Topics>,
    clock: Arc<dyn Clock>,
    jobs: BTreeMap<WorkId, Record<C>>,
    names: HashMap<String, WorkId>,
    high: VecDeque<WorkId>,
    normal: VecDeque<WorkId>,
    /// High steps run in a row since the last normal one.
    high_streak: u64,
    /// Jobs that have left and are owed their `exit`, which needs the state
    /// the next turn brings.
    leaving: Vec<Leaving<C>>,
    /// Jobs whose `exit` has been called, for whoever keeps a list of the
    /// jobs outside the core: see [`Core::take_departed`].
    departed: Vec<WorkId>,
    reporter: Reporter,
    load: Load,
    underway: Arc<Underway>,
    /// Steps and exits begun so far: the next one's [`Current::serial`].
    serial: u64,
    closed: bool,
}

impl<C: 'static> Core<C> {
    pub fn new(options: Arc<Options>, topics: Arc<Topics>, clock: Arc<dyn Clock>) -> Self {
        Self {
            options,
            topics,
            clock,
            jobs: BTreeMap::new(),
            names: HashMap::new(),
            high: VecDeque::new(),
            normal: VecDeque::new(),
            high_streak: 0,
            leaving: Vec::new(),
            departed: Vec::new(),
            reporter: Reporter::default(),
            load: Load::default(),
            underway: Arc::default(),
            serial: 0,
            closed: false,
        }
    }

    /// Returns where this core says what it is running, for other threads to
    /// read while it runs. See [`Underway`].
    pub(crate) fn underway(&self) -> Arc<Underway> {
        self.underway.clone()
    }

    /// Says that the job ID, called NAME, is in a step -- or in its `exit` --
    /// from STARTED on.
    fn begin(&mut self, id: WorkId, name: &str, exit: bool, started: Instant) {
        self.serial += 1;
        self.underway.begin(Current {
            serial: self.serial,
            id,
            name: name.to_string(),
            exit,
            started,
        });
    }

    /// Takes JOB, under the name in SPEC, and returns its number.
    ///
    /// A job that already has the name leaves, as [`Exit::Cancelled`], on the
    /// next turn. `None` once the core has shut down: the job is not taken, and
    /// its `exit` is not called -- the caller knows it was refused.
    pub fn submit(&mut self, spec: Spec, job: Box<dyn Job<C>>) -> Option<WorkId> {
        let id = WorkId::next();
        self.submit_as(id, spec, job).then_some(id)
    }

    /// Takes JOB as [`Self::submit`] does, under a number the caller has
    /// already handed out.
    ///
    /// For the WorkManager, which answers `submit` at once with the number and
    /// passes the job on to the executor's thread: the number has to exist
    /// before the core sees the job. False when the core has shut down.
    pub(crate) fn submit_as(&mut self, id: WorkId, spec: Spec, job: Box<dyn Job<C>>) -> bool {
        if self.closed {
            return false;
        }
        if let Some(&old) = self.names.get(&spec.name) {
            self.cancel(old);
        }
        let now = self.clock.now();
        let subscriptions = spec
            .topics
            .iter()
            .map(|name| {
                let topic = self.topics.get(name);
                // What was emitted before the job existed is not news to it:
                // its first step looks at the state anyway.
                Subscription {
                    seen: topic.count(),
                    topic,
                }
            })
            .collect();
        let (place, deadline) = match spec.start {
            Start::Now => (Place::Ready, None),
            Start::After(delay) => (Place::Sleeping, Some(later(now, delay))),
        };
        if place == Place::Ready {
            self.queue(spec.class).push_back(id);
        }
        self.names.insert(spec.name.clone(), id);
        self.jobs.insert(
            id,
            Record {
                spec,
                job: Some(job),
                place,
                subscriptions,
                deadline,
                event_due: None,
                woken_by: Wake::Start,
                due_since: now,
                behind: Behind::default(),
                steps: 0,
                total: Duration::ZERO,
                longest: Duration::ZERO,
            },
        );
        true
    }

    /// Stops the job ID. It leaves on the next turn, as [`Exit::Cancelled`],
    /// without another step. False when there is no such job.
    pub fn cancel(&mut self, id: WorkId) -> bool {
        let Some(mut record) = self.jobs.remove(&id) else {
            return false;
        };
        if self.names.get(&record.spec.name) == Some(&id) {
            self.names.remove(&record.spec.name);
        }
        self.high.retain(|queued| *queued != id);
        self.normal.retain(|queued| *queued != id);
        // A job taking a step is not here to take: `run_step` finds its record
        // gone when the step ends, and makes it leave then.
        if let Some(job) = record.job.take() {
            self.leaving.push(Leaving {
                id,
                name: record.spec.name,
                job,
                why: Exit::Cancelled,
            });
        }
        true
    }

    /// Stops the job called NAME. False when there is none.
    pub fn cancel_named(&mut self, name: &str) -> bool {
        match self.names.get(name) {
            Some(&id) => self.cancel(id),
            None => false,
        }
    }

    /// Shuts the core down: every job leaves, as [`Exit::Shutdown`], and
    /// nothing is taken after.
    pub fn shutdown(&mut self, state: &C) {
        self.closed = true;
        for (id, mut record) in std::mem::take(&mut self.jobs) {
            if let Some(job) = record.job.take() {
                self.leaving.push(Leaving {
                    id,
                    name: record.spec.name,
                    job,
                    why: Exit::Shutdown,
                });
            }
        }
        self.names.clear();
        self.high.clear();
        self.normal.clear();
        self.flush_leaving(state);
    }

    /// Runs one turn of an executor's loop: the jobs that are leaving leave,
    /// the deadlines that passed and the events that matured make jobs ready,
    /// and at most one step runs. True when a step ran.
    pub fn run_once(&mut self, state: &C) -> bool {
        self.flush_leaving(state);
        let now = self.clock.now();
        self.poll(now);
        self.tick(now);
        let Some(id) = self.choose() else {
            return false;
        };
        self.run_step(id, state);
        self.flush_leaving(state);
        true
    }

    /// Runs turns until no job is ready, or until LIMIT steps have run: a job
    /// that always answers `Continue` would otherwise never let this return.
    /// Returns the number of steps that ran.
    pub fn run_until_idle(&mut self, state: &C, limit: usize) -> usize {
        let mut steps = 0;
        while steps < limit && self.run_once(state) {
            steps += 1;
        }
        steps
    }

    /// Runs SPAN of scheduling on CLOCK -- the clock this core reads -- moving
    /// it from one wake-up to the next instead of waiting: what the core's
    /// tests use. LIMIT bounds the steps, as in [`Self::run_until_idle`].
    pub fn run_for(
        &mut self,
        state: &C,
        clock: &ManualClock,
        span: Duration,
        limit: usize,
    ) -> usize {
        let end = clock.now() + span;
        let mut steps = 0;
        while steps < limit {
            if self.run_once(state) {
                steps += 1;
                continue;
            }
            match self.next_wakeup() {
                // A wake-up that is not in the future, with nothing ready,
                // would be a turn that changes nothing: stop rather than spin.
                Some(at) if at <= end && at > clock.now() => clock.advance_to(at),
                _ => break,
            }
        }
        clock.advance_to(end);
        steps
    }

    /// Returns when the next turn has something to do: now when a job is
    /// ready or leaving, otherwise the earliest deadline, maturing event or
    /// summary. `None` when only a new job or an event can bring work.
    pub fn next_wakeup(&self) -> Option<Instant> {
        let now = self.clock.now();
        if !self.leaving.is_empty() || !self.high.is_empty() || !self.normal.is_empty() {
            return Some(now);
        }
        let delay = self.options.event_delay();
        let jobs = self.jobs.values().flat_map(|record| {
            let event = match record.place {
                Place::Waiting => record
                    .event_due
                    .or_else(|| record.has_unseen_events().then(|| now + delay)),
                _ => None,
            };
            [record.deadline, event]
        });
        let summary = self.reporter.next_tick(self.options.summary_every());
        jobs.chain([summary]).flatten().min()
    }

    /// Returns the report lines written since the last call, oldest first.
    pub fn take_reports(&mut self) -> Vec<String> {
        self.reporter.take()
    }

    /// Returns the jobs that have left since the last call, their `exit`
    /// already called: what a list of the jobs kept outside the core drops.
    pub fn take_departed(&mut self) -> Vec<WorkId> {
        std::mem::take(&mut self.departed)
    }

    /// Returns every job the core holds, in the order they were submitted.
    pub fn info(&self) -> Vec<WorkInfo> {
        self.jobs
            .iter()
            .map(|(id, record)| WorkInfo {
                id: *id,
                name: record.spec.name.clone(),
                class: record.spec.class,
                place: record.place,
                steps: record.steps,
                total: record.total,
                longest: record.longest,
            })
            .collect()
    }

    fn queue(&mut self, class: Class) -> &mut VecDeque<WorkId> {
        match class {
            Class::High => &mut self.high,
            Class::Normal => &mut self.normal,
        }
    }

    /// Makes ready every job whose deadline passed or whose events matured.
    fn poll(&mut self, now: Instant) {
        let delay = self.options.event_delay();
        let mut due = Vec::new();
        for (id, record) in &mut self.jobs {
            match record.place {
                Place::Waiting => {
                    if record.event_due.is_none() && record.has_unseen_events() {
                        record.event_due = Some(now + delay);
                    }
                }
                // A sleeping job ignores events: its next step sees what they
                // announced anyway.
                Place::Sleeping => {}
                Place::Ready | Place::Running => continue,
            }
            let timer = record.deadline.filter(|at| *at <= now);
            let event = record.event_due.filter(|at| *at <= now);
            let wake = match (event, timer) {
                (Some(event), Some(timer)) if event <= timer => Some((event, Wake::Event)),
                (_, Some(timer)) => Some((timer, Wake::Timer)),
                (Some(event), None) => Some((event, Wake::Event)),
                (None, None) => None,
            };
            if let Some((at, why)) = wake {
                due.push((at, *id, why));
            }
        }
        // In the order they fell due, and by number among equals: the same
        // inputs always give the same queue.
        due.sort_by_key(|(at, id, _)| (*at, *id));
        for (at, id, why) in due {
            let record = self.jobs.get_mut(&id).expect("a due job has a record");
            record.place = Place::Ready;
            record.deadline = None;
            record.event_due = None;
            record.woken_by = if record.steps == 0 { Wake::Start } else { why };
            record.due_since = at;
            record.behind = Behind::default();
            let class = record.spec.class;
            self.queue(class).push_back(id);
        }
    }

    /// Chooses the next job by the rule: high first, but after `high-steps`
    /// high steps in a row a ready normal job goes. Counted in steps, not
    /// time, so that it is deterministic.
    fn choose(&mut self) -> Option<WorkId> {
        let normal_due = !self.normal.is_empty()
            && (self.high.is_empty() || self.high_streak >= self.options.high_steps());
        if normal_due {
            self.high_streak = 0;
            self.normal.pop_front()
        } else {
            let id = self.high.pop_front()?;
            self.high_streak += 1;
            Some(id)
        }
    }

    /// Takes one step of ID, measures it, and applies its answer.
    fn run_step(&mut self, id: WorkId, state: &C) {
        let started = self.clock.now();
        let record = self.jobs.get_mut(&id).expect("a chosen job has a record");
        // Taken before the step, not after: an event that arrives while the
        // step runs stays unseen, and wakes the job again.
        for subscription in &mut record.subscriptions {
            subscription.seen = subscription.topic.count();
        }
        let mut job = record.job.take().expect("a queued job is not running");
        record.place = Place::Running;
        let woken_by = record.woken_by;
        let waited = started.saturating_duration_since(record.due_since);
        let behind = std::mem::take(&mut record.behind);
        let name = record.spec.name.clone();
        let class = record.spec.class;
        let unit = record.spec.unit.clone();

        if waited > self.options.late(class) {
            let after = match &behind.longest {
                Some((longest, _)) => format!(", behind {longest} ({} steps)", behind.steps),
                None => String::new(),
            };
            let line = format!(
                "[BKG] late start: {name} started {} ms late{after}",
                waited.as_millis()
            );
            self.reporter.report(
                &name,
                Rule::LateStart,
                waited.as_millis() as u64,
                line,
                started,
            );
        }

        let mut cx = StepContext {
            state,
            woken_by,
            id,
            worked: None,
            topics: None,
        };
        self.begin(id, &name, false, started);
        let result = catch_unwind(AssertUnwindSafe(|| job.step(&mut cx)));
        self.underway.end();
        let worked = cx.worked.take();
        let topics = cx.topics.take();
        let ended = self.clock.now();
        let took = ended.saturating_duration_since(started);

        self.measure(id, &name, &unit, took, worked, started);

        // Cancelled while it ran: nothing to come back to.
        if !self.jobs.contains_key(&id) {
            self.leaving.push(Leaving {
                id,
                name,
                job,
                why: Exit::Cancelled,
            });
            return;
        }
        match result {
            Ok(Ok(step)) => {
                let fresh = topics.is_some_and(|names| self.resubscribe(id, names));
                self.apply(id, job, step, ended, fresh);
            }
            Ok(Err(JobError(message))) => {
                self.reporter
                    .always(format!("[BKG] error: {name}: {message}"));
                self.leave(id, job, Exit::Failed);
            }
            Err(payload) => {
                self.reporter.always(format!(
                    "[BKG] panic: {name} retired: {}",
                    panic_message(&*payload)
                ));
                self.leave(id, job, Exit::Panicked);
            }
        }
    }

    /// Records a step of job ID, called NAME, that took TOOK; reports it if it
    /// was long.
    fn measure(
        &mut self,
        id: WorkId,
        name: &str,
        unit: &str,
        took: Duration,
        worked: Option<Worked>,
        started: Instant,
    ) {
        if let Some(record) = self.jobs.get_mut(&id) {
            record.steps += 1;
            record.total += took;
            record.longest = record.longest.max(took);
        }
        // Everything still in a queue waited behind this step.
        let queued: Vec<WorkId> = self.high.iter().chain(&self.normal).copied().collect();
        for id in queued {
            if let Some(record) = self.jobs.get_mut(&id) {
                record.behind.steps += 1;
                if record
                    .behind
                    .longest
                    .as_ref()
                    .is_none_or(|(_, longest)| took > *longest)
                {
                    record.behind.longest = Some((name.to_string(), took));
                }
            }
        }
        self.load.record(started, started + took, name);

        if took > self.options.long_step() {
            // A job may say how much it did without saying on what: a Lisp
            // job counts the fuel it spent, and has no file to name.
            let on = |on: &str| match on {
                "" => String::new(),
                on => format!(" on {on}"),
            };
            let what = match worked {
                Some(Worked { units, on: what }) if units > 0 => format!(
                    "{} ({units} {unit}, {} each)",
                    on(&what),
                    per_unit(took, units)
                ),
                Some(Worked { on: what, .. }) => on(&what),
                None => String::new(),
            };
            let line = format!("[BKG] long step: {name}, {} ms{what}", took.as_millis());
            self.reporter
                .report(name, Rule::LongStep, took.as_millis() as u64, line, started);
        }
    }

    /// Makes NAMES the topics of job ID, as its step asked: a topic it already
    /// had keeps the count it last saw, a new one starts from now. True when
    /// one of them is new -- see [`StepContext::subscribe`] for why that
    /// counts as a change.
    fn resubscribe(&mut self, id: WorkId, names: Vec<String>) -> bool {
        let topics = &self.topics;
        let Some(record) = self.jobs.get_mut(&id) else {
            return false;
        };
        let mut old = std::mem::take(&mut record.subscriptions);
        let mut fresh = false;
        for name in names {
            if record
                .subscriptions
                .iter()
                .any(|subscription| subscription.topic.name() == name)
            {
                continue;
            }
            let subscription = match old
                .iter()
                .position(|subscription| subscription.topic.name() == name)
            {
                Some(at) => old.swap_remove(at),
                None => {
                    fresh = true;
                    let topic = topics.get(&name);
                    Subscription {
                        seen: topic.count(),
                        topic,
                    }
                }
            };
            record.subscriptions.push(subscription);
        }
        fresh
    }

    /// Puts job ID back where its answer STEP says. FRESH: the step named a
    /// topic the job did not have, which counts as a change.
    fn apply(&mut self, id: WorkId, job: Box<dyn Job<C>>, step: Step, now: Instant, fresh: bool) {
        let delay = self.options.event_delay();
        let record = self
            .jobs
            .get_mut(&id)
            .expect("a job that stepped has a record");
        match step {
            Step::Continue => {
                record.job = Some(job);
                record.place = Place::Ready;
                record.woken_by = Wake::Turn;
                record.due_since = now;
                let class = record.spec.class;
                self.queue(class).push_back(id);
            }
            Step::Sleep(span) => {
                record.job = Some(job);
                record.place = Place::Sleeping;
                record.deadline = Some(later(now, span));
            }
            Step::Wait if record.subscriptions.is_empty() => {
                let name = record.spec.name.clone();
                self.reporter.always(format!(
                    "[BKG] error: {name} waits for an event but has no topic to be woken by"
                ));
                self.leave(id, job, Exit::Failed);
            }
            Step::Wait | Step::WaitAtMost(_) => {
                record.job = Some(job);
                record.place = Place::Waiting;
                record.deadline = match step {
                    Step::WaitAtMost(span) => Some(later(now, span)),
                    _ => None,
                };
                // Events that arrived during the step mature from now.
                record.event_due = (fresh || record.has_unseen_events()).then(|| now + delay);
            }
            Step::Done => self.leave(id, job, Exit::Finished),
        }
    }

    /// Makes job ID leave, for WHY. Its `exit` comes with the next state.
    fn leave(&mut self, id: WorkId, job: Box<dyn Job<C>>, why: Exit) {
        if let Some(record) = self.jobs.remove(&id) {
            if self.names.get(&record.spec.name) == Some(&id) {
                self.names.remove(&record.spec.name);
            }
            self.leaving.push(Leaving {
                id,
                name: record.spec.name,
                job,
                why,
            });
        }
    }

    /// Calls `exit` on every job that left.
    fn flush_leaving(&mut self, state: &C) {
        for Leaving {
            id,
            name,
            mut job,
            why,
        } in std::mem::take(&mut self.leaving)
        {
            // Guarded like a step: an `exit` that panics must not take the
            // scheduler down with it. And watched like one: an `exit` that
            // never returns holds the executor as surely as a step.
            let started = self.clock.now();
            self.begin(id, &name, true, started);
            let exited = catch_unwind(AssertUnwindSafe(|| job.exit(state, why)));
            self.underway.end();
            if let Err(payload) = exited {
                self.reporter.always(format!(
                    "[BKG] panic: {name} panicked in exit: {}",
                    panic_message(&*payload)
                ));
            }
            self.departed.push(id);
        }
    }

    /// Closes what is due to close: the second of load, and the report
    /// windows.
    fn tick(&mut self, now: Instant) {
        if let Some((share, by_job)) = self.load.check(now)
            && share > self.options.busy_share()
        {
            let jobs: Vec<String> = by_job
                .iter()
                .take(3)
                .map(|(name, share)| format!("{name} {share}%"))
                .collect();
            let line = format!(
                "[BKG] load: {share}% of the last second ({})",
                jobs.join(", ")
            );
            self.reporter
                .report("executor", Rule::Load, share, line, now);
        }
        self.reporter.tick(now, self.options.summary_every());
    }
}

/// Returns TOOK shared out over UNITS, at least one: in milliseconds, or in
/// micro- or nanoseconds when there is less than a millisecond each. A line of
/// colour costs milliseconds; a step of fuel costs a fraction of a
/// microsecond, and "0 ms each" would say nothing.
fn per_unit(took: Duration, units: u64) -> String {
    let each = took.as_nanos() / u128::from(units);
    match each {
        1_000_000.. => format!("{} ms", each / 1_000_000),
        1_000.. => format!("{} µs", each / 1_000),
        _ => format!("{each} ns"),
    }
}

/// Returns what a panic said, when it said it with a string.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_string())
}
