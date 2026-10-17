//! The diagnostics log: what it keeps, what reaches its file and when, and
//! what it must survive.
#[cfg(test)]
mod tests {
    use crate::managers::Log;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    /// A file for one test to mirror its log into, gone when the test is.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            Scratch(
                std::env::temp_dir().join(format!("rsedit-log-test-{}-{name}", std::process::id())),
            )
        }

        fn create(&self) -> std::fs::File {
            std::fs::File::create(&self.0).expect("a scratch file")
        }

        fn text(&self) -> String {
            std::fs::read_to_string(&self.0).expect("the scratch file")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn the_log_keeps_only_the_most_recent_lines() {
        // AVV-4: a long session used to keep every diagnostic it ever logged.
        let mut log = Log::default();
        for i in 0..Log::MAX_LINES + 5 {
            log.record(&format!("line {i}"));
        }
        let lines = log.lines();
        assert_eq!(lines.len(), Log::MAX_LINES, "the oldest lines are dropped");
        assert_eq!(lines[0], "line 5", "the five oldest went first");
        assert_eq!(lines[Log::MAX_LINES - 1], format!("line {}", Log::MAX_LINES + 4));
    }

    #[test]
    fn a_write_the_file_refuses_is_dropped_rather_than_a_panic() {
        // AVV-3. A file opened only for reading refuses every write, on every
        // platform: the full disk, without filling one.
        let scratch = Scratch::new("refused");
        scratch.create();
        let read_only = std::fs::File::open(&scratch.0).expect("opened for reading only");

        let log = RwLock::new(Log::default());
        log.write()
            .unwrap()
            .enable_file(read_only)
            .expect("an empty list has nothing to catch up on");
        Log::append(&log, "first");
        Log::flush(&log);

        assert_eq!(
            log.read().unwrap().lines(),
            ["first"],
            "the line is kept all the same"
        );
    }

    #[test]
    fn what_is_logged_reaches_the_file_when_it_is_flushed_and_not_before() {
        // Nothing that logs writes the file: the watchdog's flush does. So a
        // command on a full disk, or a step of the scheduler, waits for no
        // write.
        let scratch = Scratch::new("flushed");
        let log = RwLock::new(Log::default());
        Log::append(&log, "before");
        log.write().unwrap().enable_file(scratch.create()).unwrap();
        assert_eq!(
            scratch.text(),
            "[LOG] before\n",
            "the history goes in at once"
        );

        Log::append(&log, "one");
        Log::append(&log, "two");
        assert_eq!(scratch.text(), "[LOG] before\n", "logging wrote nothing");
        Log::flush(&log);
        assert_eq!(scratch.text(), "[LOG] before\none\ntwo\n");

        Log::flush(&log);
        Log::append(&log, "three");
        Log::flush(&log);
        assert_eq!(
            scratch.text(),
            "[LOG] before\none\ntwo\nthree\n",
            "each line once, in order"
        );
    }

    #[test]
    fn without_a_file_a_flush_writes_nothing_and_owes_nothing() {
        // Lines logged before the file existed are its history, written when
        // it is enabled -- not again by the first flush after.
        let scratch = Scratch::new("history");
        let log = RwLock::new(Log::default());
        Log::append(&log, "early");
        Log::flush(&log);
        log.write().unwrap().enable_file(scratch.create()).unwrap();
        Log::flush(&log);
        assert_eq!(scratch.text(), "[LOG] early\n");
    }

    #[test]
    fn lines_that_leave_the_list_before_a_flush_never_reach_the_file() {
        // What is owed is a count of the newest lines, never more than the
        // list holds: the file takes the list's limit too.
        let scratch = Scratch::new("overflow");
        let log = RwLock::new(Log::default());
        log.write().unwrap().enable_file(scratch.create()).unwrap();
        for i in 0..Log::MAX_LINES + 5 {
            Log::append(&log, &format!("line {i}"));
        }
        Log::flush(&log);
        let text = scratch.text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), Log::MAX_LINES);
        assert_eq!(lines[0], "line 5");
        assert_eq!(
            lines[Log::MAX_LINES - 1],
            format!("line {}", Log::MAX_LINES + 4)
        );
    }

    #[test]
    fn flushes_from_two_threads_write_every_line_once_and_in_order() {
        // The watchdog flushes every turn and `shutdown` once more, possibly
        // at the same moment: what reaches the file is still the list, in
        // order, each line once.
        let scratch = Scratch::new("racing");
        let log = Arc::new(RwLock::new(Log::default()));
        log.write().unwrap().enable_file(scratch.create()).unwrap();
        let flusher = {
            let log = log.clone();
            std::thread::spawn(move || {
                for _ in 0..500 {
                    Log::flush(&log);
                }
            })
        };
        for i in 0..2_000 {
            Log::append(&log, &format!("line {i}"));
            if i % 7 == 0 {
                Log::flush(&log);
            }
        }
        flusher.join().unwrap();
        Log::flush(&log);
        let expected: String = (0..2_000).map(|i| format!("line {i}\n")).collect();
        assert_eq!(scratch.text(), expected);
    }
}
