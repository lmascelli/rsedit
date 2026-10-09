//! The background job that keeps a recoverable copy of unsaved work.
//!
//! # What it is for, and what it is not
//!
//! Everything else in this tier protects work from being *discarded* -- a quit
//! that asks, a save that asks, a reload that refuses. None of them help when
//! the editor stops without being asked: a crash, a lost terminal, a machine
//! that goes down. For that the only defence is a copy already on disk, and
//! this is what writes it.
//!
//! It is not a backup. A backup is the file's *previous* contents, written
//! once when you first save over it, and it answers "I broke this and saved
//! it". An auto-save is the buffer's *current* contents, rewritten as you
//! type, and it answers "the editor is gone and the work was never saved".
//! Different files, different lifetimes, different questions.
//!
//! # What it writes, and where
//!
//! `#name#` beside the file, which is the convention people already know. With
//! `auto-save-directory` set, all of them go there instead, with the path
//! flattened into the filename so that two `main.rs` from different
//! directories do not fight over one name.
//!
//! # Why the version and not the modified flag
//!
//! Because "modified" stays true from the first keystroke until the save, so a
//! job that wrote every modified buffer every turn would rewrite an untouched
//! one forever. [`crate::buffer::Buffer::version`] moves only when the text
//! does, so remembering the version last written turns "is there anything to
//! save" into an integer comparison -- and an editor left open overnight on a
//! modified buffer costs nothing rather than a write every few seconds.
use crate::{
    BufferTrait, EditorState,
    background::ScheduledTask,
};
use risp::{Env, LispContext};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// How often the auto-saver writes when nothing says otherwise.
///
/// Thirty seconds, which is Emacs' own `auto-save-timeout`. It is the answer
/// to "how much would you mind retyping", and half a minute is about as long
/// as anybody notices losing.
pub const DEFAULT_AUTO_SAVE_INTERVAL: Duration = Duration::from_secs(30);

/// How many buffers one turn may write.
///
/// Smaller than the watcher's batch because the work is bigger: a `stat` is a
/// syscall, a write is a file. This runs on the thread that also colours
/// syntax, and a turn that wrote twenty buffers would be twenty writes between
/// the user and their next colour update.
pub const BUFFERS_PER_TURN: usize = 4;

/// The name of the Lisp variable that turns auto-saving off.
pub const AUTO_SAVE: &str = "auto-save";

/// The name of the Lisp variable holding how often it writes, in seconds.
pub const AUTO_SAVE_INTERVAL: &str = "auto-save-interval";

/// The name of the Lisp variable naming one directory for every auto-save
/// file, instead of putting each beside its own file.
pub const AUTO_SAVE_DIRECTORY: &str = "auto-save-directory";

/// Whether auto-saving is switched on, as Lisp currently has it.
///
/// Unset means on, for the reason the watcher's does: something that protects
/// you from losing work should not have to be asked for.
pub fn auto_saving<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(AUTO_SAVE, true)
}

/// How often it writes, as Lisp currently has it.
pub fn auto_save_interval<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Duration {
    // A second is the floor because the point of the floor is that the editor
    // not spend its time writing: more often than once a second is no longer
    // insurance against a crash, it is the crash.
    match env.number_at_least(AUTO_SAVE_INTERVAL, 1.0) {
        Some(seconds) => Duration::from_secs_f64(seconds),
        None => DEFAULT_AUTO_SAVE_INTERVAL,
    }
}

/// The directory every auto-save file goes in, or `None` for "beside its own
/// file".
pub fn auto_save_directory<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<String> {
    match env.get_variable(AUTO_SAVE_DIRECTORY) {
        Some(crate::ELispExp::String(dir)) if !dir.is_empty() => Some(dir.to_string()),
        _ => None,
    }
}

/// PATH with its separators flattened, so that it can be one component of a
/// filename.
///
/// `/home/me/p/src/main.rs` becomes `!home!me!p!src!main.rs`, which is Emacs'
/// scheme and readable enough to recognise your own file in a listing.
///
/// # Why a literal `!` is doubled
///
/// So that two different paths cannot flatten to the same name. Without it,
/// `/a!b/c` and `/a/b/c` would both come out as `!a!b!c`, and one buffer's
/// recovery file would quietly be another's.
///
/// # Why this is not reversible
///
/// Because nothing needs it to be. Recovery always starts from a path you
/// have -- the buffer's own, or one you typed -- and computes the auto-save
/// name from it. Going the other way would mean deciding which separator each
/// `!` had been, which on a platform with two of them cannot be answered.
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

/// Where the auto-save file for FILE goes, given the configured DIRECTORY.
///
/// A pure function of two strings, so the naming can be tested against the
/// awkward paths -- a file at the root, a name that is already `#...#`, a path
/// with a `!` in it -- without any of them having to exist.
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
    pub fn new(env: Arc<Env<EditorState<B>>>) -> Self {
        Self { next: 0, env }
    }
}

impl<B: BufferTrait> ScheduledTask<B> for AutoSaver<B> {
    fn execute(&mut self, state: &EditorState<B>) -> bool {
        if auto_saving(&self.env) {
            let directory = auto_save_directory(&self.env);
            self.next = state.auto_save_turn(self.next, directory.as_deref());
        }
        // Never finishes: there is always more typing coming.
        true
    }
}

/// One buffer's copy, decided but not yet written.
pub(crate) struct AutoSave {
    buffer: String,
    where_to: PathBuf,
    text: String,
    /// The version this copy is of. Recorded against the buffer once the write
    /// lands, so the next turn can tell "nothing has changed" from "changed
    /// again while I was writing".
    version: u64,
}

impl<B: BufferTrait> EditorState<B> {
    /// Auto-save up to [`BUFFERS_PER_TURN`] buffers, starting at FROM, and
    /// answer where the next turn should start.
    pub(crate) fn auto_save_turn(&self, from: usize, directory: Option<&str>) -> usize {
        let names = self.buffer_names();
        if names.is_empty() {
            return 0;
        }
        let start = from % names.len();
        let count = BUFFERS_PER_TURN.min(names.len());
        for step in 0..count {
            let name = &names[(start + step) % names.len()];
            if let Some(pending) = self.auto_save_for(name, directory) {
                self.write_auto_save(pending);
            }
        }
        (start + count) % names.len()
    }

    /// What NAME needs written, or `None` when it needs nothing.
    pub(crate) fn auto_save_for(&self, name: &str, directory: Option<&str>) -> Option<AutoSave> {
        let (path, version, saved_at, modified, read_only, text) =
            self.with_buffer(name, |buf| {
                (
                    buf.file_path.clone(),
                    buf.version,
                    buf.auto_saved_at,
                    buf.is_modified,
                    buf.read_only,
                    buf.text.to_string(),
                )
            })?;

        // Only buffers with somewhere to be recovered *to*. A listing, a
        // backtrace and a manual page are "modified" in a way that means
        // nothing, and a recovery file for one would be a copy of a view.
        let path = path?;
        if read_only || !modified || saved_at == Some(version) {
            return None;
        }

        Some(AutoSave {
            buffer: name.to_string(),
            where_to: auto_save_path(&path, directory),
            text,
            version,
        })
    }

    /// Write what was decided, and record that it was written.
    pub(crate) fn write_auto_save(&self, pending: AutoSave) {
        let AutoSave {
            buffer,
            where_to,
            text,
            version,
        } = pending;
        // With no lock held, for the reason every other write here does it: a
        // slow disk must not stop the threads walking the buffer.
        if let Err(why) = std::fs::write(&where_to, text) {
            // Said once and not retried into a loop: a directory that cannot
            // be written to will not become writable because this asked again
            // in thirty seconds. Recording the version stops the complaint
            // repeating until the buffer changes.
            self.log_diagnostic(&format!(
                "[WARNING] could not auto-save {buffer} to {}: {why}",
                where_to.display()
            ));
        }
        // Recorded whether or not the write worked: it is "the version this
        // job has already tried", not "the version safely on disk".
        self.with_buffer_mut(&buffer, |buf| buf.auto_saved_at = Some(version));
    }

    /// Throw away NAME's auto-save file, because its contents are safely in
    /// the real one.
    pub(crate) fn discard_auto_save(&self, name: &str, directory: Option<&str>) {
        let Some(Some(path)) = self.with_buffer(name, |buf| buf.file_path.clone()) else {
            return;
        };
        let _ = std::fs::remove_file(auto_save_path(&path, directory));
        self.with_buffer_mut(name, |buf| buf.auto_saved_at = None);
    }
}
