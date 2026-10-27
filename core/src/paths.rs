//! Where the editor keeps what it keeps on disk: its configuration, its data
//! and its cache, and where its own data was installed. [`app_dir`] is asked for a directory by what it is for, and answers
//! with where that is on this system, making it when asked to.
//!
//! Design: `doc/architettura-editor.typ, <directory>`.
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// The variable that names the configuration directory outright: the
/// directory holding `init.lisp`, with nothing joined onto it.
pub const CONFIG_DIR: &str = "RSEDIT_CONFIG_DIR";

/// The variable that names the cache directory outright.
pub const CACHE_DIR: &str = "RSEDIT_CACHE_DIR";

/// The variable that names the data directory outright.
pub const DATA_DIR: &str = "RSEDIT_DATA_DIR";

/// The freedesktop parent of every application's configuration, honoured on
/// Unix only: on Windows, `%APPDATA%` is where Windows programs look.
pub const XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";

/// The freedesktop parent of every application's cache, honoured on Unix.
pub const XDG_CACHE_HOME: &str = "XDG_CACHE_HOME";

/// The freedesktop parent of every application's data, honoured on Unix.
pub const XDG_DATA_HOME: &str = "XDG_DATA_HOME";

/// Which of the editor's directories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppDir {
    /// Where `init.lisp` is: what the person writes, and the editor reads at
    /// every start.
    Config,
    /// What the editor keeps that it can make again, such as its log.
    /// Deleting it loses nothing that matters.
    Cache,
    /// What is installed for the editor rather than written for it, such as
    /// its plugins.
    Data,
}

impl AppDir {
    /// Returns what it is called in a message.
    fn describe(self) -> &'static str {
        match self {
            AppDir::Config => "the configuration directory",
            AppDir::Cache => "the cache directory",
            AppDir::Data => "the data directory",
        }
    }

    /// Returns the variable that names it outright, and the freedesktop one.
    fn variables(self) -> (&'static str, &'static str) {
        match self {
            AppDir::Config => (CONFIG_DIR, XDG_CONFIG_HOME),
            AppDir::Cache => (CACHE_DIR, XDG_CACHE_HOME),
            AppDir::Data => (DATA_DIR, XDG_DATA_HOME),
        }
    }
}

/// What [`app_dir`] is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirOptions {
    pub dir: AppDir,
    /// Make it, with any parent that is missing, when it is not there.
    pub create: bool,
}

/// Why a directory could not be had.
#[derive(Debug)]
pub enum DirError {
    /// Nothing in the environment says where it is: no variable naming it,
    /// and no home to put it under -- `HOME` unset, or on Windows `APPDATA`,
    /// `LOCALAPPDATA` for the cache and the data.
    Undetermined(AppDir),
    /// It is not there, and making it was not asked for.
    NotFound(PathBuf),
    /// Something that is not a directory is where it should be.
    NotADirectory(PathBuf),
    /// It is not there, and it could not be made.
    CannotCreate {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for DirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DirError::Undetermined(dir) => write!(
                f,
                "nowhere for {}: nothing in the environment says where it goes",
                dir.describe()
            ),
            DirError::NotFound(path) => write!(f, "{} does not exist", path.display()),
            DirError::NotADirectory(path) => {
                write!(f, "{} is not a directory", path.display())
            }
            DirError::CannotCreate { path, source } => {
                write!(f, "{} could not be created: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for DirError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DirError::CannotCreate { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Returns the directory `options.dir` names on this system, creating it and
/// its parents first when it is missing and `options.create` is set.
///
/// Where each directory is: the `configuration` manual page.
///
/// # Errors
///
/// [`DirError::Undetermined`] when nothing in the environment says where the
/// directory is; [`DirError::NotFound`] when it is missing and `create` is
/// false; [`DirError::NotADirectory`] or [`DirError::CannotCreate`] otherwise.
///
/// # Examples
///
/// ```no_run
/// use rsedit_core::{AppDir, DirOptions, app_dir};
///
/// let cache = app_dir(DirOptions { dir: AppDir::Cache, create: true })?;
/// # Ok::<(), rsedit_core::DirError>(())
/// ```
pub fn app_dir(options: DirOptions) -> Result<PathBuf, DirError> {
    let path = locate(options.dir).ok_or(DirError::Undetermined(options.dir))?;
    ensure_dir(path, options.create)
}

/// Returns the directory the editor's own data was installed in -- its
/// modules, themes and manual pages -- or `None` when the executable cannot
/// be found.
///
/// `data` beside the executable, or one level up when only that one is
/// there; when neither is, the one beside it, which is where it should be.
pub(crate) fn installed_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    // One level up because a test binary lives in `target/<profile>/deps/`
    // while the data was put in `target/<profile>/data/` -- so without this
    // the editor's own pages are unreachable from every test that looks for
    // them, which is exactly the code that needs testing.
    let candidates = [parent.join("data"), parent.join("..").join("data")];
    let found = candidates.iter().find(|path| path.is_dir());
    Some(found.unwrap_or(&candidates[0]).clone())
}

/// Returns where DIR is, from the environment, whether or not it is there.
fn locate(dir: AppDir) -> Option<PathBuf> {
    let (named, xdg) = dir.variables();
    let named = env_path(named);
    #[cfg(unix)]
    let xdg = xdg_parent(std::env::var_os(xdg));
    // Set on Windows only by somebody carrying a Unix habit across, and
    // deliberately not acted on: the platform has its own answer below.
    #[cfg(not(unix))]
    let xdg: Option<PathBuf> = {
        let _ = xdg;
        None
    };
    choose_dir(named, xdg, platform_dir(dir))
}

/// Returns the platform's own place for DIR, already complete.
#[cfg(windows)]
pub(crate) fn platform_dir(dir: AppDir) -> Option<PathBuf> {
    match dir {
        AppDir::Config => env_path("APPDATA").map(|appdata| appdata.join("rsedit")),
        AppDir::Cache => env_path("LOCALAPPDATA").map(|local| local.join("rsedit").join("cache")),
        AppDir::Data => env_path("LOCALAPPDATA").map(|local| local.join("rsedit").join("data")),
    }
}

/// Returns the platform's own place for DIR, already complete.
#[cfg(not(windows))]
pub(crate) fn platform_dir(dir: AppDir) -> Option<PathBuf> {
    let home = env_path("HOME")?;
    Some(match dir {
        AppDir::Config => home.join(".config").join("rsedit"),
        AppDir::Cache => home.join(".cache").join("rsedit"),
        AppDir::Data => home.join(".local").join("share").join("rsedit"),
    })
}

/// Returns the variable NAME as a path, or `None` when it is not set or is
/// empty. Read as an `OsString`, not a `String`: a home directory whose name
/// is not UTF-8 is still a home directory.
fn env_path(name: &str) -> Option<PathBuf> {
    path_from(std::env::var_os(name))
}

/// Returns VALUE, a variable's, as a path: `None` when it is not set or is
/// empty.
pub(crate) fn path_from(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// Returns VALUE, a freedesktop variable's, as the parent it names: also
/// `None` when it is not absolute, which the specification calls invalid and
/// has ignored.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn xdg_parent(value: Option<OsString>) -> Option<PathBuf> {
    path_from(value).filter(|path| path.is_absolute())
}

/// Returns the candidate that wins: NAMED as given, since it names the
/// directory itself; else XDG with the application's name joined on, since
/// it is a parent of application directories; else PLATFORM, which is
/// already complete.
///
/// Takes its inputs rather than reading them, so that the precedence can be
/// tested without writing to the environment, which every test can see.
pub(crate) fn choose_dir(
    named: Option<PathBuf>,
    xdg: Option<PathBuf>,
    platform: Option<PathBuf>,
) -> Option<PathBuf> {
    named
        .or_else(|| xdg.map(|xdg| xdg.join("rsedit")))
        .or(platform)
}

/// Returns PATH, if it is a directory, creating it first when it is missing
/// and CREATE is set.
///
/// Apart from [`app_dir`] so that what is done on the disk can be tested on a
/// directory of the test's own, rather than on the person's.
pub(crate) fn ensure_dir(path: PathBuf, create: bool) -> Result<PathBuf, DirError> {
    // TODO(improve): whether the directory can be written is not checked. One
    // that is there but read-only is answered as usable, and the first write
    // into it fails -- which, for the log, the caller ignores.
    if path.is_dir() {
        return Ok(path);
    }
    if path.exists() {
        return Err(DirError::NotADirectory(path));
    }
    if !create {
        return Err(DirError::NotFound(path));
    }
    // Fine if somebody else makes it meanwhile: `create_dir_all` succeeds on a
    // directory that is already there.
    match std::fs::create_dir_all(&path) {
        Ok(()) => Ok(path),
        Err(source) => Err(DirError::CannotCreate { path, source }),
    }
}
