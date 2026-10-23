//! What the WorkManager's tests share. Most use the manual mode, as the
//! editor's own tests do; the few that need real threads wait for a condition
//! with a deadline -- never for a fixed time -- and shut the WorkManager down
//! even when they fail, which is enough to leave no thread behind.
//!
//! Design: `doc/architettura-editor.typ, <prove-sfondo>`.
use crate::work::{Exit, Job, JobError, Step, StepContext, Stop, Wake, WorkManager};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

pub(crate) fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// How long a test waits for what should take a moment. Only a failing test
/// waits this long.
pub(crate) const PATIENCE: Duration = Duration::from_secs(10);

/// Waits until COND holds, failing after PATIENCE with WHAT.
pub(crate) fn eventually(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !cond() {
        assert!(Instant::now() < deadline, "still waiting for {what}");
        std::thread::sleep(ms(1));
    }
}

/// The report lines, as a test reads them.
#[derive(Clone, Default)]
pub(crate) struct Lines(Arc<Mutex<Vec<String>>>);

impl Lines {
    fn sink(&self) -> impl Fn(&str) + Send + Sync + 'static {
        let lines = self.0.clone();
        move |line: &str| lines.lock().unwrap().push(line.to_string())
    }

    pub(crate) fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    pub(crate) fn contain(&self, needle: &str) -> bool {
        self.all().iter().any(|line| line.contains(needle))
    }
}

pub(crate) fn manual() -> (WorkManager<()>, Lines) {
    let lines = Lines::default();
    (WorkManager::manual(lines.sink()), lines)
}

/// A threaded WorkManager, started, and shut down when this drops -- also when
/// the test fails, so a failure leaves no threads behind.
pub(crate) struct Threaded {
    pub(crate) work: WorkManager<()>,
    pub(crate) lines: Lines,
}

impl Drop for Threaded {
    fn drop(&mut self) {
        self.work.shutdown(&(), PATIENCE);
    }
}

pub(crate) fn threaded() -> Threaded {
    let lines = Lines::default();
    let work = WorkManager::threaded(lines.sink());
    work.start(());
    Threaded { work, lines }
}

/// What a scripted job lived through.
#[derive(Default)]
pub(crate) struct Trace {
    wakes: Vec<Wake>,
    exits: Vec<Exit>,
}

pub(crate) type Traced = Arc<Mutex<Trace>>;

pub(crate) fn wakes(trace: &Traced) -> Vec<Wake> {
    trace.lock().unwrap().wakes.clone()
}

pub(crate) fn exits(trace: &Traced) -> Vec<Exit> {
    trace.lock().unwrap().exits.clone()
}

/// A job that answers a list, then `Done`, whatever state it is given.
pub(crate) struct Scripted {
    answers: VecDeque<Step>,
    trace: Traced,
}

pub(crate) fn scripted(answers: &[Step]) -> (Box<Scripted>, Traced) {
    let trace = Traced::default();
    let job = Scripted {
        answers: answers.iter().copied().collect(),
        trace: trace.clone(),
    };
    (Box::new(job), trace)
}

impl<C: 'static> Job<C> for Scripted {
    fn step(&mut self, cx: &mut StepContext<'_, C>) -> Result<Step, JobError> {
        self.trace.lock().unwrap().wakes.push(cx.woken_by);
        Ok(self.answers.pop_front().unwrap_or(Step::Done))
    }

    fn exit(&mut self, _state: &C, why: Exit) {
        self.trace.lock().unwrap().exits.push(why);
    }
}

/// A job whose every step is F.
pub(crate) struct Steps<F>(pub(crate) F);

impl<C: 'static, F: FnMut() -> Step + Send + 'static> Job<C> for Steps<F> {
    fn step(&mut self, _cx: &mut StepContext<'_, C>) -> Result<Step, JobError> {
        Ok((self.0)())
    }
}

/// A dedicated thread's body that waits until it is asked to stop, the way a
/// shell command's does: its stop action ends the receive, as killing the
/// child ends the read.
pub(crate) fn blocked_until_stopped(stop: Stop) {
    let (unblock, blocked) = mpsc::channel::<()>();
    stop.on_cancel(move || drop(unblock));
    let _ = blocked.recv();
}

/// A job whose one step -- or whose `exit`, for `in_exit` -- does not end
/// until the test lets it go: the step the watchdog is there for. It says when
/// it has begun.
pub(crate) struct Held {
    in_exit: bool,
    begun: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

impl Held {
    fn wait(&self) {
        let _ = self.begun.send(());
        let _ = self.release.recv();
    }
}

impl<C: 'static> Job<C> for Held {
    fn step(&mut self, _cx: &mut StepContext<'_, C>) -> Result<Step, JobError> {
        if !self.in_exit {
            self.wait();
        }
        Ok(Step::Done)
    }

    fn exit(&mut self, _state: &C, _why: Exit) {
        if self.in_exit {
            self.wait();
        }
    }
}

/// Returns a [`Held`] job, what says it has begun, and what lets it go.
pub(crate) fn held(in_exit: bool) -> (Box<Held>, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (begun, has_begun) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let job = Held {
        in_exit,
        begun,
        release: released,
    };
    (Box::new(job), has_begun, release)
}

/// Drives WORK until it is idle, on a thread of its own: in manual mode the
/// driver is the executor, so a step that does not end has to be taken
/// somewhere other than where the test watches it from.
pub(crate) fn drive_elsewhere(work: &WorkManager<()>) -> std::thread::JoinHandle<usize> {
    let work = work.clone();
    std::thread::spawn(move || work.run_until_idle(&()))
}

/// Returns the watchdog's reports of a step that does not end, so far.
pub(crate) fn stuck(lines: &Lines) -> Vec<String> {
    lines
        .all()
        .into_iter()
        .filter(|line| line.starts_with("[BKG] stuck step:"))
        .collect()
}

/// Returns how many threads of this process have a name starting with PREFIX.
#[cfg(target_os = "linux")]
pub(crate) fn threads_named(prefix: &str) -> usize {
    std::fs::read_dir("/proc/self/task")
        .expect("the threads of this process")
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .filter(|name| name.starts_with(prefix))
        .count()
}
