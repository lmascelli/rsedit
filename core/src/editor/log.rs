//! Diagnostics: the log and its file, the backtrace of the last error, and how
//! an uncaught error reaches the user.
//!
//! Design: `doc/architettura-editor.typ, <coppie>`.

use super::*;
use std::fs::{OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// Opens PATH for this editor alone: locked, emptied, and written at its end;
/// `None` when another editor holds it.
///
/// The lock is the operating system's, so it goes with the process, a crash
/// included, and the next editor to start takes the file again. Opened before
/// it is emptied: emptying it first was what cut short the log of an editor
/// still running. Where the file system has no locks, the file is used as it
/// was before there were any.
fn open_alone(path: &Path) -> std::io::Result<Option<File>> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(why)) if why.kind() == std::io::ErrorKind::Unsupported => {}
        Err(TryLockError::Error(why)) => return Err(why),
    }
    file.set_len(0)?;
    Ok(Some(file))
}

/// Returns the file a second editor logs to when PATH is taken: PATH with
/// this process's id before its extension, `rsedit-4242.log`.
fn own_log_path(path: &Path, pid: u32) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    let name = match path.extension() {
        Some(extension) => format!("{stem}-{pid}.{}", extension.to_string_lossy()),
        None => format!("{stem}-{pid}"),
    };
    path.with_file_name(name)
}

impl<B: BufferTrait> EditorState<B> {
    /// Starts copying the log to a file at PATH, emptied, first writing out
    /// what has been logged so far, and returns where it went. What is logged
    /// afterwards reaches the file through the WorkManager's watchdog, a turn
    /// at a time, and [`Self::shutdown`].
    ///
    /// One editor to a file: PATH is locked while this editor runs. When
    /// another editor has it, this one logs to PATH with its process id in the
    /// name, and says so in the log; two editors in one file overwrote each
    /// other's lines, and the second emptied the first's.
    ///
    /// The file is under the log's lock, so every clone of the editor logs to
    /// it, whichever enabled it.
    pub fn enable_log_file<P: AsRef<Path>>(&self, path: P) -> std::io::Result<PathBuf> {
        let path = path.as_ref();
        let (file, used) = match open_alone(path)? {
            Some(file) => (file, path.to_path_buf()),
            None => {
                let own = own_log_path(path, std::process::id());
                match open_alone(&own)? {
                    Some(file) => (file, own),
                    None => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::WouldBlock,
                            format!("{} and {} are in use", path.display(), own.display()),
                        ));
                    }
                }
            }
        };
        self.log_mut(|log| log.enable_file(file))?;
        if used != path {
            self.log_diagnostic(&format!(
                "[LOG] {} is in use by another rsedit: this one logs to {}",
                path.display(),
                used.display()
            ));
        }
        Ok(used)
    }

    /// Starts the log file NAME in the directory CACHE, which `app_dir` found
    /// or could not, and returns where it went; or says why there is none, in
    /// the log and in the echo area, and returns `None`.
    ///
    /// Said, because an editor running with its log in memory alone looks
    /// like one with a file, until somebody looks for the file after a crash.
    /// The echo area only when it is free, empty or still greeting: what
    /// `init.lisp` said there, an error, comes first.
    pub fn start_log_file(
        &self,
        cache: Result<PathBuf, crate::paths::DirError>,
        name: &str,
    ) -> Option<PathBuf> {
        let why = match cache {
            Ok(cache) => {
                let path = cache.join(name);
                match self.enable_log_file(&path) {
                    Ok(used) => return Some(used),
                    Err(why) => format!("{}: {why}", path.display()),
                }
            }
            Err(why) => why.to_string(),
        };
        let said = format!("No log file, the log is in memory only (C-h e): {why}");
        self.log_diagnostic(&format!("[WARNING] {said}"));
        let echo = self.get_echo_message();
        if echo.is_empty() || echo == super::boot::WELCOME {
            self.set_echo_message(&said);
        }
        None
    }

    /// Returns every line logged so far, oldest first.
    pub fn get_logs(&self) -> Vec<String> {
        self.log(|log| log.lines())
    }

    /// Returns the backtrace of the last uncaught error as a suffix,
    /// `" | backtrace: a -> b -> c"`, innermost call first, or an empty string
    /// when there is none; and clears it either way, so that the next error
    /// starts from a clean stack.
    pub fn take_backtrace_suffix(&self) -> String {
        let frames = self.backtrace();
        self.clear_backtrace();
        if frames.is_empty() {
            String::new()
        } else {
            format!(" | backtrace: {}", frames.join(" -> "))
        }
    }

    /// Reports an uncaught evaluation error to the user, and counts it as a
    /// failed command.
    ///
    /// When Lisp defines `report-error`, as `debug.lisp` does, it is called
    /// with MESSAGE and the backtrace, innermost call first, and decides how to
    /// show them: the one shipped echoes the message, and opens a
    /// `*Backtrace*` window when `debug-on-error` is set. Before it exists,
    /// early in startup, the error is logged, with the suffix
    /// [`Self::take_backtrace_suffix`] gives.
    pub fn report_error(&self, message: &str, env: &Arc<Env<Self>>) {
        // Counted first, so that a `report-error` that fails itself cannot keep
        // a replaying macro from noticing the failure.
        self.note_command_error();
        let frames = self.backtrace();
        self.clear_backtrace();

        if env.get_function("report-error").is_some() {
            let call_ast = ELispExp::form(vec![
                ELispExp::symbol("report-error".into()),
                ELispExp::string(message.to_string()),
                ELispExp::form(vec![
                    ELispExp::symbol("quote".into()),
                    ELispExp::proper_list(frames.into_iter().map(ELispExp::string).collect()),
                ]),
            ]);
            let _command = self.begin_command();
            if let Err(e) = eval(&call_ast, env.clone(), self) {
                self.log_diagnostic(&format!("[ERROR] report-error hook itself failed: {:?}", e));
            }
        } else {
            let suffix = if frames.is_empty() {
                String::new()
            } else {
                format!(" | backtrace: {}", frames.join(" -> "))
            };
            self.log_diagnostic(&format!("Eval Error: {message}{suffix}"));
        }
    }
}
