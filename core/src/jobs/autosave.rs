//! The auto-saver: the job that writes a copy of each buffer with unsaved
//! work, `#name#` beside its file or in `auto-save-directory`, a few at a
//! turn, when its version has moved since the last copy. The copy is what
//! `recover-file` reads after the editor stopped without being asked.
//!
//! Design: `doc/architettura-editor.typ, <lavori-editor>`.
use crate::{
    BufferTrait, EditorState,
    work::{Job, JobError, Spec, Start, Step, StepContext},
};
use risp::{Env, LispContext};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
pub fn auto_save_directory<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<String> {
    match env.get_variable(AUTO_SAVE_DIRECTORY) {
        Some(crate::ELispExp::String(dir)) if !dir.is_empty() => {
            Some(crate::primitives::file_names::expand_path(&dir))
        }
        _ => None,
    }
}

// TODO(FIL-6): beside a separator, a `!` can give another path's name:
// `/a!/b` and `/a/!b` both become `!a!!!b`.
/// Returns PATH as one component of a file name, Emacs' way: each `/`, `\`
/// and `:` becomes `!`, and each `!` is doubled, so that
/// `/home/me/p/src/main.rs` becomes `!home!me!p!src!main.rs`, and `/a!b/c`
/// and `/a/b/c` stay apart.
fn flatten(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    for ch in path.chars() {
        match ch {
            '!' => out.push_str("!!"),
            '/' | '\\' | ':' => out.push('!'),
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
            let directory = auto_save_directory(&self.env);
            let (next, written) = cx.state.auto_save_turn(self.next, directory.as_deref());
            self.next = next;
            cx.worked(written as u64, "auto-save files");
        }
        // Re-read at every step: see `AUTO_SAVE_INTERVAL`.
        Ok(Step::Sleep(auto_save_interval(&self.env)))
    }
}

/// One buffer's copy, decided but not yet written.
pub(crate) struct AutoSave {
    buffer: String,
    where_to: PathBuf,
    text: String,
    /// The version this copy is of, recorded against the buffer once it is
    /// written, so that the next turn tells a buffer unchanged since from one
    /// changed again while it was written.
    version: u64,
}

impl<B: BufferTrait> EditorState<B> {
    /// Writes the copies of up to [`BUFFERS_PER_TURN`] buffers, starting at
    /// FROM, and returns where the next turn starts and how many it wrote.
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
        let (path, version, saved_at, modified, read_only) = self.with_buffer(name, |buf| {
            (
                buf.file_path.clone(),
                buf.version,
                buf.auto_saved_at,
                buf.is_modified,
                buf.read_only,
            )
        })?;

        // Only buffers with a file to be recovered to: a listing, a backtrace
        // and a manual page are modified in a way that means nothing.
        let path = path?;
        if read_only || !modified || saved_at == Some(version) {
            return None;
        }
        // The text only now, with its own version: a turn looks at every
        // buffer, and copying each one's text to decide would copy them all.
        let (text, version) = self.with_buffer(name, |buf| (buf.text.to_string(), buf.version))?;

        Some(AutoSave {
            buffer: name.to_string(),
            where_to: auto_save_path(&path, directory),
            text,
            version,
        })
    }

    // TODO(FIL-2): the copy is written in place, and an error halfway leaves
    // it cut short.
    // TODO(FIL-5): a failed write is recorded as tried, and is not tried again
    // until the buffer changes.
    /// Writes PENDING, with no lock held, and records its version against the
    /// buffer, whether or not the write worked.
    pub(crate) fn write_auto_save(&self, pending: AutoSave) {
        let AutoSave {
            buffer,
            where_to,
            text,
            version,
        } = pending;
        // With no lock held: a slow disk must not stop the threads that walk
        // the buffer.
        if let Err(why) = std::fs::write(&where_to, text) {
            // Said once: the version recorded below keeps the complaint from
            // coming back until the buffer changes.
            self.log_diagnostic(&format!(
                "[WARNING] could not auto-save {buffer} to {}: {why}",
                where_to.display()
            ));
        }
        // Recorded whether or not the write worked: it is the version this
        // job has tried, not the version safely on disk.
        self.with_buffer_mut(&buffer, |buf| buf.auto_saved_at = Some(version));
    }

    /// Removes NAME's auto-save file, once its contents are safely in the file
    /// itself.
    pub(crate) fn discard_auto_save(&self, name: &str, directory: Option<&str>) {
        let Some(Some(path)) = self.with_buffer(name, |buf| buf.file_path.clone()) else {
            return;
        };
        let _ = std::fs::remove_file(auto_save_path(&path, directory));
        self.with_buffer_mut(name, |buf| buf.auto_saved_at = None);
    }
}
