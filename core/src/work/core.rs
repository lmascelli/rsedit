//! The scheduler's core: the jobs, their queues and deadlines, the rule that
//! picks the next step, and the measurements.
//!
//! See `doc/workmanager.typ`, "La logica dello scheduler". In short:
//!
//! - A job is in one place at a time: ready (in the queue of its class),
//!   running, sleeping until a deadline, or waiting for one of its topics --
//!   with a deadline too, for [`Step::WaitAtMost`].
//! - [`Core::run_once`] is one turn of an executor's loop: jobs that are
//!   leaving leave, deadlines that passed and events that matured make jobs
//!   ready, and at most one step runs, measured.
//! - The next step comes from the high queue, except that after `high-steps`
//!   high steps in a row a ready normal job gets its turn. Within a queue,
//!   jobs take turns.
//!
//! The core scans its jobs rather than keeping a heap of deadlines: there are
//! tens of them, not thousands, and a scan cannot go out of step with the jobs
//! the way a heap with stale entries can. A heap can come if the count ever
//! says so.
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
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

/// SPAN after NOW, but no further than [`FURTHEST`].
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

/// A job as the core sees it: what `work-list` will show.
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

    /// How busy the last second was, in percent, and with which jobs, busiest
    /// first -- at most once a second, `None` in between.
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

/// The core of a scheduler. Driven by one thread at a time; see the module
/// documentation.
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
            closed: false,
        }
    }

    /// Take JOB, under the name in SPEC.
    ///
    /// A job that already has the name leaves, as [`Exit::Cancelled`], on the
    /// next turn. `None` once the core has shut down: the job is not taken, and
    /// its `exit` is not called -- the caller knows it was refused.
    pub fn submit(&mut self, spec: Spec, job: Box<dyn Job<C>>) -> Option<WorkId> {
        let id = WorkId::next();
        self.submit_as(id, spec, job).then_some(id)
    }

    /// [`Self::submit`], under a number the caller has already handed out.
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

    /// Stop the job ID. It leaves on the next turn, as [`Exit::Cancelled`],
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

    /// Stop the job called NAME. False when there is none.
    pub fn cancel_named(&mut self, name: &str) -> bool {
        match self.names.get(name) {
            Some(&id) => self.cancel(id),
            None => false,
        }
    }

    /// Every job leaves, as [`Exit::Shutdown`], and nothing is taken after.
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

    /// One turn of an executor's loop: the jobs that are leaving leave, the
    /// deadlines that passed and the events that matured make jobs ready, and
    /// at most one step runs. True when a step ran.
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

    /// Turns until no job is ready, or until LIMIT steps have run: a job that
    /// always answers `Continue` would otherwise never let this return. The
    /// number of steps that ran.
    pub fn run_until_idle(&mut self, state: &C, limit: usize) -> usize {
        let mut steps = 0;
        while steps < limit && self.run_once(state) {
            steps += 1;
        }
        steps
    }

    /// Run SPAN of scheduling on CLOCK -- the clock this core reads -- moving
    /// it from one wake-up to the next instead of waiting: what the tests, and
    /// later the manual mode, use. LIMIT bounds the steps, as in
    /// [`Self::run_until_idle`].
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

    /// When the next turn has something to do: now when a job is ready or
    /// leaving, otherwise the earliest deadline, maturing event or summary.
    /// `None` when only a new job or an event can bring work.
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

    /// The report lines written since the last call, oldest first.
    pub fn take_reports(&mut self) -> Vec<String> {
        self.reporter.take()
    }

    /// The jobs that have left since the last call, their `exit` already
    /// called: what a list of the jobs kept outside the core drops.
    pub fn take_departed(&mut self) -> Vec<WorkId> {
        std::mem::take(&mut self.departed)
    }

    /// Every job the core holds, in the order they were submitted.
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

    /// Make ready every job whose deadline passed or whose events matured.
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

    /// The rule: high first, but after `high-steps` high steps in a row a
    /// ready normal job goes. Counted in steps, not time, so that it is
    /// deterministic.
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

    /// Take one step of ID, measure it, and apply its answer.
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
            worked: None,
        };
        let result = catch_unwind(AssertUnwindSafe(|| job.step(&mut cx)));
        let worked = cx.worked.take();
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
            Ok(Ok(step)) => self.apply(id, job, step, ended),
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

    /// Record a step of job ID, called NAME, that took TOOK; report it if it
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
            let what = match worked {
                Some(Worked { units, on }) if units > 0 => format!(
                    " on {on} ({units} {unit}, {} ms each)",
                    took.as_millis() / u128::from(units)
                ),
                Some(Worked { on, .. }) => format!(" on {on}"),
                None => String::new(),
            };
            let line = format!("[BKG] long step: {name}, {} ms{what}", took.as_millis());
            self.reporter
                .report(name, Rule::LongStep, took.as_millis() as u64, line, started);
        }
    }

    /// Put job ID back where its answer STEP says.
    fn apply(&mut self, id: WorkId, job: Box<dyn Job<C>>, step: Step, now: Instant) {
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
                record.event_due = record.has_unseen_events().then(|| now + delay);
            }
            Step::Done => self.leave(id, job, Exit::Finished),
        }
    }

    /// Job ID leaves, for WHY. Its `exit` comes with the next state.
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

    /// Call `exit` on every job that left.
    fn flush_leaving(&mut self, state: &C) {
        for Leaving {
            id,
            name,
            mut job,
            why,
        } in std::mem::take(&mut self.leaving)
        {
            // Guarded like a step: an `exit` that panics must not take the
            // scheduler down with it.
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| job.exit(state, why))) {
                self.reporter.always(format!(
                    "[BKG] panic: {name} panicked in exit: {}",
                    panic_message(&*payload)
                ));
            }
            self.departed.push(id);
        }
    }

    /// Close what is due to close: the second of load, and the report
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

/// What a panic said, when it said it with a string.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_string())
}
