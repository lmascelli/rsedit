//! The diagnostics log: what it keeps, and what it must survive.
#[cfg(test)]
mod tests {
    use crate::managers::Log;

    #[test]
    fn the_log_keeps_only_the_most_recent_lines() {
        // AVV-4: a long session used to keep every diagnostic it ever logged.
        let mut log = Log::default();
        for i in 0..Log::MAX_LINES + 5 {
            let _ = log.record(&format!("line {i}"));
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
        let path = std::env::temp_dir().join(format!("rsedit-log-test-{}", std::process::id()));
        std::fs::write(&path, "").expect("a scratch file");
        let read_only = std::fs::File::open(&path).expect("opened for reading only");

        let mut log = Log::default();
        log.enable_file(read_only)
            .expect("an empty list has nothing to catch up on");
        let sink = log.record("first").expect("the file is enabled");
        Log::write_to(&sink, "first");

        assert_eq!(log.lines(), ["first"], "the line is kept all the same");
        let _ = std::fs::remove_file(&path);
    }
}
