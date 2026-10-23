//! Diagnostics: the lines logged so far, and the file they are mirrored to.
//! Logging only adds a line to the list; the file is written by
//! [`Log::flush`], which the WorkManager's watchdog calls every turn, with the
//! list's lock given back.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
use std::{
    collections::VecDeque,
    fs::File,
    io::Write,
    sync::{Arc, PoisonError, RwLock},
};

/// The diagnostics logged so far, and the file they are copied to.
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
    /// How many lines the list keeps; past it the oldest goes, so that a long
    /// session does not grow with every diagnostic. A line dropped before it
    /// was written never reaches the file, which takes more lines than this
    /// logged within one turn of the watchdog.
    pub const MAX_LINES: usize = 10_000;

    /// Returns every line logged so far, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    /// Records MSG: in the list, and as owed to the file.
    pub fn record(&mut self, msg: &str) {
        if self.lines.len() == Self::MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(msg.to_string());
        self.unwritten = (self.unwritten + 1).min(self.lines.len());
    }

    /// Records MSG in LOG, taking its lock: logging for whoever holds the
    /// compartment rather than the editor, as the WorkManager does -- it must
    /// not hold an editor, which holds it.
    pub fn append(log: &RwLock<Log>, msg: &str) {
        log.write().expect("write lock on log").record(msg);
    }

    /// Writes to the file, oldest first, the lines LOG holds that are not in
    /// it yet; nothing when there is no file.
    ///
    /// The file's lock is taken first and held until the lines are written,
    /// so that two flushes write in the order the lines were logged; the
    /// list's lock only while they are copied out. A failed write is dropped,
    /// leaving a gap in the file, and a poisoned lock is used as it is: this
    /// runs on the watchdog, which must not panic -- and with the disk full,
    /// saving a file logs a diagnostic whose writing fails too.
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
            // TODO(AVV-6): written where this file had got to, over the lines
            // a second editor sharing it has written there.
            let _ = file.write_all(text.as_bytes());
        }
    }

    /// Starts mirroring to FILE, first writing out, marked `[LOG]`, everything
    /// logged so far: under this lock, so that no line logged meanwhile is
    /// missed or written twice.
    pub fn enable_file(&mut self, mut file: File) -> std::io::Result<()> {
        for msg in &self.lines {
            file.write_all(format!("[LOG] {msg}\n").as_bytes())?;
        }
        self.file = Some(Arc::new(RwLock::new(file)));
        self.unwritten = 0;
        Ok(())
    }
}
