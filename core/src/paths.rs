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

/// Returns PATH as an absolute, normalised path, read in the directory the
/// editor was started in: for a setting, such as `auto-save-directory`, whose
/// meaning must not move with `cd`. A name the user types is read in the
/// working directory instead, by [`EditorState::expand_path`].
pub(crate) fn expand_path_from_start(path: &str) -> String {
    expand_path_in(path, &starting_directory())
}

/// Returns PATH made absolute against BASE rather than against the working
/// directory.
///
/// What `expand-file-name`'s second argument is for: a relative name typed into
/// a directory listing means "in the directory being listed", which is not
/// where the editor happens to have been started.
pub(crate) fn expand_path_in(path: &str, base: &std::path::Path) -> String {
    // The base is made absolute too, which is what lets everything below
    // assume it is working on an absolute path. A relative base would hand
    // back a relative answer -- from a function whose whole purpose is that
    // what comes out is absolute -- and the caller would be no better off than
    // before.
    let base = if base.is_absolute() {
        base.to_path_buf()
    } else {
        starting_directory().join(base)
    };
    let expanded = expand_tilde(path);
    let joined = if std::path::Path::new(&expanded).is_absolute() {
        std::path::PathBuf::from(expanded)
    } else {
        base.join(expanded)
    };
    normalise(&joined)
}

/// Returns the directory the editor is running in, or the root if even that
/// cannot be had.
///
/// A failure here means the working directory was deleted out from under the
/// process. The root is a poor answer but a usable one, and better than
/// refusing to expand anything for the rest of the session.
pub(crate) fn starting_directory() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from(std::path::MAIN_SEPARATOR_STR))
}

/// Expands a leading `~`, leaving anything else alone.
///
/// Only `~` and `~/...`, deliberately: `~other-user` needs the password
/// database, which is a platform question this editor has no business
/// answering. A path that cannot be expanded is returned unchanged rather than
/// rejected -- a file really can be called `~weird`, and failing to open it
/// would be worse than trying.
fn expand_tilde(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_string();
    };
    if !(rest.is_empty() || rest.starts_with(std::path::is_separator)) {
        return path.to_string();
    }
    // `USERPROFILE` as well as `HOME`, because Windows sets that one and
    // frequently not the other -- and a `~` that silently stayed a `~` would
    // make every path built from it name a directory called `~`.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    match home {
        Some(home) => format!("{home}{rest}"),
        None => path.to_string(),
    }
}

/// Collapses `.` and `..` without touching the filesystem.
///
/// Walks `std::path::Component`s rather than splitting on a character, which is
/// what makes this right on Windows: a `Prefix` (`C:`) and a `RootDir` are
/// components the platform's own parser produced, so a drive letter is never
/// mistaken for a directory name and a backslash is recognised as a separator.
///
/// PATH is always absolute here -- its only caller makes it so -- which is what
/// lets the result be returned without a case for having nothing left. An
/// absolute path keeps its root through every step below: `..` refuses to
/// remove one, and that is the only thing that removes anything.
fn normalise(path: &std::path::Path) -> String {
    use std::path::Component;
    let mut kept: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            // `.` says nothing about where the path goes. Mostly redundant:
            // `Components` already drops these, except one at the very start of
            // a relative path -- and by here the path is absolute, so there is
            // no start for one to be at. Kept so that this function is correct
            // on its own terms rather than on a neighbour's.
            Component::CurDir => {}
            Component::ParentDir => match kept.last() {
                // The only thing `..` may remove is a name.
                Some(Component::Normal(_)) => {
                    kept.pop();
                }
                // Above a root there is nothing, so the root is its own parent
                // -- which is what stops `^` held down from producing a path
                // made of `..`.
                Some(_) => {}
                None => kept.push(component),
            },
            other => kept.push(other),
        }
    }
    let out: std::path::PathBuf = kept.iter().collect();
    out.to_string_lossy().to_string()
}

/// Returns PATH with a trailing separator, so that a name can be joined onto
/// it.
pub(crate) fn as_directory(path: &str) -> String {
    if path.ends_with(std::path::is_separator) {
        return path.to_string();
    }
    format!("{path}{}", std::path::MAIN_SEPARATOR)
}

/// Returns PATH without its trailing separator: the directory named as a
/// *file*, which is the form whose parent can be asked for.
///
/// A root keeps its separator: `/` without it is the empty string, and `C:\`
/// without it is `C:`, which names the drive's working directory rather than
/// its root.
pub(crate) fn without_trailing_separator(path: &str) -> String {
    let trimmed = path.trim_end_matches(std::path::is_separator);
    if trimmed.is_empty() || std::path::Path::new(trimmed).parent().is_none() {
        return path.to_string();
    }
    trimmed.to_string()
}

/// Returns the directory part of PATH -- everything up to and including its
/// last separator -- or nothing when PATH names no directory at all.
///
/// Lexical, like Emacs's `file-name-directory`: `"/a/b"` gives `"/a/"` and
/// `"/a/b/"` gives itself, because the question is about the *name*, not about
/// what is on disk.
pub(crate) fn directory_part(path: &str) -> Option<String> {
    let cut = path.rfind(std::path::is_separator)?;
    Some(path[..=cut].to_string())
}

/// Returns the last component of PATH: everything after its last separator.
pub(crate) fn last_component(path: &str) -> String {
    match path.rfind(std::path::is_separator) {
        Some(cut) => path[cut + 1..].to_string(),
        None => path.to_string(),
    }
}

/// Returns everything in DIRECTORY, sorted, with the platform's separator
/// after each subdirectory.
///
/// The trailing separator is the whole reason this is useful for completion:
/// it is what tells the caller -- and the user reading a cycled candidate --
/// that pressing Tab again will descend rather than confirm, and it is what
/// makes a completed directory name a legal prefix for the next component.
///
/// `.` and `..` are not listed. They are always there, they are never what
/// somebody is looking for, and they would sit at the top of every listing.
pub(crate) fn directory_entries(directory: &str) -> Result<Vec<String>, std::io::Error> {
    directory_entries_ordered(directory, false)
}

/// Returns the same, with directories gathered before files when DIRS_FIRST.
///
/// Ordered here rather than by the caller because the answer is already in
/// hand: `file_type` has been asked for anyway, to decide the trailing
/// separator. A caller sorting afterwards would work out what each entry is
/// all over again, at a `stat` and an evaluation apiece from Lisp, which on a
/// large tree is more than `find-file-recursive` can afford.
///
/// Reading the separator back off the name would be cheaper still and is
/// wrong: which character it is depends on the platform, and `dired--visit`
/// already declines to do it for that reason. The separator is there to be
/// read by a person.
pub(crate) fn directory_entries_ordered(
    directory: &str,
    dirs_first: bool,
) -> Result<Vec<String>, std::io::Error> {
    let mut names: Vec<(bool, String)> = Vec::new();
    for entry in std::fs::read_dir(expand_path_from_start(directory))? {
        let entry = entry?;
        let mut name = entry.file_name().to_string_lossy().to_string();
        // `file_type` rather than `metadata`, so a symlink to a directory is
        // reported as the link it is instead of failing when it dangles.
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            name.push(std::path::MAIN_SEPARATOR);
        }
        // `false` sorts before `true`, so the flag is "is this a file" when
        // directories are wanted first, and a constant when they are not.
        names.push((dirs_first && !is_dir, name));
    }
    names.sort();
    Ok(names.into_iter().map(|(_, name)| name).collect())
}

/// Splits a part-typed path into the directory to list and the name to match
/// inside it.
///
/// `"/etc/ho"` lists `/etc` looking for `ho`; `"/etc/"` lists `/etc` looking
/// for everything; `"ho"` lists the current directory. The split is on the last
/// separator rather than on `Path::parent`, because a path that ends in a
/// separator means "inside this directory" and `parent` would climb out of it.
pub(crate) fn split_for_completion(prefix: &str) -> (String, &str) {
    // Any separator the platform recognises, not the character `/`: on Windows
    // a path typed as `C:\\Users\\me\\doc` has no `/` in it at all, and
    // splitting on one would offer completions from the working directory for
    // every path the user typed.
    match prefix.rfind(std::path::is_separator) {
        // Keeping the separator matters at the root: the directory part of
        // `/us` is `/`, and dropping the slash would list the working
        // directory instead.
        Some(cut) => (prefix[..=cut].to_string(), &prefix[cut + 1..]),
        None => (String::new(), prefix),
    }
}

/// Returns the file names that could complete PREFIX, as whole paths ready to
/// be confirmed.
///
/// Whole paths rather than bare names because the minibuffer replaces its
/// entire contents with the candidate it is showing: handing back `hosts` for
/// `/etc/ho` would complete the prompt to `hosts` and open the wrong file.
///
/// Hidden files are offered only once the typed name starts with a dot, which
/// is the convention every shell uses and the reason a listing of a home
/// directory is readable at all.
pub(crate) fn file_completions(prefix: &str, base: &str) -> Vec<String> {
    let (directory, name) = split_for_completion(prefix);
    let listed = directory_entries(&expand_path_in(
        if directory.is_empty() { "." } else { &directory },
        std::path::Path::new(base),
    ));    
    listed
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.starts_with(name))
        .filter(|entry| name.starts_with('.') || !entry.starts_with('.'))
        .map(|entry| format!("{directory}{entry}"))
        .collect()
}
