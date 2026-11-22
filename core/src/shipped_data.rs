//! The editor's own data -- its Lisp modules, its themes and its manual pages
//! -- carried in the executable, and written out into the data directory
//! when what is there is not this data.
//!
//! The folders written out, [`PARTS`], belong to the editor: each is replaced
//! whole, whatever is in it, and the person's own modules and themes go in
//! the configuration directory instead, which comes first in `lisp-path` and
//! `theme-path`. The data directory's other folders, `plugins` among them,
//! are never touched.
//!
//! A stamp file, [`STAMP_FILE`], says which data the folders hold: the
//! version and a hash of every file, worked out by `build.rs`. At every start
//! only the stamp is read; when it says something else, or is not there, the
//! folders are written again. It is removed before anything is replaced and
//! written only once everything is, so a start stopped half-way leaves no
//! stamp, and the next start writes the folders again.
//!
//! Design: `doc/architettura-editor.typ, <dati-distribuiti>`.
use crate::paths::{AppDir, DirOptions, app_dir};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/shipped_data.rs"));
}

pub(crate) use generated::{FILES, STAMP};

/// The folders of the data directory the editor writes out and replaces:
/// every file in [`FILES`] is in one of them.
pub(crate) const PARTS: [&str; 3] = ["lisp", "themes", "man"];

/// The file in the data directory that says which data its [`PARTS`] hold.
pub(crate) const STAMP_FILE: &str = ".shipped-data";

/// The beginning of the name of a folder data is written into before it
/// takes the place of the one in use.
const STAGING_PREFIX: &str = ".staging-";

/// The beginning of the name of a folder the one in use is moved into before
/// it is deleted.
const OLD_PREFIX: &str = ".old-";

/// How old a staging or old folder must be to be taken for one a start
/// stopped half-way left behind, rather than one another editor is using.
const LEFTOVER_AGE: Duration = Duration::from_secs(60 * 60);

/// Held while the data is written out: two editors of one process -- the
/// tests make many -- write it one after the other, and the second finds it
/// written.
static WRITING: Mutex<()> = Mutex::new(());

/// Tells this process's staging and old folders apart.
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Returns whether DIR holds this executable's data, as its stamp says.
pub(crate) fn is_current(dir: &Path) -> bool {
    fs::read_to_string(dir.join(STAMP_FILE)).is_ok_and(|stamp| stamp == STAMP)
}

/// Writes this executable's data out into DIR unless it is there already,
/// and answers whether it wrote it.
///
/// # Errors
///
/// What the file system answered when the data could not be written; the
/// folders are then as they were, or without a stamp.
pub(crate) fn ensure_in(dir: &Path) -> io::Result<bool> {
    let _writing = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if is_current(dir) {
        return Ok(false);
    }
    write_out(dir)?;
    Ok(true)
}

/// Writes this executable's data out into DIR, whatever is there: what
/// `--restore-data` does.
///
/// # Errors
///
/// As [`ensure_in`].
pub(crate) fn restore_in(dir: &Path) -> io::Result<()> {
    let _writing = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write_out(dir)
}

/// Writes this executable's data out into the data directory, made if it is
/// missing, whatever is there, and returns where: for `--restore-data`, when
/// the data written out is damaged, without starting an editor.
///
/// # Errors
///
/// A message saying why there is no data directory, or why the data could
/// not be written into it.
pub fn restore_data() -> Result<PathBuf, String> {
    let dir = app_dir(DirOptions {
        dir: AppDir::Data,
        create: true,
    })
    .map_err(|why| why.to_string())?;
    restore_in(&dir).map_err(|why| format!("{}: {why}", dir.display()))?;
    Ok(dir)
}

/// Writes the data into a staging folder of DIR, moves it into place one
/// part at a time, and writes the stamp last.
fn write_out(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    remove_leftovers(dir);
    let unique = format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let staging = dir.join(format!("{STAGING_PREFIX}{unique}"));
    let old = dir.join(format!("{OLD_PREFIX}{unique}"));
    let written = stage(&staging).and_then(|()| {
        // From here the parts in use may stop matching the stamp.
        match fs::remove_file(dir.join(STAMP_FILE)) {
            Err(why) if why.kind() != io::ErrorKind::NotFound => return Err(why),
            _ => {}
        }
        fs::create_dir_all(&old)?;
        for part in PARTS {
            swap_in(&staging.join(part), &dir.join(part), &old.join(part))?;
        }
        let stamp = staging.join(STAMP_FILE);
        fs::write(&stamp, STAMP)?;
        fs::rename(&stamp, dir.join(STAMP_FILE))
    });
    // What is left of the two is no longer needed, whatever happened: a
    // folder that cannot be deleted now is deleted by a later start.
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_dir_all(&old);
    written
}

/// Writes every file of [`FILES`] under STAGING, with a folder for each of
/// [`PARTS`] even when nothing is in it, so that a part a later version no
/// longer ships replaces the one there with nothing.
fn stage(staging: &Path) -> io::Result<()> {
    for part in PARTS {
        fs::create_dir_all(staging.join(part))?;
    }
    for (name, bytes) in FILES {
        let mut path = staging.to_path_buf();
        path.extend(name.split('/'));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, bytes)?;
    }
    Ok(())
}

/// Puts STAGED where LIVE is, the one there moved to OLD first.
///
/// Tried again when another editor, starting at the same moment, put its own
/// in place in between: both write the same data, so either is right.
fn swap_in(staged: &Path, live: &Path, old: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        let aside = old.with_extension(attempt.to_string());
        match fs::rename(live, &aside) {
            Err(why) if why.kind() != io::ErrorKind::NotFound => return Err(why),
            _ => {}
        }
        match fs::rename(staged, live) {
            Ok(()) => return Ok(()),
            Err(_) if attempt < 3 && fs::symlink_metadata(live).is_ok() => attempt += 1,
            Err(why) => return Err(why),
        }
    }
}

/// Deletes the staging and old folders of DIR that a start stopped half-way
/// left behind: those older than [`LEFTOVER_AGE`], since a younger one may be
/// another editor's, starting now.
fn remove_leftovers(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(STAGING_PREFIX) || name.starts_with(OLD_PREFIX)) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > LEFTOVER_AGE);
        if old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}
