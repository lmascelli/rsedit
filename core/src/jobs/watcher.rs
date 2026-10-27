//! The file watcher: the job that compares a few open files each turn with
//! what was last read or written. A buffer without unsaved edits is reloaded;
//! one with them, or one that changed while its file was being read, is only
//! marked stale, and the next save asks.
//!
//! Design: `doc/architettura-editor.typ, <lavori-editor>`.
use crate::{
    BufferTrait, EditorState,
    buffer::{FileStamp, OnDisk, disk::compare},
    work::{Job, JobError, Spec, Start, Step, StepContext},
};
use risp::{Env, LispContext};
use std::sync::Arc;
use std::time::Duration;

/// How often the watcher looks.
///
/// Three seconds: long enough that the cost is invisible, short enough that a
/// branch switch has caught up before you have finished reading the commit
/// message.
pub const DEFAULT_WATCH_INTERVAL: Duration = Duration::from_secs(3);

/// How many files one turn may look at.
pub const FILES_PER_TURN: usize = 8;

/// The name of the Lisp variable that turns the watcher off.
pub const WATCH_FILES: &str = "watch-files";

/// The name of the Lisp variable holding how often it looks, in seconds.
/// Read at every step, as the sleep that follows it, so that a change counts
/// from the next look.
pub const WATCH_FILE_INTERVAL: &str = "watch-file-interval";

/// Returns how often the watcher looks, as Lisp has it now: the seconds of
/// `watch-file-interval` when they are a number of at least a tenth, and
/// three otherwise.
pub fn watch_interval<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Duration {
    // A tenth of a second, and not the auto-saver's one second: a handful of
    // modification times is cheap enough to look at that often.
    match env.number_at_least(WATCH_FILE_INTERVAL, 0.1) {
        // A number too large for a `Duration` is a very long time, not a panic
        // in the middle of a step.
        Some(seconds) => Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX),
        None => DEFAULT_WATCH_INTERVAL,
    }
}

/// Returns whether watching is on, as Lisp has it now; unset counts as on.
/// Set to nil, the job is parked rather than ended, and it can be switched on
/// again without a restart.
pub fn watching<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(WATCH_FILES, true)
}

/// Looks at a few open files each turn, in turn: its cursor is an index into
/// the buffer names as they stand at each turn. It carries the environment,
/// as a Lisp worker does, for the settings that switch it off and pace it.
pub struct FileWatcher<B: BufferTrait> {
    next: usize,
    env: Arc<Env<EditorState<B>>>,
}

impl<B: BufferTrait> FileWatcher<B> {
    /// Returns a watcher that reads its settings from ENV.
    pub fn new(env: Arc<Env<EditorState<B>>>) -> Self {
        Self { next: 0, env }
    }

    /// Returns how it is scheduled: in the normal class, first looking one
    /// interval from now, since nothing it could find has had time to change.
    pub(crate) fn spec(&self) -> Spec {
        Spec::new("file-watcher")
            .unit("buffers")
            .start(Start::After(watch_interval(&self.env)))
    }
}

impl<B: BufferTrait> Job<EditorState<B>> for FileWatcher<B> {
    fn step(&mut self, cx: &mut StepContext<'_, EditorState<B>>) -> Result<Step, JobError> {
        // Asked at every step, so that the setting means something after the
        // editor has booted. Switched off, the job is parked, not ended.
        if watching(&self.env) {
            let (next, looked_at) = cx.state.watch_files_turn(self.next);
            self.next = next;
            cx.worked(looked_at as u64, "open buffers");
        }
        // The interval too: `(setq watch-file-interval 10)` counts from the
        // next look.
        Ok(Step::Sleep(watch_interval(&self.env)))
    }
}

/// What one buffer's check found, and what has to be done about it.
enum Verdict {
    /// Reload it from what was read.
    Adopt {
        text: String,
        stamp: Option<FileStamp>,
    },
    /// Say it has diverged, and leave it alone.
    Flag,
}

/// One buffer's check, decided but not yet applied. The file is read with no
/// lock held, so the buffer may change before the verdict is applied, and a
/// test can change it there.
pub(crate) struct FileCheck {
    buffer: String,
    path: String,
    /// The buffer's version when the check began, compared again when the
    /// verdict is applied: see [`EditorState::store_file_check`].
    version: u64,
    verdict: Verdict,
}

impl<B: BufferTrait> EditorState<B> {
    /// Looks at up to [`FILES_PER_TURN`] buffers, starting at FROM, and
    /// returns where the next turn starts and how many it looked at.
    pub(crate) fn watch_files_turn(&self, from: usize) -> (usize, usize) {
        let names = self.buffer_names();
        if names.is_empty() {
            return (0, 0);
        }
        let start = from % names.len();
        let count = FILES_PER_TURN.min(names.len());
        for step in 0..count {
            let name = &names[(start + step) % names.len()];
            if let Some(check) = self.file_check_for(name) {
                self.store_file_check(check);
            }
        }
        ((start + count) % names.len(), count)
    }

    /// Compares NAME with its file and returns what to do, touching nothing:
    /// `None` when there is nothing to do, no file, nothing recorded to
    /// compare with, or the file as it was left.
    pub(crate) fn file_check_for(&self, name: &str) -> Option<FileCheck> {
        // Only what the question needs, under the lock: the lock is not held
        // across a `stat`, let alone a read.
        let (path, recorded, modified, version) = self.with_buffer(name, |buf| {
            buf.file_path
                .as_ref()
                .map(|path| (path.clone(), buf.file_stamp, buf.is_modified, buf.version))
        })??;

        let verdict = match compare(recorded, FileStamp::of(&path)) {
            OnDisk::Unchanged | OnDisk::Unknown => return None,
            // Gone is not a reason to empty the buffer: the text in it is now
            // the only copy there is.
            OnDisk::Gone => Verdict::Flag,
            // Decided here as well as when the verdict is applied: this one
            // saves reading a file whose contents would be thrown away, and
            // the one there is the one that keeps the buffer safe.
            OnDisk::Changed if modified => Verdict::Flag,
            OnDisk::Changed => match std::fs::read_to_string(&path) {
                Ok(text) => Verdict::Adopt {
                    text,
                    // Stamped after the read, so that a file rewritten during
                    // it is caught next turn rather than recorded as seen.
                    stamp: FileStamp::of(&path),
                },
                // Unreadable is not unchanged, nor something to reload from.
                Err(_) => Verdict::Flag,
            },
        };

        Some(FileCheck {
            buffer: name.to_string(),
            path,
            version,
            verdict,
        })
    }

    /// Applies what CHECK decided, if the buffer has not changed since the
    /// check began. One that has, or that has unsaved edits, is marked stale
    /// and left alone, so that nothing typed while the file was being read is
    /// lost.
    pub(crate) fn store_file_check(&self, check: FileCheck) {
        let FileCheck {
            buffer,
            path,
            version,
            verdict,
        } = check;
        let reverted = self.with_buffer_mut(&buffer, |buf| {
            if buf.version != version || buf.is_modified {
                // It changed while the file was being read: marked, as if it
                // had been modified all along, and not touched.
                buf.stale = true;
                return false;
            }
            match verdict {
                Verdict::Flag => {
                    buf.stale = true;
                    false
                }
                Verdict::Adopt { text, stamp } => {
                    buf.adopt_text(&text);
                    buf.file_stamp = stamp;
                    true
                }
            }
        });

        if reverted == Some(true) {
            self.log_diagnostic(&format!("[INFO] {buffer} reloaded: {path} changed on disk"));
        }
    }
}
