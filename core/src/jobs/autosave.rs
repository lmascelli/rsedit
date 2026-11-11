//! The auto-saver: the job that writes a copy of each buffer with unsaved
//! work, `#name#` beside its file or in `auto-save-directory`, a few at a
//! turn, when its version has moved since the last copy. The copy is what
//! `recover-file` reads after the editor stopped without being asked.
//!
//! Design: `doc/architettura-editor.typ, <lavori-editor>`.
use crate::buffer::io_status::{AUTO_SAVE_REPORT, AutoSaveReport, FileIssue, IoCause};
use crate::editor::file_io::{Handle, Lane};
use crate::{
    BufferTrait, EditorState,
    work::{Exit, Job, JobError, Spec, Start, Step, StepContext},
};
use risp::{Env, LispContext};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const CLEANUP: &str = "file-io:recovery-cleanup";

#[derive(Clone, PartialEq, Eq)]
struct Cleanup {
    path: PathBuf,
    version: u64,
}

/// How often the auto-saver writes when nothing says otherwise: thirty
/// seconds, Emacs' own `auto-save-timeout`.
pub const DEFAULT_AUTO_SAVE_INTERVAL: Duration = Duration::from_secs(30);

/// How many buffers one turn may write: fewer than the watcher's, since a
/// write costs more than a `stat`, and the executor that runs it also colours.
pub const BUFFERS_PER_TURN: usize = 4;

/// The name of the Lisp variable that turns auto-saving off.
pub const AUTO_SAVE: &str = "auto-save";

/// The name of the Lisp variable holding how often it writes, in seconds.
/// Read at every step, as the sleep that follows it, so that a change counts
/// from the next write.
pub const AUTO_SAVE_INTERVAL: &str = "auto-save-interval";

/// The name of the Lisp variable naming one directory for every auto-save
/// file, instead of putting each beside its own file.
pub const AUTO_SAVE_DIRECTORY: &str = "auto-save-directory";

/// Returns whether auto-saving is on, as Lisp has it now; unset counts as on.
pub fn auto_saving<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(AUTO_SAVE, true)
}

/// Returns how often the auto-saver writes, as Lisp has it now: the seconds
/// of `auto-save-interval` when they are a number of at least one, and thirty
/// otherwise.
pub fn auto_save_interval<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Duration {
    // A second is the floor, so that the editor does not spend its time
    // writing copies.
    match env.number_at_least(AUTO_SAVE_INTERVAL, 1.0) {
        // A number too large for a `Duration` is a very long time, as for the
        // watcher.
        Some(seconds) => Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX),
        None => DEFAULT_AUTO_SAVE_INTERVAL,
    }
}

/// Returns the directory every auto-save file goes in, expanded as a typed
/// path is, so that `~` is the home directory, or `None` for beside its own
/// file.
pub fn auto_save_directory<B: BufferTrait>(
    env: &Arc<Env<EditorState<B>>>,
) -> Result<Option<String>, String> {
    match env.get_variable(AUTO_SAVE_DIRECTORY) {
        Some(crate::ELispExp::String(dir)) if !dir.is_empty() => {
            Ok(Some(crate::paths::expand_path_from_start(&dir)))
        }
        Some(value) if value.is_nil() => Ok(None),
        None => Err("Auto-save directory is not configured".into()),
        _ => Err("Auto-save directory unavailable: set auto-save-directory to a writable local directory".into()),
    }
}

/// Returns PATH as one component of a file name: each separator becomes `!`,
/// as in Emacs, so that `/home/me/p/src/main.rs` becomes
/// `!home!me!p!src!main.rs`; and `!`, `%` and `:` -- with `\\` where it is no
/// separator -- are written as `%` and their code, `%21`, `%25`, `%3A`, `%5C`.
///
/// So no two paths have the same name. Emacs doubles each `!` instead, and a
/// `!` beside a separator then reads like another path's: `/a!/b` and `/a/!b`
/// were both `!a!!!b`. Here every character is written one way and every
/// written run read one way back, as in a URL.
fn flatten(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    for ch in path.chars() {
        match ch {
            '/' => out.push('!'),
            '\\' if cfg!(windows) => out.push('!'),
            '!' => out.push_str("%21"),
            '%' => out.push_str("%25"),
            ':' => out.push_str("%3A"),
            '\\' => out.push_str("%5C"),
            other => out.push(other),
        }
    }
    out
}

/// Returns where the auto-save file of FILE goes: `#name#` beside it, or in
/// DIRECTORY with the whole path flattened into the name.
pub fn auto_save_path(file: &str, directory: Option<&str>) -> PathBuf {
    let path = Path::new(file);
    match directory {
        Some(directory) => Path::new(directory).join(format!("#{}#", flatten(file))),
        None => {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| flatten(file));
            match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent.join(format!("#{name}#")),
                _ => PathBuf::from(format!("#{name}#")),
            }
        }
    }
}

/// Writes a recoverable copy of whatever has changed, a few buffers at a turn.
pub struct AutoSaver<B: BufferTrait> {
    next: usize,
    env: Arc<Env<EditorState<B>>>,
}

impl<B: BufferTrait> AutoSaver<B> {
    /// Returns an auto-saver that reads its settings from ENV.
    pub fn new(env: Arc<Env<EditorState<B>>>) -> Self {
        Self { next: 0, env }
    }

    /// Returns how it is scheduled: in the normal class, first writing one
    /// interval from now, since what was typed before then is not worth a copy
    /// yet.
    pub(crate) fn spec(&self) -> Spec {
        Spec::new("auto-saver")
            .unit("files")
            .start(Start::After(auto_save_interval(&self.env)))
    }
}

impl<B: BufferTrait> Job<EditorState<B>> for AutoSaver<B> {
    fn step(&mut self, cx: &mut StepContext<'_, EditorState<B>>) -> Result<Step, JobError> {
        // Switched off, it is parked rather than ended, as the watcher is.
        if auto_saving(&self.env) {
            let (next, written) = cx.state.auto_save_async_turn(
                self.next,
                auto_save_directory(&self.env),
                crate::editor::file_io::timeout(&self.env),
            );
            self.next = next;
            cx.worked(written as u64, "auto-save files");
        }
        // Re-read at every step: see `AUTO_SAVE_INTERVAL`.
        Ok(Step::Sleep(auto_save_interval(&self.env)))
    }

    fn exit(&mut self, state: &EditorState<B>, why: Exit) {
        if why == Exit::Panicked {
            let env = self.env.clone();
            state.restart_after_panic("auto-saver", move || {
                let saver = AutoSaver::new(env);
                (saver.spec(), Box::new(saver))
            });
        }
    }
}

/// One buffer's copy, decided but not yet written.
pub(crate) struct AutoSave {
    buffer: String,
    source: String,
    where_to: PathBuf,
    text: String,
    /// The version this copy is of, recorded against the buffer once it is
    /// written, so that the next turn tells a buffer unchanged since from one
    /// changed again while it was written.
    version: u64,
}

impl<B: BufferTrait> EditorState<B> {
    /// The scheduler only snapshots memory. One bounded recovery worker does
    /// the disk writes, independently of reads, saves and the file watcher.
    pub(crate) fn auto_save_async_turn(
        &self,
        from: usize,
        directory: Result<Option<String>, String>,
        timeout: Option<Duration>,
    ) -> (usize, usize) {
        let names = self.buffer_names();
        if names.is_empty() {
            return (0, 0);
        }
        let directory = match directory {
            Ok(directory) => directory,
            Err(why) => {
                for name in &names {
                    let first = self.with_buffer_mut(name, |buf| {
                        if !buf.is_modified || buf.file_path.is_none() || buf.read_only() {
                            return false;
                        }
                        let first = !buf.auto_save_failing;
                        buf.note_auto_save(
                            std::time::SystemTime::now(),
                            buf.version,
                            None,
                            Err(FileIssue::new(IoCause::Configuration, why.clone())),
                        );
                        first
                    });
                    if first == Some(true) {
                        self.log_diagnostic(&format!("[ERROR] {why}"));
                        self.set_echo_message(&why);
                    }
                }
                return (from, 0);
            }
        };
        let start = from % names.len();
        let mut batch = Vec::new();
        let mut next = start;
        for step in 0..names.len() {
            let index = (start + step) % names.len();
            if let Some(handle) = self.file_handle(&names[index]) {
                self.try_recovery_cleanup(handle.clone(), timeout);
                if let Some(pending) = self.auto_save_for(&names[index], directory.as_deref()) {
                    batch.push((handle, pending));
                }
            }
            next = (index + 1) % names.len();
            if batch.len() >= BUFFERS_PER_TURN {
                break;
            }
        }
        if batch.is_empty() {
            return (next, 0);
        }
        let count = batch.len();
        let recovery_worker = self.clone();
        let submitted = self.start_file_io(
            Lane::Recovery,
            "recovery-batch".into(),
            "Auto-saving",
            None,
            timeout,
            move || {
                batch
                    .into_iter()
                    .map(|(handle, pending)| {
                        let result = recovery_worker.write_recovery(
                            &handle,
                            &pending.where_to,
                            pending.version,
                            &pending.text,
                        );
                        (handle, pending, result)
                    })
                    .collect::<Vec<_>>()
            },
            move |state, _, results| {
                for (handle, pending, result) in results {
                    if let Ok(version) = &result {
                        let mut buf = handle
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if buf.file_path.as_deref() == Some(pending.source.as_str()) {
                            buf.auto_saved_at =
                                Some(buf.auto_saved_at.map_or(*version, |old| old.max(*version)));
                        }
                    }
                    if result.is_err() {
                        // A later recovery can finish before this UI callback runs.
                        let failure = {
                            let buf = handle
                                .read()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            buf.data::<AutoSaveReport>(AUTO_SAVE_REPORT)
                                .filter(|report| {
                                    report.version == pending.version
                                        && report.copy.as_ref() == Some(&pending.where_to)
                                })
                                .and_then(|report| report.outcome.as_ref().err())
                                .cloned()
                        };
                        if let Some(why) = failure {
                            let message = format!("Auto-save failed for {}: {why}", pending.buffer);
                            state.set_echo_message(&message);
                            state.log_diagnostic(&message);
                        }
                    }
                    state.try_recovery_cleanup(handle, timeout);
                }
            },
        );
        if submitted.is_some() {
            (next, count)
        } else {
            (from, 0)
        }
    }

    /// Cleanup shares the recovery lane with writes. A save completing while
    /// a copy is being written cannot delete it just before it is renamed.
    pub(crate) fn queue_recovery_cleanup(
        &self,
        handle: Handle<B>,
        path: PathBuf,
        version: u64,
        timeout: Option<Duration>,
    ) {
        {
            let mut buf = handle
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if buf.data::<Vec<Cleanup>>(CLEANUP).is_none() {
                buf.put_data(CLEANUP, Vec::<Cleanup>::new());
            }
            let queue = buf
                .data_mut::<Vec<Cleanup>>(CLEANUP)
                .expect("cleanup queue");
            queue.retain(|item| item.path != path);
            queue.push(Cleanup { path, version });
        }
        self.try_recovery_cleanup(handle, timeout);
    }

    fn try_recovery_cleanup(&self, handle: Handle<B>, timeout: Option<Duration>) {
        let pending = {
            let buf = handle
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Preserve a copy if new work has appeared since the save.
            if buf.is_modified {
                return;
            }
            buf.data::<Vec<Cleanup>>(CLEANUP)
                .cloned()
                .unwrap_or_default()
        };
        if pending.is_empty() {
            return;
        }
        let done_handle = handle.clone();
        let cleanup_worker = self.clone();
        self.start_file_io(
            Lane::Recovery,
            "recovery-batch".into(),
            "Recovery cleanup",
            None,
            timeout,
            move || {
                pending
                    .into_iter()
                    .map(|cleanup| {
                        let result =
                            cleanup_worker.remove_recovery(&handle, &cleanup.path, cleanup.version);
                        (cleanup, result)
                    })
                    .collect::<Vec<_>>()
            },
            move |state, _, results| {
                let mut warnings = Vec::new();
                let mut buf = done_handle
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for (cleanup, result) in results {
                    match result {
                        Ok(true) => {
                            if let Some(queue) = buf.data_mut::<Vec<Cleanup>>(CLEANUP) {
                                queue.retain(|item| item != &cleanup);
                            }
                            if let Some(report) = buf.data_mut::<AutoSaveReport>(AUTO_SAVE_REPORT)
                                && report.copy.as_ref() == Some(&cleanup.path)
                                && report.version <= cleanup.version
                            {
                                report.removed = true;
                            }
                            if buf
                                .auto_saved_at
                                .is_some_and(|version| version <= cleanup.version)
                            {
                                buf.auto_saved_at = None;
                            }
                        }
                        Ok(false) => {}
                        Err(why) => warnings.push(format!(
                            "[WARNING] Could not remove recovery copy {}: {why}",
                            cleanup.path.display()
                        )),
                    }
                }
                drop(buf);
                for warning in warnings {
                    state.log_diagnostic(&warning);
                }
            },
        );
    }

    /// Writes the copies of up to [`BUFFERS_PER_TURN`] buffers, starting at
    /// FROM, and returns where the next turn starts and how many it wrote.
    #[cfg(test)]
    pub(crate) fn auto_save_turn(&self, from: usize, directory: Option<&str>) -> (usize, usize) {
        let names = self.buffer_names();
        if names.is_empty() {
            return (0, 0);
        }
        // Every buffer is looked at, from where the last turn stopped, and at
        // most BUFFERS_PER_TURN written: the limit is on the writes, which are
        // the cost. A turn that fills up leaves the rest to the next one.
        let start = from % names.len();
        let mut next = start;
        let mut written = 0;
        for step in 0..names.len() {
            let index = (start + step) % names.len();
            if written == BUFFERS_PER_TURN {
                next = index;
                break;
            }
            if let Some(pending) = self.auto_save_for(&names[index], directory) {
                // Made when first needed, or it would have to be made by
                // hand.
                if let Some(directory) = directory {
                    let _ = std::fs::create_dir_all(directory);
                }
                self.write_auto_save(pending);
                written += 1;
            }
            next = (index + 1) % names.len();
        }
        (next, written)
    }

    /// Returns what NAME needs written, or `None` when it needs nothing: no
    /// file, read-only, unmodified, or a copy of this version already tried.
    pub(crate) fn auto_save_for(&self, name: &str, directory: Option<&str>) -> Option<AutoSave> {
        self.with_buffer(name, |buf| {
            let path = buf.file_path.as_ref()?;
            if buf.read_only() || !buf.is_modified || buf.auto_saved_at == Some(buf.version) {
                return None;
            }
            Some(AutoSave {
                buffer: name.to_string(),
                source: path.clone(),
                where_to: auto_save_path(path, directory),
                text: buf.text.to_string(),
                version: buf.version,
            })
        })?
    }

    /// Writes PENDING, with no lock held, and records its version against the
    /// buffer once it is on disk.
    ///
    /// A write that fails is not recorded, and is tried again at the next turn:
    /// a full disk or a directory briefly gone is not a reason to stop keeping
    /// a copy until the buffer changes. The log hears of it once, when the
    /// writes start failing, and not at every turn after.
    #[cfg(test)]
    pub(crate) fn write_auto_save(&self, pending: AutoSave) {
        let AutoSave {
            buffer,
            where_to,
            text,
            version,
            ..
        } = pending;
        // With no lock held: a slow disk must not stop the threads that walk
        // the buffer.
        // Whole or not at all, as a save: a copy cut short would offer
        // `recover-file` half of the work.
        let Some(handle) = self.file_handle(&buffer) else {
            return;
        };
        let was_failing = handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .auto_save_failing;
        let result = self.write_recovery(&handle, &where_to, version, &text);
        if let Ok(version) = result {
            handle
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .auto_saved_at = Some(version);
        }
        if let Err(why) = result
            && !was_failing
        {
            self.set_echo_message(&format!("Auto-save failed for {buffer}: {why}"));
            self.log_diagnostic(&format!(
                "[WARNING] could not auto-save {buffer} to {}: {why}",
                where_to.display()
            ));
        }
    }
}
