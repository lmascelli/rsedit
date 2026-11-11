//! What the file a buffer is visiting looked like when the editor last read
//! or wrote it: a stamp of its modification time and length, taken with one
//! `stat` and compared for difference. The file watcher compares them. And
//! how a file is written: whole, or not at all.
//!
//! Design: `doc/architettura-editor.typ, <file-su-disco>`.
use std::fs::{File, Metadata, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
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
    /// Only NotFound means absent. Network and permission errors stay errors.
    pub fn try_of(path: impl AsRef<Path>) -> std::io::Result<Option<Self>> {
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(Some(Self {
                modified: metadata.modified().ok(),
                len: metadata.len(),
            })),
            Err(why) if why.kind() == ErrorKind::NotFound => Ok(None),
            Err(why) => Err(why),
        }
    }

    /// Lossy compatibility helper. Use try_of when absence must be
    /// distinguished from an access or network error.
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

/// Writes CONTENTS to the file at PATH so that what is there afterwards is the
/// old file or the new one, never one cut short: a write that stops halfway --
/// a full disk, the editor killed, the power gone -- leaves the old file.
///
/// The new text goes into a file of its own beside the old one, which is
/// given the old one's permissions, flushed to the disk, and then renamed
/// over it, a step the system does whole. Where a rename would change what
/// the file is, it is written in place instead, as `std::fs::write` does:
///
/// - through a symbolic link, the file it names is written, and the link
///   stays a link;
/// - a file with other names (hard links) is written in place, or the other
///   names would keep the old text;
/// - so is one owned by another user or group, which a new file would not be;
/// - and so is one in a directory the editor may not write in, where no new
///   file can be made beside it.
pub fn write_atomically(path: impl AsRef<Path>, contents: &[u8]) -> std::io::Result<()> {
    write_with_check(path.as_ref(), contents, &mut || Ok(()))
}

/// A changed file needs confirmation; an uncertain write must retain both
/// the modified flag and its recovery copy.
#[derive(Debug)]
pub(crate) enum WriteFailure {
    Conflict(Option<FileStamp>),
    Io(std::io::Error),
    Uncertain(String),
}

#[derive(Debug)]
struct Changed(Option<FileStamp>);
impl std::fmt::Display for Changed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "file changed since it was read")
    }
}
impl std::error::Error for Changed {}

pub(crate) fn write_checked(
    path: &str,
    contents: &[u8],
    expected: Option<FileStamp>,
) -> Result<FileStamp, WriteFailure> {
    let check = &mut || {
        let found = FileStamp::try_of(path)?;
        if found != expected {
            Err(std::io::Error::new(
                ErrorKind::AlreadyExists,
                Changed(found),
            ))
        } else {
            Ok(())
        }
    };
    if let Err(why) = check().and_then(|()| write_with_check(Path::new(path), contents, check)) {
        return Err(
            match why
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<Changed>())
            {
                Some(Changed(found)) => WriteFailure::Conflict(*found),
                None => WriteFailure::Io(why),
            },
        );
    }
    match FileStamp::try_of(path) {
        Ok(Some(stamp)) if stamp.len == contents.len() as u64 => Ok(stamp),
        Ok(_) => Err(WriteFailure::Uncertain(
            "write returned, but the file cannot be verified".into(),
        )),
        Err(why) => Err(WriteFailure::Uncertain(format!(
            "write returned, but verification failed: {why}"
        ))),
    }
}

fn write_with_check(
    path: &Path,
    contents: &[u8],
    check: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let target = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => match std::fs::canonicalize(path) {
            Ok(target) => target,
            // A link to nothing yet: writing through it makes the file.
            Err(why) if why.kind() == ErrorKind::NotFound => {
                check()?;
                return std::fs::write(path, contents);
            }
            Err(why) => return Err(why),
        },
        Ok(_) => path.to_path_buf(),
        Err(why) if why.kind() == ErrorKind::NotFound => path.to_path_buf(),
        Err(why) => return Err(why),
    };
    let existing = match std::fs::metadata(&target) {
        Ok(metadata) => Some(metadata),
        Err(why) if why.kind() == ErrorKind::NotFound => None,
        Err(why) => return Err(why),
    };
    if existing.as_ref().is_some_and(has_other_names) {
        check()?;
        return std::fs::write(&target, contents);
    }
    let directory = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let (temporary, file) = match create_beside(&directory, &target) {
        Ok(created) => created,
        Err(why) if why.kind() == ErrorKind::PermissionDenied => {
            check()?;
            return std::fs::write(&target, contents);
        }
        Err(why) => return Err(why),
    };
    if let Some(existing) = &existing
        && !same_owner(existing, &file)
    {
        drop(file);
        let _ = std::fs::remove_file(&temporary);
        check()?;
        return std::fs::write(&target, contents);
    }
    let written = fill(file, contents, existing.as_ref()).and_then(|()| {
        check()?;
        std::fs::rename(&temporary, &target)
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written?;
    sync_directory(&directory);
    Ok(())
}

/// Tells the files [`create_beside`] makes apart within one process.
static NEXT_TEMPORARY: AtomicUsize = AtomicUsize::new(0);

/// Makes a new, empty file in DIRECTORY for the next contents of TARGET, named
/// after it, hidden, and never one that is there already.
fn create_beside(directory: &Path, target: &Path) -> std::io::Result<(PathBuf, File)> {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    loop {
        let number = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".{name}.{}-{number}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(why) if why.kind() == ErrorKind::AlreadyExists => continue,
            Err(why) => return Err(why),
        }
    }
}

/// Writes CONTENTS into FILE, gives it the permissions of the file it is to
/// replace, if there is one, and flushes it to the disk. Closed before it
/// returns: a file still open cannot be renamed over another on Windows.
fn fill(mut file: File, contents: &[u8], existing: Option<&Metadata>) -> std::io::Result<()> {
    file.write_all(contents)?;
    if let Some(existing) = existing {
        file.set_permissions(existing.permissions())?;
    }
    file.sync_all()
}

/// Answers whether the file METADATA describes has names besides this one.
#[cfg(unix)]
fn has_other_names(metadata: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() > 1
}

#[cfg(not(unix))]
fn has_other_names(_metadata: &Metadata) -> bool {
    false
}

/// Answers whether the new FILE has the owner and the group of the file
/// EXISTING describes, which it must have to take its place.
#[cfg(unix)]
fn same_owner(existing: &Metadata, file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;
    file.metadata()
        .is_ok_and(|new| new.uid() == existing.uid() && new.gid() == existing.gid())
}

#[cfg(not(unix))]
fn same_owner(_existing: &Metadata, _file: &File) -> bool {
    true
}

/// Flushes DIRECTORY's own entries, so that the rename is on the disk too.
/// Best effort: where a directory cannot be opened, as on Windows, the rename
/// is still done.
fn sync_directory(directory: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = File::open(directory) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = directory;
}

// Typed buffer attachments, shared with rendering without depending on EditorState.
pub(crate) const IO_ERROR: &str = "file-io:error";
pub(crate) const IO_MARK: &str = "file-io:pending";
#[derive(Clone)]
pub(crate) struct IoMark {
    pub id: u64,
    pub label: String,
    pub slow: bool,
}
impl<B: super::BufferTrait> super::Buffer<B> {
    pub(crate) fn file_io_status(&self) -> String {
        let buf = self;
        let mut status = String::new();
        if let Some(marks) = buf.data::<Vec<IoMark>>(IO_MARK)
            && !marks.is_empty()
        {
            for mark in marks {
                status.push_str(&format!(
                    " [{}{}]",
                    mark.label,
                    if mark.slow { "?" } else { "" }
                ));
            }
        } else if buf.data::<String>(IO_ERROR).is_some() {
            status.push_str(" [File unavailable]");
        } else if buf.stale {
            status.push_str(" [File conflict]");
        }
        if buf.auto_save_failing {
            status.push_str(" [Auto-save failed]");
        }
        status
    }
}
