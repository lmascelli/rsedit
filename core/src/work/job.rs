//! What a job is, and what it says back.
//!
//! Everything the scheduler knows about a job comes through here: a step, the
//! answer to it, and the one call made when the job leaves. See
//! `doc/workmanager.typ`, "I lavori".
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Work the scheduler runs a step at a time.
///
/// `C` is what every step is handed. In the editor it will be the editor's
/// state; in the scheduler's own tests it is `()`. Being generic over it is
/// what lets the core be tested without building an editor.
pub trait Job<C>: Send + 'static {
    /// Do a bounded amount of work, and say how to go on.
    ///
    /// A step never waits: no I/O that can block, no sleeping, no lock held
    /// into the next step. Whoever has to wait says so in the answer, and the
    /// scheduler wakes it. Nor does a step look at the clock to decide how much
    /// to do: it works in the units of its own data (lines, files, fuel) and
    /// says how many with [`StepContext::worked`]; the time is the scheduler's
    /// to measure.
    fn step(&mut self, cx: &mut StepContext<'_, C>) -> Result<Step, JobError>;

    /// Called exactly once, when the job leaves the scheduler, whatever the
    /// reason -- including a panic in a step, and including a job that was
    /// cancelled before it ever ran.
    ///
    /// The place for clean-up that would otherwise be spread over the branches
    /// of `step`: a counter to give back, a callback to queue.
    fn exit(&mut self, _state: &C, _why: Exit) {}
}

/// How a step says to go on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// There is more to do now: back to the end of the queue. If nothing else
    /// is ready, the next step follows at once.
    Continue,
    /// Wake me after this long. Events are ignored meanwhile -- they are not
    /// lost, the next step sees the state they announced.
    Sleep(Duration),
    /// Wake me when one of my topics changes.
    Wait,
    /// Wake me when one of my topics changes, or after this long at most.
    WaitAtMost(Duration),
    /// Finished.
    Done,
}

/// Why a job left. Handed to [`Job::exit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// A step answered [`Step::Done`].
    Finished,
    /// A step returned an error -- or answered [`Step::Wait`] with no topic
    /// that could ever wake it.
    Failed,
    /// A step panicked.
    Panicked,
    /// Somebody stopped it, or a job submitted under the same name replaced
    /// it.
    Cancelled,
    /// The scheduler is shutting down.
    Shutdown,
}

/// Why a step is happening.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// The job's first step.
    Start,
    /// The previous step answered [`Step::Continue`].
    Turn,
    /// One of the job's topics changed.
    Event,
    /// The job's deadline passed.
    Timer,
}

/// A step that failed. The scheduler logs it under the job's name, and the
/// job leaves with [`Exit::Failed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobError(pub String);

/// What a step is handed.
pub struct StepContext<'a, C> {
    /// The state the job works on.
    pub state: &'a C,
    /// Why this step is happening.
    pub woken_by: Wake,
    pub(crate) worked: Option<Worked>,
}

/// How much a step did, and on what. See [`StepContext::worked`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Worked {
    pub(crate) units: u64,
    pub(crate) on: String,
}

impl<C> StepContext<'_, C> {
    /// Say how much this step did, in the job's own unit, and on what.
    ///
    /// The deterministic half of a measurement. The scheduler times the step,
    /// and the two together give a cost per unit -- milliseconds per line --
    /// which is what tells a slow job from a large input. Optional: a job that
    /// never says gets only its time measured.
    pub fn worked(&mut self, units: u64, on: &str) {
        self.worked = Some(Worked {
            units,
            on: on.to_string(),
        });
    }
}

/// Which queue a job waits in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// What the user is waiting to see on the screen: colouring, the
    /// prescanner.
    High,
    /// Everything else. The default.
    Normal,
}

/// When a job takes its first step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// As soon as it is its turn. The default.
    Now,
    /// After this long.
    After(Duration),
}

/// The number of a job. Never reused, so a number held after its job has gone
/// can never name a different one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkId(u64);

impl WorkId {
    /// The next number. Process-wide rather than per scheduler, so that work
    /// spread over several executors still has one numbering.
    pub(crate) fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        WorkId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A job's name and how it is scheduled: everything about it except its code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub(crate) name: String,
    pub(crate) class: Class,
    pub(crate) unit: String,
    pub(crate) topics: Vec<String>,
    pub(crate) start: Start,
}

impl Spec {
    /// A job called NAME: normal class, started at once, no topics.
    ///
    /// Names are unique. Submitting a job under a name that is taken replaces
    /// the job that has it.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            class: Class::Normal,
            unit: "units".to_string(),
            topics: Vec::new(),
            start: Start::Now,
        }
    }

    /// Which queue it waits in.
    pub fn class(mut self, class: Class) -> Self {
        self.class = class;
        self
    }

    /// What [`StepContext::worked`] counts in, as reports should print it:
    /// `"lines"`, `"files"`.
    pub fn unit(mut self, unit: &str) -> Self {
        self.unit = unit.to_string();
        self
    }

    /// Subscribe to TOPIC: a change to it wakes the job when it is waiting.
    pub fn on(mut self, topic: &str) -> Self {
        self.topics.push(topic.to_string());
        self
    }

    /// When it takes its first step.
    pub fn start(mut self, start: Start) -> Self {
        self.start = start;
        self
    }
}
