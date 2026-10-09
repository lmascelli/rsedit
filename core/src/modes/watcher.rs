//! The background job that notices when a file changes under a buffer.
//!
//! # What it does, and what it deliberately does not
//!
//! Every turn it looks at a few of the open files and compares what is on disk
//! against what was recorded when the editor last read or wrote them. A buffer
//! with **no unsaved edits** is reloaded silently -- which is the whole
//! pleasure of the feature: switch branches, and the windows are already
//! showing the new text. A buffer **with** unsaved edits is never touched. It
//! is marked stale instead, and the next save asks before writing over
//! somebody else's work.
//!
//! The asymmetry is the point. Reloading a clean buffer loses nothing, so it
//! needs no permission; reloading a dirty one loses exactly the thing this
//! whole tier of work exists to protect, so it is not something a background
//! job may decide.
//!
//! # Why a few files per turn
//!
//! Because `stat` is a syscall into a filesystem, and a filesystem can hang.
//! This runs on the thread that also colours syntax and pre-scans expressions,
//! so an unbounded sweep would put every open file between the user and their
//! next colour update. Bounding the batch bounds the damage to
//! [`FILES_PER_TURN`] stalls per turn instead of one per open buffer.
//!
//! It does not bound the damage from *one* hung file, and nothing here can:
//! a `stat` on a dead network mount blocks this thread for as long as it
//! blocks. The honest mitigations are to turn the watcher off
//! (`watch-files`), or to give file work a thread of its own -- which is a
//! different design, and one worth making deliberately rather than arriving at
//! by accident.
//!
//! # The rule it shares with the highlighter
//!
//! **Never store a result for a buffer that has moved on.** The file is read
//! with no lock held, and by the time the text comes back the user may have
//! typed. So the version is checked again at the moment of storing, not merely
//! when the check began -- and a buffer that became modified in that window is
//! left alone, exactly as though it had been modified all along.
use crate::{
    BufferTrait, EditorState,
    background::ScheduledTask,
    buffer::{FileStamp, OnDisk, disk::compare},
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
///
/// Read once, when the job is scheduled: the scheduler holds an interval per
/// job and there is no message that changes one. Changing it takes effect next
/// time the editor starts, which is the honest thing to say about a setting
/// nobody changes twice.
pub const WATCH_FILE_INTERVAL: &str = "watch-file-interval";

/// How often the watcher looks, as Lisp currently has it.
pub fn watch_interval<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Duration {
    // A tenth of a second, and not the auto-saver's one second: looking at a
    // handful of modification times is cheap enough that somebody who wants it
    // to feel immediate may have that, and the cost of being wrong is a little
    // idle `stat`ting rather than a disk write.
    match env.number_at_least(WATCH_FILE_INTERVAL, 0.1) {
        Some(seconds) => Duration::from_secs_f64(seconds),
        None => DEFAULT_WATCH_INTERVAL,
    }
}

/// Whether watching is switched on, as Lisp currently has it.
///
/// Unset means on: a feature that protects you from losing work should not
/// need to be asked for. Setting it to nil parks the job rather than ending
/// it, so turning it back on needs no restart -- which matters because the one
/// reason to turn it off is a filesystem that is misbehaving, and that usually
/// stops.
pub fn watching<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(WATCH_FILES, true)
}

/// Looks at a few open files each turn, forever.
///
/// The cursor is an index into the buffer names as they stand each turn, so a
/// buffer created or killed between turns shifts what comes next. That is
/// acceptable for a round robin whose only promise is that nothing is starved:
/// every buffer is reached within a few turns however the list moves, and the
/// alternative -- remembering a name and resuming after it -- costs a search
/// per turn to fix an unfairness nobody can perceive.
///
/// It carries the environment for the same reason a Lisp worker does: the
/// scheduler holds an `EditorState` and no environment, and the setting that
/// switches this off lives in the environment. `Env` crosses threads already.
pub struct FileWatcher<B: BufferTrait> {
    next: usize,
    env: Arc<Env<EditorState<B>>>,
}

impl<B: BufferTrait> FileWatcher<B> {
    pub fn new(env: Arc<Env<EditorState<B>>>) -> Self {
        Self { next: 0, env }
    }
}

impl<B: BufferTrait> ScheduledTask<B> for FileWatcher<B> {
    fn execute(&mut self, state: &EditorState<B>) -> bool {
        // Asked every turn rather than once at the start, so that the setting
        // means something after the editor has booted.
        if watching(&self.env) {
            self.next = state.watch_files_turn(self.next);
        }
        // Never finishes: there is always another write coming.
        true
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

/// One buffer's check, decided but not yet applied.
///
/// # Why this is a value and not a statement
///
/// Because the interesting part happens *between* deciding and applying. The
/// file is read with no lock held -- it has to be, since a read is exactly the
/// slow thing a lock must not span -- and in that window the user may have
/// typed into the buffer being checked. A test cannot arrange that window if
/// the two halves are one function, which is the same reason the highlighter
/// separated `store_turn` from the turn that computed it.
pub(crate) struct FileCheck {
    buffer: String,
    path: String,
    /// What the buffer looked like when the check began. Compared again at the
    /// moment of storing; see [`EditorState::store_file_check`].
    version: u64,
    verdict: Verdict,
}

impl<B: BufferTrait> EditorState<B> {
    /// Look at up to [`FILES_PER_TURN`] buffers, starting at FROM, and answer
    /// where the next turn should start.
    pub(crate) fn watch_files_turn(&self, from: usize) -> usize {
        let names = self.buffer_names();
        if names.is_empty() {
            return 0;
        }
        let start = from % names.len();
        let count = FILES_PER_TURN.min(names.len());
        for step in 0..count {
            let name = &names[(start + step) % names.len()];
            if let Some(check) = self.file_check_for(name) {
                self.store_file_check(check);
            }
        }
        (start + count) % names.len()
    }

    /// Compare one buffer against its file, and decide -- touching nothing.
    ///
    /// `None` when there is nothing to decide: no file, nothing recorded to
    /// compare against, or the file is exactly as it was left.
    pub(crate) fn file_check_for(&self, name: &str) -> Option<FileCheck> {
        // What the question needs, taken under the lock and nothing else: the
        // lock must not be held across a `stat`, let alone a read.
        let (path, recorded, modified, version) = self.with_buffer(name, |buf| {
            buf.file_path
                .as_ref()
                .map(|path| (path.clone(), buf.file_stamp, buf.is_modified, buf.version))
        })??;

        let verdict = match compare(recorded, FileStamp::of(&path)) {
            OnDisk::Unchanged | OnDisk::Unknown => return None,
            // Gone is not a reason to empty the buffer. The text in front of
            // the user is now the only copy there is, which makes it more
            // precious rather than less.
            OnDisk::Gone => Verdict::Flag,
            // Decided here as well as at the moment of storing, and not
            // because one of the two is redundant: this one saves *reading a
            // file whose contents will certainly be thrown away*, which on a
            // large file is the whole cost of the check. The one below is the
            // one that is about correctness.
            OnDisk::Changed if modified => Verdict::Flag,
            OnDisk::Changed => match std::fs::read_to_string(&path) {
                Ok(text) => Verdict::Adopt {
                    text,
                    // Stamped after the read, so a file rewritten *during* it
                    // is caught next turn rather than recorded as seen.
                    stamp: FileStamp::of(&path),
                },
                // Unreadable is not unchanged, and it is not something to
                // reload from either.
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

    /// Apply what a check decided, if the buffer still looks the way it did
    /// when the check began.
    ///
    /// The rule the highlighter has: **never store a result for a buffer that
    /// has moved on**. Here it is doing more than refusing a stale answer --
    /// it is the line between a background job that reloads a clean buffer and
    /// one that throws away work somebody typed while the file was being read.
    pub(crate) fn store_file_check(&self, check: FileCheck) {
        let FileCheck {
            buffer,
            path,
            version,
            verdict,
        } = check;
        let reverted = self.with_buffer_mut(&buffer, |buf| {
            if buf.version != version || buf.is_modified {
                // It moved while the file was being read. Treated exactly as
                // though it had been modified all along: marked, never
                // touched.
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
