//! What the file a buffer is visiting looked like when the editor last read
//! or wrote it: a stamp of its modification time and length, taken with one
//! `stat` and compared for difference. The file watcher compares them.
//!
//! Design: `doc/architettura-editor.typ, <file-su-disco>`.
use std::path::Path;
use std::time::SystemTime;

/// A file's modification time and length, as of some moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    /// `None` when the platform or filesystem would not say, in which case the
    /// length is the whole of what there is to compare. Rare, and better than
    /// refusing to watch the file at all.
    pub modified: Option<SystemTime>,
    pub len: u64,
}

impl FileStamp {
    /// Returns PATH's stamp, or `None` when it cannot be read at all -- which
    /// for this purpose means the file is not there.
    pub fn of(path: impl AsRef<Path>) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// What a check of the file found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnDisk {
    /// The same file the editor last read or wrote.
    Unchanged,
    /// Something else wrote it.
    Changed,
    /// It is no longer there.
    Gone,
    /// Nothing was ever recorded, so there is nothing to compare against.
    ///
    /// Not the same as `Changed`: a buffer that was never stamped has not been
    /// seen to differ from anything, and treating "I do not know" as "it
    /// changed" would revert a buffer on the strength of never having looked.
    Unknown,
}

/// Returns what RECORDED, the stamp kept, and FOUND, the stamp on disk now,
/// say about the file.
///
/// A function of two values rather than a method that reads the filesystem,
/// so the decision can be tested against every combination -- including the
/// ones that are awkward to arrange on a real disk, like a timestamp that went
/// backwards.
pub fn compare(recorded: Option<FileStamp>, found: Option<FileStamp>) -> OnDisk {
    match (recorded, found) {
        (None, _) => OnDisk::Unknown,
        (Some(_), None) => OnDisk::Gone,
        (Some(recorded), Some(found)) => {
            if recorded == found {
                OnDisk::Unchanged
            } else {
                OnDisk::Changed
            }
        }
    }
}
