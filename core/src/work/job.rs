//! What a job is, and what it says back: a step, the answer to it, and the
//! one call made when the job leaves. Everything the scheduler knows about a
//! job comes through here.
//!
//! Design: `doc/architettura-editor.typ, <lavori>`.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Work the scheduler runs a step at a time.
///
/// `C` is what every step is handed: the editor's state in the editor, and
/// `()` in the scheduler's own tests, which therefore need no editor.
pub trait Job<C>: Send + 'static {
    /// Does a bounded amount of work, and says how to go on.
    ///
    /// A step never waits: no I/O that can block, no sleeping, no lock held
    /// into the next step. Whoever has to wait says so in the answer, and the
    /// scheduler wakes it. Nor does a step look at the clock to decide how much
    /// to do: it works in the units of its own data (lines, files, fuel) and
    /// says how many with [`StepContext::worked`]; the time is the scheduler's
    /// to measure.
    fn step(&mut self, cx: &mut StepContext<'_, C>) -> Result<Step, JobError>;

    /// Ends the job: called exactly once, when it leaves the scheduler,
    /// whatever the reason -- a panic in a step included, and a cancel before
    /// it ever ran.
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
    /// The job's own number: what it was given when it was submitted.
    pub id: WorkId,
    pub(crate) worked: Option<Worked>,
    pub(crate) topics: Option<Vec<String>>,
}

/// How much a step did, and on what. See [`StepContext::worked`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Worked {
    pub(crate) units: u64,
    pub(crate) on: String,
}

impl<C> StepContext<'_, C> {
    /// Records how much this step did, in the job's own unit, and on what.
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

    /// Has the job woken by TOPICS from the end of this step, instead of the
    /// topics it had: those it was submitted with, or those an earlier step
    /// named here. They hold until a later step names others. What a Lisp
    /// `(yield '(wait buffer-changed))` comes down to.
    ///
    /// A topic the job was not already subscribed to counts as changed, and a
    /// waiting job is woken for it `event-delay` later: the step that named it
    /// may have looked at the state just before an emission. Naming the same
    /// topics as before costs nothing.
    pub fn subscribe<S: Into<String>>(&mut self, topics: impl IntoIterator<Item = S>) {
        self.topics = Some(topics.into_iter().map(Into::into).collect());
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

/// The number of a job or of a dedicated thread. Never reused, so a number
/// held after its work has gone can never name different work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkId(u64);

impl WorkId {
    /// Returns the next number. Process-wide rather than per WorkManager: a
    /// number names one piece of work in the whole process.
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
    /// Returns the spec of a job called NAME: normal class, started at once,
    /// no topics.
    ///
    /// Names are unique: submitting a job under a name that is taken replaces
    /// the work that has it.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            class: Class::Normal,
            unit: "units".to_string(),
            topics: Vec::new(),
            start: Start::Now,
        }
    }

    /// Sets the queue the job waits in.
    pub fn class(mut self, class: Class) -> Self {
        self.class = class;
        self
    }

    /// Sets what [`StepContext::worked`] counts in, as reports print it:
    /// `"lines"`, `"files"`.
    pub fn unit(mut self, unit: &str) -> Self {
        self.unit = unit.to_string();
        self
    }

    /// Subscribes the job to TOPIC: a change to it wakes the job when it is
    /// waiting.
    pub fn on(mut self, topic: &str) -> Self {
        self.topics.push(topic.to_string());
        self
    }

    /// Sets when the job takes its first step.
    pub fn start(mut self, start: Start) -> Self {
        self.start = start;
        self
    }
}
