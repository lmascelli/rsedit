//! The diagnostics log: what it keeps, what reaches its file and when, and
//! what it must survive.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
#[cfg(test)]
mod tests {
    use crate::managers::Log;
    use crate::tests::work_support::PATIENCE;
    use crate::{GapBuffer, create_global_env};
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
        // A long session must not grow with every diagnostic.
        let mut log = Log::default();
        for i in 0..Log::MAX_LINES + 5 {
            log.record(&format!("line {i}"));
        }
        let lines = log.lines();
        assert_eq!(lines.len(), Log::MAX_LINES, "the oldest lines are dropped");
        assert_eq!(lines[0], "line 5", "the five oldest went first");
        assert_eq!(
            lines[Log::MAX_LINES - 1],
            format!("line {}", Log::MAX_LINES + 4)
        );
    }

    #[test]
    fn a_write_the_file_refuses_is_dropped_rather_than_a_panic() {
        // A file opened only for reading refuses every write, on every
        // platform: a full disk, without filling one.
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

    // ---------------- one editor to a file ----------------

    /// A directory of its own for one test's log files, gone when it is.
    struct LogDir(PathBuf);

    impl LogDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsedit-log-dir-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch directory");
            LogDir(dir)
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.0.join(name)).unwrap_or_default()
        }
    }

    impl Drop for LogDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The second editor emptied the first one's file and wrote over its
    /// lines: one file, two writers, each from where it had got to.
    #[test]
    fn a_second_editor_logs_to_a_file_of_its_own() {
        use crate::LispContext;
        let dir = LogDir::new("two");
        let path = dir.0.join("rsedit.log");
        let (first, _) = create_global_env::<GapBuffer>().expect("global env");
        assert_eq!(first.enable_log_file(&path).expect("a log file"), path);
        first.log_diagnostic("the first editor's line");
        first.work().watch_once();

        let (second, _) = create_global_env::<GapBuffer>().expect("global env");
        let own = second.enable_log_file(&path).expect("a log file");
        let own_name = format!("rsedit-{}.log", std::process::id());
        assert_eq!(own, dir.0.join(&own_name));
        second.log_diagnostic("the second editor's line");
        second.work().watch_once();
        first.log_diagnostic("the first editor again");
        first.work().watch_once();

        let mine = dir.read("rsedit.log");
        assert!(mine.contains("\nthe first editor's line\n"), "{mine}");
        assert!(mine.ends_with("\nthe first editor again\n"), "{mine}");
        assert!(!mine.contains("second editor"), "{mine}");
        let theirs = dir.read(&own_name);
        assert!(theirs.contains("the second editor's line"), "{theirs}");
        assert!(
            second
                .get_logs()
                .iter()
                .any(|line| line.contains("is in use by another rsedit")),
            "and the second says where it logs"
        );
    }

    /// The file still begins again at each start, as the diagnostics page says.
    #[test]
    fn a_log_file_left_from_before_is_emptied_when_taken() {
        let dir = LogDir::new("left");
        let path = dir.0.join("rsedit.log");
        std::fs::write(&path, "an old session\n").expect("an old log");
        let (editor, _) = create_global_env::<GapBuffer>().expect("global env");
        editor.enable_log_file(&path).expect("a log file");
        editor.work().watch_once();
        assert!(!dir.read("rsedit.log").contains("an old session"));
    }

    // ---------------- no file at all ----------------

    /// Without a cache directory, or with a log file that cannot be made in
    /// it, the editor ran with its log in memory alone, and said nothing.
    #[test]
    fn with_no_cache_directory_the_editor_says_it_has_no_log_file() {
        let (editor, _) = create_global_env::<GapBuffer>().expect("global env");
        let started = editor.start_log_file(
            Err(crate::DirError::Undetermined(crate::AppDir::Cache)),
            "rsedit.log",
        );
        assert_eq!(started, None);
        assert!(
            editor
                .get_logs()
                .iter()
                .any(|line| line.starts_with("[WARNING] No log file")),
            "{:?}",
            editor.get_logs()
        );
        assert!(editor.get_echo_message().starts_with("No log file"));
    }

    #[test]
    fn a_log_file_that_cannot_be_made_is_named_and_does_not_hide_another_message() {
        let dir = LogDir::new("blocked");
        // A file where the cache directory should be.
        let blocked = dir.0.join("cache");
        std::fs::write(&blocked, "not a directory").expect("a file");
        let (editor, _) = create_global_env::<GapBuffer>().expect("global env");
        editor.set_echo_message("init.lisp stopped at an error");
        assert_eq!(
            editor.start_log_file(Ok(blocked.clone()), "rsedit.log"),
            None
        );
        let said = editor.get_logs().join("\n");
        assert!(
            said.contains(&blocked.join("rsedit.log").display().to_string()),
            "{said}"
        );
        assert_eq!(editor.get_echo_message(), "init.lisp stopped at an error");
    }

    #[test]
    fn a_cache_directory_that_works_gets_the_log() {
        let dir = LogDir::new("works");
        let (editor, _) = create_global_env::<GapBuffer>().expect("global env");
        assert_eq!(
            editor.start_log_file(Ok(dir.0.clone()), "rsedit.log"),
            Some(dir.0.join("rsedit.log"))
        );
        assert_eq!(
            editor.get_echo_message(),
            "Welcome to rsedit",
            "nothing to say"
        );
    }
}
