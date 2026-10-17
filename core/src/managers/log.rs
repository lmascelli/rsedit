//! Diagnostics: what has been logged, and where a copy of it goes.
//!
//! # Why these two are one thing
//!
//! The file is not a second destination so much as a *mirror* of the list.
//! Turning it on writes out everything logged so far, so the file always holds
//! the whole history rather than the part that happened after somebody thought
//! to ask for it. That only works if the list and the file are settled
//! together: between reading the list and installing the file, a diagnostic
//! logged by another thread is written to neither -- it arrives too late for
//! the catch-up and too early for the sink.
//!
//! # Who writes the file
//!
//! Not whoever logs. [`Log::append`] puts the line in the list and counts it
//! as not written yet, and that is all: no disk, so logging costs the same on
//! a full disk or a home directory on the network as anywhere, and nothing
//! that logs -- a command, a step of the scheduler -- can be held up by it.
//!
//! [`Log::flush`] writes what is owed, and the editor has the WorkManager's
//! watchdog call it at every turn: a quarter of a second, by default, from a
//! line being logged to its being in the file, and on a thread that is not
//! stuck when everything else is -- which is when a log is read. It is
//! called once more as the editor shuts down.
//!
//! `log_diagnostic` used to take the list's lock and, still holding it,
//! `write_all` to the file, which put a disk write inside a lock every
//! diagnostic in the editor had to queue behind. The flush writes with the
//! list's lock given back.
use std::{
    collections::VecDeque,
    fs::File,
    io::Write,
    sync::{Arc, PoisonError, RwLock},
};

#[derive(Default)]
pub struct Log {
    lines: VecDeque<String>,
    /// How many of the newest lines are not in the file yet. Before there is
    /// a file, all of them: they go in as its history when it is enabled.
    unwritten: usize,
    /// Where a copy goes, once somebody asks for one.
    ///
    /// Shared rather than owned so that [`Log::flush`] can take it out and
    /// write with this compartment's lock given back.
    file: Option<Arc<RwLock<File>>>,
}

impl Log {
    /// How many lines the list keeps. Past it the oldest goes: a long session
    /// must not grow with every diagnostic (AVV-4). A line that goes before it
    /// was written never reaches the file -- which takes more lines than the
    /// list holds, logged within one turn of the watchdog.
    pub const MAX_LINES: usize = 10_000;

    /// Every diagnostic logged so far, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    /// Record MSG: in the list, and as owed to the file.
    pub fn record(&mut self, msg: &str) {
        if self.lines.len() == Self::MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(msg.to_string());
        self.unwritten = (self.unwritten + 1).min(self.lines.len());
    }

    /// Record MSG in LOG: the whole of logging a diagnostic, for whoever
    /// holds the compartment rather than the editor. The WorkManager's
    /// executor is one: it must not hold an editor, or the editor would hold
    /// itself.
    ///
    /// The file is [`Log::flush`]'s.
    pub fn append(log: &RwLock<Log>, msg: &str) {
        log.write().expect("write lock on log").record(msg);
    }

    /// Write to the file the lines LOG holds that are not in it yet, oldest
    /// first. Nothing when there is no file.
    ///
    /// The file's lock is taken before the lines are, and held until they are
    /// written: two flushes at once write in the order the lines were
    /// logged. The list's lock is held only to copy them out.
    ///
    /// A write that fails is dropped, never a panic (AVV-3). With the disk
    /// full, the first attempt to save produces exactly a diagnostic, and that
    /// diagnostic used to close the editor. The lines stay in the list; the
    /// file just has a gap. A poisoned lock is used as it is, for the same
    /// reason: this runs on the watchdog, which reports everybody else's
    /// panics and must not have one of its own.
    pub fn flush(log: &RwLock<Log>) {
        let file = log
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .file
            .clone();
        let Some(file) = file else {
            return;
        };
        let mut file = file.write().unwrap_or_else(PoisonError::into_inner);
        let text: String = {
            let mut log = log.write().unwrap_or_else(PoisonError::into_inner);
            let owed = std::mem::take(&mut log.unwritten);
            let from = log.lines.len().saturating_sub(owed);
            log.lines
                .iter()
                .skip(from)
                .map(|line| format!("{line}\n"))
                .collect()
        };
        if !text.is_empty() {
            let _ = file.write_all(text.as_bytes());
        }
    }

    /// Start mirroring to FILE, first writing out everything logged so far.
    ///
    /// The catch-up happens under this lock, with the file not yet installed:
    /// that is what stops a diagnostic arriving between the two and being
    /// written neither as history nor as news.
    pub fn enable_file(&mut self, mut file: File) -> std::io::Result<()> {
        for msg in &self.lines {
            file.write_all(format!("[LOG] {msg}\n").as_bytes())?;
        }
        self.file = Some(Arc::new(RwLock::new(file)));
        self.unwritten = 0;
        Ok(())
    }
}
