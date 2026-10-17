//! The WorkManager: the core with an executor around it, the registry, the
//! dedicated threads and shutdown.
//!
//! Most tests use the manual mode, as the editor's own tests do: nothing runs
//! unless the test says so, on a clock that moves when it is told to. The few
//! that need real threads wait for a condition with a deadline -- never for a
//! fixed time -- and shut the WorkManager down even when they fail, which the
//! leak test shows is enough to leave no thread behind. See
//! `doc/workmanager.typ`, "Come si prova".
#[cfg(test)]
mod tests {
    use crate::work::{
        Class, Exit, Job, JobError, Place, Spec, Start, Step, StepContext, Stop, Wake, WorkInfo,
        WorkManager,
    };
    use crate::{GapBuffer, create_global_env};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// How long a test waits for what should take a moment. Only a failing
    /// test waits this long.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// Wait until COND holds, failing after PATIENCE with WHAT.
    fn eventually(what: &str, cond: impl Fn() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !cond() {
            assert!(Instant::now() < deadline, "still waiting for {what}");
            std::thread::sleep(ms(1));
        }
    }

    /// The report lines, as a test reads them.
    #[derive(Clone, Default)]
    struct Lines(Arc<Mutex<Vec<String>>>);

    impl Lines {
        fn sink(&self) -> impl Fn(&str) + Send + Sync + 'static {
            let lines = self.0.clone();
            move |line: &str| lines.lock().unwrap().push(line.to_string())
        }

        fn all(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }

        fn contain(&self, needle: &str) -> bool {
            self.all().iter().any(|line| line.contains(needle))
        }
    }

    fn manual() -> (WorkManager<()>, Lines) {
        let lines = Lines::default();
        (WorkManager::manual(lines.sink()), lines)
    }

    /// A threaded WorkManager, started, and shut down when this drops -- also
    /// when the test fails, so a failure leaves no threads behind.
    struct Threaded {
        work: WorkManager<()>,
        lines: Lines,
    }

    impl Drop for Threaded {
        fn drop(&mut self) {
            self.work.shutdown(&(), PATIENCE);
        }
    }

    fn threaded() -> Threaded {
        let lines = Lines::default();
        let work = WorkManager::threaded(lines.sink());
        work.start(());
        Threaded { work, lines }
    }

    /// What a scripted job lived through.
    #[derive(Default)]
    struct Trace {
        wakes: Vec<Wake>,
        exits: Vec<Exit>,
    }

    type Traced = Arc<Mutex<Trace>>;

    fn wakes(trace: &Traced) -> Vec<Wake> {
        trace.lock().unwrap().wakes.clone()
    }

    fn exits(trace: &Traced) -> Vec<Exit> {
        trace.lock().unwrap().exits.clone()
    }

    /// A job that answers a list, then `Done`, whatever state it is given.
    struct Scripted {
        answers: VecDeque<Step>,
        trace: Traced,
    }

    fn scripted(answers: &[Step]) -> (Box<Scripted>, Traced) {
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
    struct Steps<F>(F);

    impl<C: 'static, F: FnMut() -> Step + Send + 'static> Job<C> for Steps<F> {
        fn step(&mut self, _cx: &mut StepContext<'_, C>) -> Result<Step, JobError> {
            Ok((self.0)())
        }
    }

    /// A dedicated thread's body that waits until it is asked to stop, the way
    /// a shell command's does: its stop action ends the receive, as killing
    /// the child ends the read.
    fn blocked_until_stopped(stop: Stop) {
        let (unblock, blocked) = mpsc::channel::<()>();
        stop.on_cancel(move || drop(unblock));
        let _ = blocked.recv();
    }

    /// A job whose one step -- or whose `exit`, for `in_exit` -- does not end
    /// until the test lets it go: the step the watchdog is there for. It says
    /// when it has begun.
    struct Held {
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

    /// A [`Held`] job, what says it has begun, and what lets it go.
    fn held(in_exit: bool) -> (Box<Held>, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (begun, has_begun) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let job = Held {
            in_exit,
            begun,
            release: released,
        };
        (Box::new(job), has_begun, release)
    }

    /// Drive WORK until it is idle, on a thread of its own: in manual mode
    /// the driver is the executor, so a step that does not end has to be
    /// taken somewhere other than where the test watches it from.
    fn drive_elsewhere(work: &WorkManager<()>) -> std::thread::JoinHandle<usize> {
        let work = work.clone();
        std::thread::spawn(move || work.run_until_idle(&()))
    }

    /// The watchdog's reports of a step that does not end, so far.
    fn stuck(lines: &Lines) -> Vec<String> {
        lines
            .all()
            .into_iter()
            .filter(|line| line.starts_with("[BKG] stuck step:"))
            .collect()
    }

    // -- the manual mode ------------------------------------------------------

    #[test]
    fn in_manual_mode_nothing_runs_until_the_test_drives_it() {
        let (work, _) = manual();
        let (job, trace) = scripted(&[Step::Continue, Step::Continue]);
        work.submit(Spec::new("counter"), job).expect("taken");
        assert!(wakes(&trace).is_empty(), "submitting runs nothing");

        assert_eq!(work.run_until_idle(&()), 3);
        assert_eq!(wakes(&trace), [Wake::Start, Wake::Turn, Wake::Turn]);
        assert_eq!(exits(&trace), [Exit::Finished]);
        assert!(work.jobs().is_empty(), "a job that finished is not listed");
    }

    #[test]
    fn the_registry_answers_at_once_and_the_executor_fills_it_in() {
        let (work, _) = manual();
        let (waiter, _) = scripted(&[Step::Wait]);
        let id = work
            .submit(Spec::new("waiter").class(Class::High).on("t"), waiter)
            .unwrap();
        let (later, _) = scripted(&[]);
        work.submit(Spec::new("later").start(Start::After(ms(50))), later)
            .unwrap();

        // Before the executor has seen either.
        let listed = work.jobs();
        assert_eq!(listed.len(), 2);
        let as_submitted = WorkInfo {
            id,
            name: "waiter".to_string(),
            class: Class::High,
            place: Place::Ready,
            steps: 0,
            total: Duration::ZERO,
            longest: Duration::ZERO,
        };
        assert_eq!(listed[0], as_submitted);
        assert_eq!(listed[1].place, Place::Sleeping, "a delayed start sleeps");

        work.run_until_idle(&());
        let listed = work.jobs();
        assert_eq!((listed[0].place, listed[0].steps), (Place::Waiting, 1));
    }

    #[test]
    fn a_burst_of_emissions_wakes_a_waiting_job_once() {
        // The example in the design document.
        let (work, _) = manual();
        let (probe, trace) = scripted(&[Step::Wait, Step::Wait]);
        work.submit(Spec::new("probe").on("buffer-changed"), probe)
            .unwrap();
        work.run_until_idle(&());

        let changed = work.topic("buffer-changed");
        for _ in 0..3 {
            changed.emit();
            work.advance(ms(5)); // three keystrokes in 15 ms
        }
        work.run_for(&(), ms(20)); // the event delay runs out
        assert_eq!(wakes(&trace), [Wake::Start, Wake::Event]);
    }

    #[test]
    fn work_submitted_from_a_step_runs_in_the_same_drive() {
        // The test's thread is the executor here, and a step submitting work
        // must not wait for it -- nor be left out of the run.
        let (work, _) = manual();
        let (child, trace) = scripted(&[]);
        let mut child = Some(child);
        let inner = work.clone();
        let parent = Steps(move || {
            if let Some(child) = child.take() {
                inner
                    .submit(Spec::new("child"), child)
                    .expect("taken from inside a step");
            }
            Step::Done
        });
        work.submit(Spec::new("parent"), Box::new(parent)).unwrap();

        assert_eq!(
            work.run_until_idle(&()),
            2,
            "the parent's step, then the child's"
        );
        assert_eq!(exits(&trace), [Exit::Finished]);
        work.shutdown(&(), PATIENCE);
    }

    #[test]
    fn a_name_is_one_job_and_a_new_one_replaces_the_old() {
        let (work, _) = manual();
        let (old, old_trace) = scripted(&[Step::Wait]);
        let first = work.submit(Spec::new("x").on("t"), old).unwrap();
        work.run_until_idle(&());

        let (new, new_trace) = scripted(&[Step::Wait]);
        let second = work.submit(Spec::new("x").on("t"), new).unwrap();
        assert_ne!(first, second, "a number is never reused");
        let ids: Vec<_> = work.jobs().iter().map(|job| job.id).collect();
        assert_eq!(ids, [second], "the registry knows at once");

        work.run_until_idle(&());
        assert_eq!(exits(&old_trace), [Exit::Cancelled]);
        assert_eq!(wakes(&new_trace), [Wake::Start]);
    }

    #[test]
    fn cancel_answers_whether_there_was_work_to_stop() {
        let (work, _) = manual();
        let (ran, ran_trace) = scripted(&[Step::Wait]);
        work.submit(Spec::new("ran").on("t"), ran).unwrap();
        work.run_until_idle(&());
        let (unrun, unrun_trace) = scripted(&[]);
        let unrun = work.submit(Spec::new("unrun"), unrun).unwrap();

        // One the executor has not even seen yet: its message is still in the
        // inbox, behind the one that brings it.
        assert!(work.cancel(unrun));
        assert!(!work.cancel(unrun), "only once");
        assert!(work.cancel_named("ran"));
        assert!(!work.cancel_named("ran"));
        assert!(!work.cancel_named("nobody"));

        work.run_until_idle(&());
        assert_eq!(exits(&ran_trace), [Exit::Cancelled]);
        assert_eq!(exits(&unrun_trace), [Exit::Cancelled]);
        assert!(wakes(&unrun_trace).is_empty(), "no step after a cancel");
        assert!(work.jobs().is_empty());
    }

    #[test]
    fn the_cores_reports_go_to_the_log() {
        let (work, lines) = manual();
        let clock = work.clone();
        let slow = Steps(move || {
            clock.advance(ms(100));
            Step::Done
        });
        work.submit(Spec::new("slow"), Box::new(slow)).unwrap();
        work.run_until_idle(&());
        assert!(
            lines.contain("[BKG] long step: slow, 100 ms"),
            "{:?}",
            lines.all()
        );
        work.shutdown(&(), PATIENCE);
    }

    #[test]
    fn a_step_that_panics_is_logged_and_its_job_leaves() {
        let (work, lines) = manual();
        let boom = Steps(|| -> Step { panic!("kaboom") });
        work.submit(Spec::new("boom"), Box::new(boom)).unwrap();
        work.run_until_idle(&());
        assert!(
            lines.contain("[BKG] panic: boom retired: kaboom"),
            "{:?}",
            lines.all()
        );
        assert!(work.jobs().is_empty());
    }

    #[test]
    fn shutdown_ends_every_job_and_refuses_what_comes_after() {
        let (work, _) = manual();
        let (waiter, waiter_trace) = scripted(&[Step::Wait]);
        work.submit(Spec::new("waiter").on("t"), waiter).unwrap();
        work.run_until_idle(&());
        let (queued, queued_trace) = scripted(&[]);
        work.submit(Spec::new("queued"), queued).unwrap();

        work.shutdown(&(), PATIENCE);
        assert_eq!(exits(&waiter_trace), [Exit::Shutdown]);
        assert_eq!(
            exits(&queued_trace),
            [Exit::Shutdown],
            "a job still in the inbox is owed its exit too"
        );
        assert!(wakes(&queued_trace).is_empty());
        assert!(work.jobs().is_empty());

        let (late, _) = scripted(&[]);
        assert_eq!(work.submit(Spec::new("late"), late), None);
        assert_eq!(work.spawn("late thread", |_, _| {}), None);
        assert!(
            !work.cancel_named("waiter"),
            "everything has stopped already"
        );
        assert_eq!(work.run_until_idle(&()), 0);
        work.shutdown(&(), PATIENCE); // a second call does nothing
    }

    #[test]
    #[should_panic(expected = "only a manual WorkManager is driven by hand")]
    fn a_threaded_manager_is_not_driven_by_hand() {
        let threaded = threaded();
        threaded.work.run_until_idle(&());
    }

    // -- the executor thread --------------------------------------------------

    #[test]
    fn a_threaded_manager_runs_work_by_itself() {
        let t = threaded();
        let (job, trace) = scripted(&[Step::Sleep(ms(1))]);
        t.work.submit(Spec::new("sleeper"), job).unwrap();
        eventually("the job to finish", || exits(&trace) == [Exit::Finished]);
        assert_eq!(wakes(&trace), [Wake::Start, Wake::Timer]);
        eventually("the registry to drop it", || t.work.jobs().is_empty());
    }

    #[test]
    fn an_emission_wakes_the_sleeping_executor() {
        let t = threaded();
        let (job, trace) = scripted(&[Step::Wait, Step::Wait]);
        t.work.submit(Spec::new("waiter").on("t"), job).unwrap();
        eventually("the first step", || wakes(&trace).len() == 1);
        // No deadline anywhere now: only the emission's nudge can wake the
        // executor.
        t.work.topic("t").emit();
        eventually("the event", || wakes(&trace).len() == 2);
        assert_eq!(wakes(&trace), [Wake::Start, Wake::Event]);
    }

    #[test]
    fn threaded_shutdown_returns_once_the_jobs_have_left() {
        let t = threaded();
        let (job, trace) = scripted(&[Step::Wait]);
        t.work.submit(Spec::new("waiter").on("t"), job).unwrap();
        eventually("the first step", || wakes(&trace).len() == 1);

        // Any span will do: the wait ends when the executor does.
        t.work.shutdown(&(), Duration::MAX);
        assert_eq!(exits(&trace), [Exit::Shutdown]);
        let (late, _) = scripted(&[]);
        assert_eq!(t.work.submit(Spec::new("late"), late), None);
    }

    #[test]
    fn the_latest_under_a_name_outlives_its_work_until_something_takes_the_name() {
        // What a queued callback is checked against: a job that finished
        // still speaks for its name, one replaced or stopped does not.
        let (work, _) = manual();
        let (job, _) = scripted(&[]);
        let first = work.submit(Spec::new("index"), job).unwrap();
        assert!(work.is_latest("index", first));
        work.run_until_idle(&());
        assert!(work.jobs().is_empty(), "it has finished");
        assert!(
            work.is_latest("index", first),
            "and still speaks for the name"
        );

        let (job, _) = scripted(&[Step::Sleep(ms(10))]);
        let second = work.submit(Spec::new("index"), job).unwrap();
        assert!(!work.is_latest("index", first), "a newer one took the name");
        assert!(work.is_latest("index", second));
        assert!(work.cancel_named("index"));
        assert!(
            !work.is_latest("index", second),
            "and a stopped one is silent"
        );
        assert!(!work.is_latest("never", second), "a name is its own");
    }

    // -- the hook after a step -----------------------------------------------

    #[test]
    fn the_hook_is_called_after_every_step_and_only_then() {
        let (work, _) = manual();
        let steps = Arc::new(Mutex::new(0));
        assert!(work.after_step({
            let steps = steps.clone();
            move || *steps.lock().unwrap() += 1
        }));
        assert!(!work.after_step(|| {}), "set once: the second is refused");
        let (job, _) = scripted(&[Step::Continue, Step::Sleep(ms(10))]);
        work.submit(Spec::new("three"), job).unwrap();

        work.run_until_idle(&());
        assert_eq!(*steps.lock().unwrap(), 2, "two steps so far, two calls");
        work.run_for(&(), ms(100));
        assert_eq!(*steps.lock().unwrap(), 3);
        work.run_for(&(), ms(100));
        assert_eq!(*steps.lock().unwrap(), 3, "no step, no call");
    }

    #[test]
    fn the_hook_finds_a_finished_job_gone_from_the_registry() {
        // Whoever the hook wakes looks at the state: a job that has just
        // finished must not still be listed.
        let (work, _) = manual();
        let listed = Arc::new(Mutex::new(Vec::new()));
        work.after_step({
            let (work, listed) = (work.clone(), listed.clone());
            move || listed.lock().unwrap().push(work.jobs().len())
        });
        let (job, _) = scripted(&[]);
        work.submit(Spec::new("brief"), job).unwrap();
        work.run_until_idle(&());
        assert_eq!(*listed.lock().unwrap(), [0]);
    }

    #[test]
    fn a_hook_that_panics_is_logged_and_the_work_goes_on() {
        let (work, lines) = manual();
        work.after_step(|| panic!("the host fell over"));
        let (job, trace) = scripted(&[Step::Continue]);
        work.submit(Spec::new("survivor"), job).unwrap();
        work.run_until_idle(&());
        assert_eq!(exits(&trace), [Exit::Finished], "both steps ran");
        assert!(lines.contain("[BKG] panic: the after-step hook: the host fell over"));
    }

    #[test]
    fn on_a_threaded_manager_the_hook_runs_on_the_executor() {
        let t = threaded();
        let (tell, told) = mpsc::channel();
        t.work.after_step(move || {
            let _ = tell.send(std::thread::current().name().map(str::to_string));
        });
        let (job, _) = scripted(&[]);
        t.work.submit(Spec::new("brief"), job).unwrap();
        let name = told.recv_timeout(PATIENCE).expect("the hook was called");
        assert!(
            name.is_some_and(|name| name.ends_with(" executor")),
            "called from the executor's thread"
        );
    }

    // -- dedicated threads ----------------------------------------------------

    #[test]
    fn cancel_raises_the_stop_and_runs_the_action_that_unblocks() {
        let t = threaded();
        let saw_stop = Arc::new(AtomicBool::new(false));
        let saw = saw_stop.clone();
        let id = t
            .work
            .spawn("reader", move |stop: Stop, _| {
                blocked_until_stopped(stop.clone());
                saw.store(stop.is_cancelled(), Ordering::SeqCst);
            })
            .unwrap();
        let listed = t.work.threads();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].id, listed[0].name.as_str()), (id, "reader"));
        assert!(!listed[0].stopping);

        assert!(t.work.cancel(id));
        eventually("the thread to finish", || t.work.threads().is_empty());
        assert!(saw_stop.load(Ordering::SeqCst));
        assert!(!t.work.cancel(id), "nothing left to stop");
    }

    #[test]
    fn a_stop_action_runs_exactly_once_whenever_it_is_registered() {
        // Registered after the request: at once, by `on_cancel`.
        let stop = Stop::default();
        assert!(stop.raise().is_none(), "nothing registered yet");
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        stop.on_cancel(move || flag.store(true, Ordering::SeqCst));
        assert!(ran.load(Ordering::SeqCst));

        // Registered before: handed to whoever raises, once. The last one
        // registered is the one that counts.
        let stop = Stop::default();
        let (first, second) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        let flag = first.clone();
        stop.on_cancel(move || flag.store(true, Ordering::SeqCst));
        let flag = second.clone();
        stop.on_cancel(move || flag.store(true, Ordering::SeqCst));
        assert!(!stop.is_cancelled());
        (stop.raise().expect("the action"))();
        assert!(stop.is_cancelled());
        assert!(stop.raise().is_none(), "and only once");
        assert!(!first.load(Ordering::SeqCst) && second.load(Ordering::SeqCst));
    }

    #[test]
    fn a_stop_action_that_panics_is_logged() {
        let (work, lines) = manual();
        let (registered, ready) = mpsc::channel::<()>();
        let (release, held) = mpsc::channel::<()>();
        let id = work
            .spawn("clumsy", move |stop: Stop, _| {
                stop.on_cancel(|| panic!("bad action"));
                let _ = registered.send(());
                let _ = held.recv();
            })
            .unwrap();
        ready.recv_timeout(PATIENCE).expect("the action registered");

        assert!(work.cancel(id), "the panic stays in the action");
        assert!(
            lines.contain("[BKG] panic: thread clumsy: its stop action panicked: bad action"),
            "{:?}",
            lines.all()
        );
        let _ = release.send(());
        work.shutdown(&(), PATIENCE);
    }

    #[test]
    fn a_panic_in_a_dedicated_thread_reaches_the_log() {
        let t = threaded();
        t.work.options().set("watchdog-every", 5).unwrap();
        t.work.spawn("doomed", |_, _| panic!("kaboom")).unwrap();
        eventually("the watchdog to log the panic", || {
            t.lines.contain("[BKG] panic: thread doomed: kaboom")
        });
        assert!(t.work.threads().is_empty());
    }

    #[test]
    fn a_thread_that_will_not_stop_is_reported_then_abandoned() {
        let t = threaded();
        t.work.options().set("watchdog-every", 5).unwrap();
        t.work.options().set("stuck-step", 20).unwrap();
        let (release, held) = mpsc::channel::<()>();
        // It never looks at its stop, and registers nothing to unblock it.
        let id = t
            .work
            .spawn("stubborn", move |_, _| {
                let _ = held.recv();
            })
            .unwrap();
        assert!(t.work.cancel(id));
        assert!(t.work.threads()[0].stopping);
        eventually("the watchdog's report", || {
            t.lines.contain("[BKG] thread: stubborn still running")
        });

        t.work.shutdown(&(), ms(20));
        assert!(
            t.lines
                .contain("[BKG] thread: stubborn abandoned at shutdown"),
            "{:?}",
            t.lines.all()
        );
        // Let it go, so it does not outlive the test by much.
        let _ = release.send(());
    }

    #[test]
    fn a_thread_that_has_finished_is_not_there_to_stop() {
        // Listed until it is joined, but not stoppable: stopping it would
        // answer that it had been running.
        let t = threaded();
        let id = t.work.spawn("brief", |_, _| {}).unwrap();
        eventually("the thread to end", || t.work.threads().is_empty());
        assert!(!t.work.cancel(id));
        // Ended, and not yet joined by anybody: its body has returned, and
        // the thread is over a moment later.
        let token = Arc::new(());
        let held = token.clone();
        t.work.spawn("brief", move |_, _| drop(held)).unwrap();
        eventually("the body to return", || Arc::strong_count(&token) == 1);
        std::thread::sleep(ms(20));
        assert!(!t.work.cancel_named("brief"), "it had finished");
        assert!(!t.work.is_named("brief"));
    }

    #[test]
    fn a_name_is_taken_while_its_work_runs() {
        let (work, _) = manual();
        assert!(!work.is_named("one"));
        let (job, _) = scripted(&[Step::Sleep(ms(10))]);
        work.submit(Spec::new("one"), job).unwrap();
        assert!(work.is_named("one"));
        assert!(work.cancel_named("one"));
        assert!(!work.is_named("one"));
    }

    #[test]
    fn jobs_and_threads_share_one_namespace() {
        let (work, _) = manual();
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        work.spawn("x", move |stop: Stop, _| {
            blocked_until_stopped(stop);
            flag.store(true, Ordering::SeqCst);
        })
        .unwrap();

        // A job under the thread's name stops the thread...
        let (job, trace) = scripted(&[Step::Wait]);
        work.submit(Spec::new("x").on("t"), job).unwrap();
        eventually("the thread to stop", || stopped.load(Ordering::SeqCst));
        work.run_until_idle(&());
        assert_eq!(wakes(&trace), [Wake::Start]);

        // ...and a thread under the job's name stops the job.
        work.spawn("x", |_, _| {}).unwrap();
        work.run_until_idle(&());
        assert_eq!(exits(&trace), [Exit::Cancelled]);
        eventually("both threads to be reaped", || work.threads().is_empty());
        work.shutdown(&(), PATIENCE);
    }

    /// How many threads of this process have a name starting with PREFIX.
    // -- the watchdog ---------------------------------------------------------

    #[test]
    fn a_step_that_does_not_end_is_reported_and_again_each_time_it_doubles() {
        // The executor cannot report a step it is still in; the watchdog,
        // looking from outside, can. At `stuck-step`, then at twice and four
        // times that -- not at every look.
        let (work, lines) = manual();
        let (job, begun, release) = held(false);
        work.submit(Spec::new("stuck"), job).unwrap();
        let stepping = drive_elsewhere(&work);
        begun.recv_timeout(PATIENCE).expect("the step began");

        work.watch_once();
        work.advance(ms(999));
        work.watch_once();
        assert!(stuck(&lines).is_empty(), "not before stuck-step");
        work.advance(ms(1));
        work.watch_once();
        work.watch_once();
        assert_eq!(stuck(&lines), ["[BKG] stuck step: stuck running for 1.0 s"]);
        work.advance(ms(999));
        work.watch_once();
        work.advance(ms(1));
        work.watch_once();
        // A look that comes late -- past the 4 s mark and the 8 s one --
        // reports once, for as long as the step has run, and the next mark is
        // the first doubling past it: 16 s, not 8.
        work.advance(ms(7_000));
        work.watch_once();
        work.advance(ms(6_500));
        work.watch_once();
        work.advance(ms(500));
        work.watch_once();
        assert_eq!(
            stuck(&lines),
            [
                "[BKG] stuck step: stuck running for 1.0 s",
                "[BKG] stuck step: stuck running for 2.0 s",
                "[BKG] stuck step: stuck running for 9.0 s",
                "[BKG] stuck step: stuck running for 16.0 s",
            ]
        );

        release.send(()).unwrap();
        stepping.join().unwrap();
        work.advance(ms(60_000));
        work.watch_once();
        assert_eq!(
            stuck(&lines).len(),
            4,
            "a step that has ended is not watched"
        );
        assert!(lines.contain("[BKG] long step: stuck, 16000 ms"));
    }

    #[test]
    fn each_step_is_watched_from_its_own_start() {
        // The marks belong to one step. The next one -- here another job's,
        // the moment the first lets go -- starts again from stuck-step.
        let (work, lines) = manual();
        let (first, first_begun, release_first) = held(false);
        let (second, second_begun, release_second) = held(false);
        work.submit(Spec::new("first"), first).unwrap();
        work.submit(Spec::new("second"), second).unwrap();
        let stepping = drive_elsewhere(&work);
        first_begun
            .recv_timeout(PATIENCE)
            .expect("the first step began");
        work.advance(ms(1_500));
        work.watch_once();
        release_first.send(()).unwrap();
        second_begun
            .recv_timeout(PATIENCE)
            .expect("the second step began");
        work.watch_once();
        work.advance(ms(999));
        work.watch_once();
        assert_eq!(stuck(&lines), ["[BKG] stuck step: first running for 1.5 s"]);
        work.advance(ms(1));
        work.watch_once();
        assert_eq!(
            stuck(&lines),
            [
                "[BKG] stuck step: first running for 1.5 s",
                "[BKG] stuck step: second running for 1.0 s",
            ]
        );
        release_second.send(()).unwrap();
        stepping.join().unwrap();
    }

    #[test]
    fn an_exit_that_does_not_end_is_reported_as_one() {
        // An `exit` runs on the executor too, and holds it as surely.
        let (work, lines) = manual();
        let (job, begun, release) = held(true);
        work.submit(Spec::new("leaving"), job).unwrap();
        let stepping = drive_elsewhere(&work);
        begun.recv_timeout(PATIENCE).expect("the exit began");
        work.advance(ms(1_000));
        work.watch_once();
        assert_eq!(
            stuck(&lines),
            ["[BKG] stuck step: leaving running for 1.0 s in its exit"]
        );
        release.send(()).unwrap();
        stepping.join().unwrap();
    }

    #[test]
    fn the_job_in_a_step_is_listed_as_running() {
        // What the executor publishes it publishes between steps, so a step
        // that does not end would leave its job listed as it was before the
        // step. The list asks what is running instead.
        let (work, _) = manual();
        let (job, begun, release) = held(false);
        work.submit(Spec::new("stuck"), job).unwrap();
        let (waiter, _) = scripted(&[Step::Wait]);
        work.submit(Spec::new("waiter").on("t"), waiter).unwrap();
        let places = || -> Vec<(String, Place)> {
            work.jobs()
                .into_iter()
                .map(|job| (job.name, job.place))
                .collect()
        };
        let stepping = drive_elsewhere(&work);
        begun.recv_timeout(PATIENCE).expect("the step began");
        assert_eq!(
            places(),
            [
                ("stuck".to_string(), Place::Running),
                ("waiter".to_string(), Place::Ready)
            ]
        );
        release.send(()).unwrap();
        stepping.join().unwrap();
        assert_eq!(places(), [("waiter".to_string(), Place::Waiting)]);
    }

    #[test]
    fn the_hook_is_called_after_the_turn_has_reported() {
        // So that the editor's hook, which writes the log file, writes what
        // the turn has just said.
        let (work, lines) = manual();
        let saw = Arc::new(Mutex::new(Vec::new()));
        work.after_watch({
            let (saw, lines) = (saw.clone(), lines.clone());
            move || saw.lock().unwrap().push(stuck(&lines).len())
        });
        let (job, begun, release) = held(false);
        work.submit(Spec::new("stuck"), job).unwrap();
        let stepping = drive_elsewhere(&work);
        begun.recv_timeout(PATIENCE).expect("the step began");
        work.watch_once();
        work.advance(ms(1_000));
        work.watch_once();
        assert_eq!(*saw.lock().unwrap(), [0, 1]);
        release.send(()).unwrap();
        stepping.join().unwrap();
        work.shutdown(&(), PATIENCE);
        assert_eq!(saw.lock().unwrap().len(), 3, "and once more at shutdown");
    }

    #[test]
    fn a_watch_hook_that_panics_is_logged_and_the_watchdog_goes_on() {
        let (work, lines) = manual();
        work.after_watch(|| panic!("no disk"));
        work.watch_once();
        work.watch_once();
        let panics = lines
            .all()
            .into_iter()
            .filter(|line| line == "[BKG] panic: the after-watch hook: no disk")
            .count();
        assert_eq!(panics, 2);
    }

    #[test]
    fn the_watchdog_reports_a_step_that_does_not_end_by_itself() {
        let t = threaded();
        t.work.options().set("watchdog-every", 5).unwrap();
        t.work.options().set("stuck-step", 20).unwrap();
        let (job, begun, release) = held(false);
        t.work.submit(Spec::new("stuck"), job).unwrap();
        begun.recv_timeout(PATIENCE).expect("the step began");
        eventually("the watchdog's report", || {
            t.lines.contain("[BKG] stuck step: stuck running for")
        });
        eventually("its second report", || stuck(&t.lines).len() >= 2);
        release.send(()).unwrap();
    }

    #[test]
    fn on_a_threaded_manager_the_hook_runs_on_the_watchdog() {
        let t = threaded();
        t.work.options().set("watchdog-every", 5).unwrap();
        let names = Arc::new(Mutex::new(Vec::new()));
        // Set before any turn has had the time to call it: it is read at
        // each turn, not when the watchdog starts.
        t.work.after_watch({
            let names = names.clone();
            move || {
                let name = std::thread::current().name().unwrap_or("").to_string();
                names.lock().unwrap().push(name);
            }
        });
        eventually("a turn of the watchdog", || {
            !names.lock().unwrap().is_empty()
        });
        let expected = format!("w{} watchdog", t.work.instance());
        assert_eq!(names.lock().unwrap()[0], expected);
    }

    #[test]
    fn shutdown_names_the_step_it_leaves_behind() {
        let t = threaded();
        let (job, begun, release) = held(false);
        t.work.submit(Spec::new("stuck"), job).unwrap();
        begun.recv_timeout(PATIENCE).expect("the step began");
        t.work.shutdown(&(), ms(20));
        assert!(
            t.lines
                .contain("[BKG] executor abandoned at shutdown: stuck running for"),
            "{:?}",
            t.lines.all()
        );
        // Let it go, so that the executor does not outlive the test by much.
        let _ = release.send(());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_watchdog_starts_with_the_executor() {
        // It watches the executor's steps, so it cannot wait for the first
        // dedicated thread, as it once did.
        let t = threaded();
        let prefix = format!("w{} ", t.work.instance());
        eventually("the executor and the watchdog", || {
            threads_named(&prefix) == 2
        });
    }

    #[cfg(target_os = "linux")]
    fn threads_named(prefix: &str) -> usize {
        std::fs::read_dir("/proc/self/task")
            .expect("the threads of this process")
            .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
            .filter(|name| name.starts_with(prefix))
            .count()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shutdown_leaves_no_thread_behind() {
        // Today's scheduler keeps a thread alive for every editor ever built.
        // The WorkManager names its threads after itself, so this counts its
        // own and nobody else's, whatever runs beside it.
        let t = threaded();
        let prefix = format!("w{} ", t.work.instance());
        t.work.options().set("watchdog-every", 5).unwrap();
        // A busy executor...
        let ticker = Steps(|| Step::Sleep(ms(1)));
        t.work
            .submit(Spec::new("ticker"), Box::new(ticker))
            .unwrap();
        // ...and three threads that wait until they are asked to stop.
        for i in 0..3 {
            t.work
                .spawn(&format!("block {i}"), |stop, _| blocked_until_stopped(stop))
                .unwrap();
        }
        // The executor, the watchdog and the three.
        eventually("five threads", || threads_named(&prefix) == 5);

        t.work.shutdown(&(), PATIENCE);
        eventually("no thread left", || threads_named(&prefix) == 0);
    }

    // -- in the editor --------------------------------------------------------

    #[test]
    fn the_editor_runs_its_work_by_hand_in_tests_and_logs_its_reports() {
        let (editor, _env) = create_global_env::<GapBuffer>().expect("global env");
        let (job, trace) = scripted(&[Step::Continue]);
        editor.work().submit(Spec::new("probe"), job).unwrap();
        assert!(wakes(&trace).is_empty(), "nothing runs by itself in a test");
        editor.work().run_until_idle(&editor);
        assert_eq!(exits(&trace), [Exit::Finished]);

        let boom = Steps(|| -> Step { panic!("kaboom") });
        editor
            .work()
            .submit(Spec::new("boom"), Box::new(boom))
            .unwrap();
        editor.work().run_until_idle(&editor);
        assert!(
            editor
                .get_logs()
                .iter()
                .any(|line| line.contains("[BKG] panic: boom retired: kaboom")),
            "the editor's log has the report"
        );

        editor.shutdown(PATIENCE);
        let (late, _) = scripted(&[]);
        assert_eq!(editor.work().submit(Spec::new("late"), late), None);
    }

    #[test]
    fn the_editors_log_file_is_written_by_the_watchdog_and_at_shutdown() {
        // Not by whoever logs: a diagnostic costs a command no disk write.
        use crate::LispContext;
        let path = std::env::temp_dir().join(format!("rsedit-watchdog-log-{}", std::process::id()));
        let text = || std::fs::read_to_string(&path).unwrap_or_default();
        let (editor, _env) = create_global_env::<GapBuffer>().expect("global env");
        editor.enable_log_file(&path).expect("a log file");
        editor.log_diagnostic("logged by a command");
        assert!(!text().contains("logged by a command"), "not yet");
        editor.work().watch_once();
        assert!(text().contains("\nlogged by a command\n"), "{}", text());

        editor.log_diagnostic("the last word");
        editor.shutdown(PATIENCE);
        assert!(text().ends_with("\nthe last word\n"), "{}", text());
        let _ = std::fs::remove_file(&path);
    }
}
