//! The editor's own background jobs, each a `Job` on the WorkManager: the
//! highlighter and the prescanner, which bring each buffer's colouring and
//! scan checkpoints up to its text, the file watcher, which notices a file
//! changed under its buffer, the auto-saver, and the session saver, which the
//! host starts.
//!
//! Design: `doc/architettura-editor.typ, <lavori-editor>`.
pub mod autosave;
pub mod highlighter;
pub mod prescan;
pub mod session_saver;
pub mod watcher;
