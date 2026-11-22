//! The directory relative names are read in, as Emacs' `default-directory`:
//! each buffer has one -- the one it was given, as a dired listing or a
//! command's output is, or else its file's folder -- and `cd` moves the
//! current buffer's alone. A buffer made with no file takes the directory of
//! the buffer it was made from. The editor's own working directory, where it
//! was started, is only what a buffer with neither goes by, as `*scratch*` at
//! startup. A call may be given one for as long as it runs. The process's own
//! directory is read once, as the first answer, and never changed: a command
//! already running stays where it started, and threads in the background
//! never see it move under them.
//!
//! Design: `doc/architettura-editor.typ, <directory-di-lavoro>`.
use super::*;
use crate::paths::expand_path_in;
use std::cell::RefCell;

thread_local! {
    /// The directory a call on this thread was given, which wins over the
    /// buffer's and the editor's while it runs: per thread, so a job's step
    /// on the executor and a command on its own thread never see each
    /// other's.
    static SCOPED_DIRECTORY: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Returns the directory the calling thread is scoped to, if any: what a job
/// started now keeps for its steps and its callbacks.
pub(crate) fn scoped_directory() -> Option<String> {
    SCOPED_DIRECTORY.with_borrow(Clone::clone)
}

/// Scopes the calling thread to a directory, or to none, until it is dropped,
/// and then puts back what it found: a call that fails or panics cannot leave
/// its directory behind, and scopes nest.
pub(crate) struct DirectoryScope(Option<String>);

impl DirectoryScope {
    /// Scopes the calling thread to DIRECTORY, already absolute; `None` lifts
    /// any scope, so the thread goes by the buffer's and the editor's again.
    pub(crate) fn enter(directory: Option<String>) -> Self {
        DirectoryScope(SCOPED_DIRECTORY.replace(directory))
    }
}

impl Drop for DirectoryScope {
    fn drop(&mut self) {
        SCOPED_DIRECTORY.set(self.0.take());
    }
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns the editor's working directory: where it was started. What a
    /// buffer with no directory of its own and no file goes by; nothing moves
    /// it, since `cd` moves a buffer's.
    pub(crate) fn working_directory(&self) -> String {
        self.working_directory
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Makes PATH, read as a typed name is, the current buffer's directory, or
    /// the scope's when the calling thread is in one, and returns it; an
    /// error, and nothing changed, when it is not a directory. What `cd`
    /// does: no other buffer moves, and commands already running stay where
    /// they started.
    pub(crate) fn change_directory(&self, path: &str) -> Result<String, String> {
        let directory = self.existing_directory(path)?;
        if scoped_directory().is_some() {
            SCOPED_DIRECTORY.set(Some(directory.clone()));
            return Ok(directory);
        }
        let mine = directory.clone();
        self.with_current_buffer_mut(|buf| buf.directory = Some(mine));
        Ok(directory)
    }

    /// Returns the directory the relative names of the buffer NAME are read
    /// in: its own, when it has one; else the folder of the file it visits;
    /// else the editor's.
    ///
    /// The one place that decides, as Emacs' `default-directory` would: a
    /// file's buffer follows its file, even when the file is renamed, until a
    /// `cd` there gives it one of its own.
    pub(crate) fn directory_of(&self, name: &str) -> String {
        // The buffer's lock let go before the editor's is taken: never both.
        let (own, file) = self
            .with_buffer(name, |buf| (buf.directory.clone(), buf.file_path.clone()))
            .unwrap_or((None, None));
        own.or_else(|| file.as_deref().and_then(folder_of))
            .unwrap_or_else(|| self.working_directory())
    }

    /// Returns PATH, read as a typed name is, when it names a directory, and
    /// the message that says it does not otherwise.
    pub(crate) fn existing_directory(&self, path: &str) -> Result<String, String> {
        let directory = self.expand_path(path);
        if std::path::Path::new(&directory).is_dir() {
            Ok(directory)
        } else {
            Err(format!("{directory} is not a directory"))
        }
    }

    /// Returns the directory relative names are read in now: the scope's,
    /// when the calling thread is in one, and the current buffer's otherwise.
    pub(crate) fn default_directory(&self) -> String {
        scoped_directory().unwrap_or_else(|| self.directory_of(&self.get_current_buffer_name()))
    }

    /// Returns PATH made absolute as a typed name is: `~` expanded, read in
    /// the default directory when relative, `.` and `..` collapsed.
    pub(crate) fn expand_path(&self, path: &str) -> String {
        expand_path_in(path, std::path::Path::new(&self.default_directory()))
    }
}

/// The folder holding the file at PATH, absolute, without a trailing
/// separator; `None` for a path with no parent, as `/`.
fn folder_of(path: &str) -> Option<String> {
    let parent = std::path::Path::new(path).parent()?;
    let shown = parent.display().to_string();
    (!shown.is_empty()).then_some(shown)
}
