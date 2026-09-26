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
    fs::File,
    io::Write,
    sync::{Arc, RwLock},
};

#[derive(Default)]
pub struct Log {
    lines: Vec<String>,
    /// Where a copy goes, once somebody asks for one.
    ///
    /// Shared rather than owned so that [`Log::record`] can hand it out and
    /// let the caller write with this compartment's lock already given back.
    file: Option<Arc<RwLock<File>>>,
}

impl Log {
    /// Every diagnostic logged so far, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.lines.clone()
    }

    /// Record MSG, and answer with the sink it should also be written to.
    ///
    /// The write itself is the caller's, deliberately: see the note above
    /// about not holding this lock across disk I/O. `None` when no file is
    /// enabled, which is the ordinary case.
    #[must_use = "the message still has to reach the file"]
    pub fn record(&mut self, msg: &str) -> Option<Arc<RwLock<File>>> {
        self.lines.push(msg.to_string());
        self.file.clone()
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
