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
//! # The lock is not held across the write
//!
//! `log_diagnostic` used to take the write lock on the list and, still holding
//! it, `write_all` to the file. Every diagnostic in the editor -- and they are
//! logged from the worker thread as well as the command thread -- put a disk
//! write inside a lock that everything else logging had to queue behind.
//!
//! [`Log::record`] appends to the list and hands the caller back the line to
//! write, so the file write happens with nothing held. The file is behind its
//! own lock for the same reason: two threads may be writing to it at once, and
//! that is the only thing they need to agree about.
use std::{
    collections::VecDeque,
    fs::File,
    io::Write,
    sync::{Arc, RwLock},
};

#[derive(Default)]
pub struct Log {
    lines: VecDeque<String>,
    /// Where a copy goes, once somebody asks for one.
    ///
    /// Shared rather than owned so that [`Log::record`] can hand it out and
    /// let the caller write with this compartment's lock already given back.
    file: Option<Arc<RwLock<File>>>,
}

impl Log {
    /// How many lines the list keeps. Past it the oldest goes: a long session
    /// must not grow with every diagnostic (AVV-4), and the file, when there
    /// is one, already has them all.
    pub const MAX_LINES: usize = 10_000;

    /// Every diagnostic logged so far, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    /// Record MSG, and answer with the sink it should also be written to.
    ///
    /// The write itself is the caller's, deliberately: see the note above
    /// about not holding this lock across disk I/O. `None` when no file is
    /// enabled, which is the ordinary case.
    #[must_use = "the message still has to reach the file"]
    pub fn record(&mut self, msg: &str) -> Option<Arc<RwLock<File>>> {
        if self.lines.len() == Self::MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(msg.to_string());
        self.file.clone()
    }

    /// Record MSG in LOG and write it to the file, when one is enabled: the
    /// whole of logging a diagnostic, for whoever holds the compartment rather
    /// than the editor. The WorkManager's executor is one: it must not hold an
    /// editor, or the editor would hold itself.
    ///
    /// The sink comes back out with the lock, and the write happens with it
    /// given back: a disk write inside the lock every diagnostic takes is a
    /// queue everything else logging has to wait in, and diagnostics are
    /// logged from the worker threads as well as the command thread.
    pub fn append(log: &RwLock<Log>, msg: &str) {
        let sink = log.write().expect("write lock on log").record(msg);
        if let Some(file) = sink {
            Log::write_to(&file, msg);
        }
    }

    /// Write MSG to SINK, the file [`Log::record`] handed back.
    ///
    /// A write that fails is dropped, never a panic (AVV-3). With the disk
    /// full, the first attempt to save produces exactly a diagnostic, and that
    /// diagnostic used to close the editor. The line is in the list already;
    /// the file just has a gap.
    pub fn write_to(sink: &RwLock<File>, msg: &str) {
        if let Ok(mut file) = sink.write() {
            let _ = file.write_all(format!("{msg}\n").as_bytes());
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
        Ok(())
    }
}
