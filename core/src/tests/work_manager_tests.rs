//! The WorkManager: the core with an executor around it, the registry and its
//! names, the hook after a step, the manual mode and shutdown.
//!
//! Design: `doc/architettura-editor.typ, <work-manager>`.
#[cfg(test)]
mod tests {
    use crate::tests::work_support::{
        PATIENCE, Steps, eventually, exits, manual, ms, scripted, threaded, wakes,
    };
    #[cfg(target_os = "linux")]
    use crate::tests::work_support::{blocked_until_stopped, threads_named};
    use crate::work::{Class, Exit, Place, Spec, Start, Step, Wake, WorkInfo};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

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
        // The example in doc/architettura-editor.typ, <argomenti>.
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

    // -- no thread left behind ------------------------------------------------

    #[cfg(target_os = "linux")]
    #[test]
    fn shutdown_leaves_no_thread_behind() {
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
}
