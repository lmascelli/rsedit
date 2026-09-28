//! What the file a buffer is visiting looked like when we last touched it.
//!
//! # Why a stamp and not a hash
//!
//! The question is "has this file changed since I read it", asked of every
//! open buffer every few seconds. A hash answers it exactly and reads the
//! whole file to do so; a stamp answers it from one `stat`, which is a
//! syscall and no I/O at all. For a question asked this often, of files this
//! large, that is the whole difference between a feature and a tax.
//!
//! What a stamp misses is a write that lands inside the filesystem's
//! timestamp granularity *and* leaves the length identical. Recording the
//! length alongside the time is what makes that narrow: a same-second write
//! that happens to preserve the byte count. The consequence of missing one is
//! a buffer that stays stale until the next change, not corruption -- and the
//! consequence of a *false* positive would be a spurious revert, which is why
//! the two are compared together rather than either alone.
//!
//! # Why the comparison is for difference, not for "newer"
//!
//! Because timestamps go backwards in ordinary use. `git checkout` restores
//! older ones, `touch -d` writes whatever it is told, network filesystems
//! disagree with the local clock, and a system that has just had its time
//! corrected disagrees with itself. "Different from what I recorded" is a
//! question the filesystem can answer; "newer than mine" is a question about
//! two clocks.
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
    /// Read PATH's stamp, or `None` when it cannot be read at all -- which
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
    /// The same file we last read or wrote.
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

/// Compare what is on disk now against what was recorded.
///
/// A free function of two values rather than a method that reads the
/// filesystem, so the decision can be tested against every combination --
/// including the ones that are awkward to arrange on a real disk, like a
/// timestamp that went backwards.
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
