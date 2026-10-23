//! Dedicated threads: the stop and the action that unblocks a body, a panic
//! in a thread, a thread that will not stop, and the one namespace threads
//! share with jobs.
//!
//! Design: `doc/architettura-editor.typ, <thread-dedicati>`.
#[cfg(test)]
mod tests {
    use crate::tests::work_support::{
        PATIENCE, blocked_until_stopped, eventually, exits, manual, ms, scripted, threaded, wakes,
    };
    use crate::work::{Exit, Spec, Step, Stop, Wake};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};

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
}
