//! The watchdog: a step or an exit that does not end, the job listed as
//! running meanwhile, the hook after each turn, and the watchdog's own thread.
//!
//! Design: `doc/architettura-editor.typ, <watchdog>`.
#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use crate::tests::work_support::threads_named;
    use crate::tests::work_support::{
        PATIENCE, drive_elsewhere, eventually, held, manual, ms, scripted, stuck, threaded,
    };
    use crate::work::{Place, Spec, Step};
    use std::sync::{Arc, Mutex};

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
        // dedicated thread.
        let t = threaded();
        let prefix = format!("w{} ", t.work.instance());
        eventually("the executor and the watchdog", || {
            threads_named(&prefix) == 2
        });
    }
}
