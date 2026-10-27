//! The editor's own background jobs, each a `Job` on the WorkManager: the
//! highlighter and the prescanner, which bring each buffer's colouring and
//! scan checkpoints up to its text, the file watcher, which notices a file
//! changed under its buffer, and the auto-saver.
//!
//! Design: `doc/architettura-editor.typ, <lavori-editor>`.
pub mod autosave;
pub mod highlighter;
pub mod prescan;
pub mod watcher;
