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
        assert_eq!(work.spawn("late thread", |_| {}), None);
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
            .spawn("reader", move |stop: Stop| {
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
            .spawn("clumsy", move |stop: Stop| {
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
        t.work.spawn("doomed", |_| panic!("kaboom")).unwrap();
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
            .spawn("stubborn", move |_| {
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
    fn jobs_and_threads_share_one_namespace() {
        let (work, _) = manual();
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        work.spawn("x", move |stop: Stop| {
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
        work.spawn("x", |_| {}).unwrap();
        work.run_until_idle(&());
        assert_eq!(exits(&trace), [Exit::Cancelled]);
        eventually("both threads to be reaped", || work.threads().is_empty());
        work.shutdown(&(), PATIENCE);
    }

    /// How many threads of this process have a name starting with PREFIX.
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
                .spawn(&format!("block {i}"), blocked_until_stopped)
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
}
