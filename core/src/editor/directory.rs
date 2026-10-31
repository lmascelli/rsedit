//! The directory relative names are read in: the editor's working directory,
//! which `cd` moves, and the one a buffer may have of its own. The process's
//! own directory is read once, as the first answer, and never changed: a
//! command already running stays where it started, and threads in the
//! background never see it move under them.
//!
//! Design: `doc/architettura-editor.typ, <directory-di-lavoro>`.
use super::*;
use crate::paths::expand_path_in;

impl<B: BufferTrait> EditorState<B> {
    /// Returns the editor's working directory: where it was started, until
    /// [`Self::set_working_directory`] moves it.
    pub(crate) fn working_directory(&self) -> String {
        self.working_directory
            .read()
            .expect("Failed to acquire read lock on working_directory")
            .clone()
    }

    /// Makes PATH, read as a typed name is, the editor's working directory,
    /// and returns it; an error, and nothing changed, when it is not a
    /// directory. Commands already running stay where they started.
    pub(crate) fn set_working_directory(&self, path: &str) -> Result<String, String> {
        let directory = self.expand_path(path);
        if !std::path::Path::new(&directory).is_dir() {
            return Err(format!("{directory} is not a directory"));
        }
        *self
            .working_directory
            .write()
            .expect("Failed to acquire write lock on working_directory") = directory.clone();
        Ok(directory)
    }

    /// Returns the directory the relative names of the buffer NAME are read
    /// in: its own, when it has one, and the editor's otherwise.
    ///
    /// The one place that decides. A buffer that follows its file's
    /// directory, as in Emacs, is a change here and in what sets
    /// [`crate::buffer::Buffer::directory`], and in no caller.
    pub(crate) fn directory_of(&self, name: &str) -> String {
        // The buffer's lock let go before the editor's is taken: never both.
        let own = self
            .with_buffer(name, |buf| buf.directory.clone())
            .flatten();
        own.unwrap_or_else(|| self.working_directory())
    }

    /// Returns the directory relative names are read in now: the current
    /// buffer's.
    pub(crate) fn default_directory(&self) -> String {
        self.directory_of(&self.get_current_buffer_name())
    }

    /// Returns PATH made absolute as a typed name is: `~` expanded, read in
    /// the default directory when relative, `.` and `..` collapsed.
    pub(crate) fn expand_path(&self, path: &str) -> String {
        expand_path_in(path, std::path::Path::new(&self.default_directory()))
    }
}
