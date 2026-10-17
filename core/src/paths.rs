//! Where the editor keeps what it keeps on disk: its configuration, its cache.
//!
//! One function, [`app_dir`], asked for a directory by what it is for and
//! answering with where that is on the system the editor runs on -- and, when
//! asked to, making it. Everything that needs one of these places asks here,
//! so that the conventions of each system are written down once.
//!
//! Where each directory is, and the variables that choose it, is user
//! documentation: the `configuration` manual page.
//!
//! # Why macOS gets `~/.config` and `~/.cache`
//!
//! Because that is where somebody using a terminal editor on a Mac will look.
//! Apple's `~/Library` is the convention for bundled applications; the editors
//! a person arrives from -- Emacs under XDG, Neovim, Helix -- all keep their
//! configuration in `~/.config` and their caches in `~/.cache`, and being the
//! one that does not would be a correctness nobody asked for.
//!
//! # Why the cache is local on Windows
//!
//! `%APPDATA%` roams: it follows the person from machine to machine, and is
//! copied each time they log on. A cache is what can be made again, which is
//! exactly what must not be copied around, so it goes under `%LOCALAPPDATA%`.
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// The name of the variable that names the configuration directory outright:
/// the directory holding `init.lisp`, with nothing joined onto it.
pub const CONFIG_DIR: &str = "RSEDIT_CONFIG_DIR";

/// The name of the variable that names the cache directory outright.
pub const CACHE_DIR: &str = "RSEDIT_CACHE_DIR";

/// The freedesktop parent of every application's configuration, honoured on
/// Unix. Windows has `%APPDATA%`, and preferring somebody else's answer to the
/// platform's own would put a Windows user's configuration somewhere no
/// Windows program would look for it.
pub const XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";

/// The freedesktop parent of every application's cache, honoured on Unix.
pub const XDG_CACHE_HOME: &str = "XDG_CACHE_HOME";

/// Which of the editor's directories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppDir {
    /// Where `init.lisp` is: what the person writes, and the editor reads at
    /// every start.
    Config,
    /// What the editor keeps that it can make again: the log, and the
    /// documentation it collects. Deleting it loses nothing that matters.
    Cache,
}

impl AppDir {
    /// What it is called in a message.
    fn describe(self) -> &'static str {
        match self {
            AppDir::Config => "the configuration directory",
            AppDir::Cache => "the cache directory",
        }
    }

    /// The variable that names it outright, and the freedesktop one.
    fn variables(self) -> (&'static str, &'static str) {
        match self {
            AppDir::Config => (CONFIG_DIR, XDG_CONFIG_HOME),
            AppDir::Cache => (CACHE_DIR, XDG_CACHE_HOME),
        }
    }
}

/// What [`app_dir`] is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirOptions {
    /// Which directory.
    pub dir: AppDir,
    /// Make it, with any parent that is missing, when it is not there.
    pub create: bool,
}

/// Why a directory could not be had.
#[derive(Debug)]
pub enum DirError {
    /// Nothing in the environment says where it is: no variable naming it,
    /// and no home to put it under -- `HOME` unset, or `APPDATA` on Windows.
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

/// The directory OPTIONS asks for, where this system keeps it -- made first,
/// when it is missing and OPTIONS says to. Where that is: the `configuration`
/// manual page.
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

/// Where DIR is, from the environment, whether or not it is there.
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

/// The platform's own place for DIR, already complete.
#[cfg(windows)]
pub(crate) fn platform_dir(dir: AppDir) -> Option<PathBuf> {
    match dir {
        AppDir::Config => env_path("APPDATA").map(|appdata| appdata.join("rsedit")),
        AppDir::Cache => env_path("LOCALAPPDATA").map(|local| local.join("rsedit").join("cache")),
    }
}

/// The platform's own place for DIR, already complete.
#[cfg(not(windows))]
pub(crate) fn platform_dir(dir: AppDir) -> Option<PathBuf> {
    let home = env_path("HOME")?;
    Some(match dir {
        AppDir::Config => home.join(".config").join("rsedit"),
        AppDir::Cache => home.join(".cache").join("rsedit"),
    })
}

/// The variable NAME as a path, or `None` when it is not set or is empty.
///
/// As an `OsString`, not a `String`: a home directory whose name is not UTF-8
/// is still a home directory.
fn env_path(name: &str) -> Option<PathBuf> {
    path_from(std::env::var_os(name))
}

/// VALUE, a variable's, as a path: `None` when it is not set or is empty.
pub(crate) fn path_from(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// VALUE, a freedesktop variable's, as the parent it names: also `None` when
/// it is not absolute -- the specification's own rule, which calls such a
/// path invalid and has it ignored.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn xdg_parent(value: Option<OsString>) -> Option<PathBuf> {
    path_from(value).filter(|path| path.is_absolute())
}

/// Which candidate wins, given what the environment says.
///
/// # Why this takes its inputs rather than reading them
///
/// So that the *precedence* can be tested without touching the environment.
/// The environment is process-wide and the test runner is threaded, so a test
/// that checked this by setting a variable would be a test every other test
/// could see -- which is the shape of the bug this was last fixed for, not a
/// way to check it.
///
/// `named` is used as given: it names the directory itself. `xdg` is a
/// *parent* of application directories by specification, so the application's
/// own name is joined onto it. `platform` is whatever the running system calls
/// the same place, already complete.
pub(crate) fn choose_dir(
    named: Option<PathBuf>,
    xdg: Option<PathBuf>,
    platform: Option<PathBuf>,
) -> Option<PathBuf> {
    named
        .or_else(|| xdg.map(|xdg| xdg.join("rsedit")))
        .or(platform)
}

/// PATH, if it is a directory; made first, when it is missing and CREATE says
/// to.
///
/// Apart from [`app_dir`] so that what is done on the disk can be tested on a
/// directory of the test's own, rather than on the person's.
pub(crate) fn ensure_dir(path: PathBuf, create: bool) -> Result<PathBuf, DirError> {
    // TODO: whether the directory can be written is not checked. One that is
    // there but read-only is answered as usable, and it is the first write
    // into it that fails -- which, for the log, the caller ignores for now.
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
