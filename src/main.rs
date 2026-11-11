//! The editor's entry point: the command line, the editor built, its log
//! opened and its session read, the files asked for opened, and then a front
//! end, the terminal or a window.
//!
//! Design: `doc/architettura-editor.typ, <ciclo-vita>`.
mod gui;
mod tui;
use rsedit_core::{
    AppDir, DirOptions, ELispExp, EditorState, Env, GapBuffer, app_dir, create_global_env, eval,
};
use std::io::{self, IsTerminal};
use std::sync::Arc;
use std::time::Duration;
use std::{ffi::OsString, path::PathBuf};

use gui::gui_main;
use tui::tui_main;

#[cfg(test)]
mod tests;

type BufferType = GapBuffer;

/// The log's file, in the cache directory.
const LOG_FILE: &str = "rsedit.log";

/// How long the background work gets to finish once the editor is quitting:
/// enough for a job's `exit`, and for a shell command to die once killed, and
/// short enough that quitting never feels stuck.
const SHUTDOWN_WAIT: Duration = Duration::from_millis(500);

/// Shuts the editor's background work down when it is dropped, as `main`
/// returns or a panic unwinds through it: the session is written, unless a
/// panic is unwinding, every job is told, every dedicated thread is asked to
/// stop, the log's last lines reach its file, and the shell commands still
/// running are killed, so that none outlives the editor.
struct ShutDownOnExit<'a>(
    &'a EditorState<BufferType>,
    Arc<Env<EditorState<BufferType>>>,
);

impl Drop for ShutDownOnExit<'_> {
    fn drop(&mut self) {
        // A panic may have left a buffer half changed, and a second panic
        // here would abort before the log is written: the session of the
        // run before stays instead.
        if !std::thread::panicking() {
            self.0.save_session(
                &self.1,
                app_dir(DirOptions {
                    dir: AppDir::State,
                    create: true,
                }),
            );
        }
        self.0.shutdown(SHUTDOWN_WAIT);
    }
}

/// What the command line asks for.
#[derive(Debug, Default)]
struct Options {
    /// The files to open, in order.
    files: Vec<PathBuf>,
    /// `--gui`: a window, even from a terminal.
    gui: bool,
    /// `--font FILE`: the monospace font the window draws with.
    font: Option<PathBuf>,
    /// `--help` or `-h`.
    help: bool,
}

impl Options {
    /// Returns what ARGS ask for, the program's name left out. After `--`
    /// every argument is a file; before it, an unknown option is an error.
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        let mut positional = false;
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--") if !positional => positional = true,
                Some("--gui") if !positional => options.gui = true,
                Some("--help" | "-h") if !positional => options.help = true,
                Some("--font") if !positional => {
                    options.font = Some(args.next().ok_or("--font requires a font file")?.into());
                }
                Some(flag) if !positional && flag.starts_with('-') => {
                    return Err(format!("Unknown option: {flag}"));
                }
                _ => options.files.push(arg.into()),
            }
        }
        Ok(options)
    }
}

/// Runs the editor with what the command line asks for, until it quits.
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    if options.help {
        println!(
            "rsedit [--gui] [--font MONOSPACE.ttf] [--] [FILES...]\n\
            In the terminal; in a window with --gui, or when stdin is not a terminal.\n\
            M-x: commands; C-x C-f: open; C-x C-s: save; C-x C-c: quit.\n\
            In the window, on Linux and Windows: Ctrl+Shift+V pastes; Ctrl+V keeps its binding.\n\
            On macOS: Cmd+O/S/Q, Cmd+C/X/V and Cmd+Z are native shortcuts.\n\
            The manual, from inside the editor: C-h m guide."
        );
        return Ok(());
    }

    // An error in init.lisp lets the editor start, and says so in it: what
    // fails here leaves no editor, and the exit status says that it failed.
    let (state, env) = match create_global_env::<BufferType>() {
        Ok((state, env)) => (state, env),
        Err(err) => return Err(format!("rsedit could not start: {err:?}").into()),
    };
    let _shut_down = ShutDownOnExit(&state, env.clone());
    // The log, in the cache directory, made if it is not there yet; or, when
    // there can be none, the reason said in the log and the echo area.
    state.start_log_file(
        app_dir(DirOptions {
            dir: AppDir::Cache,
            create: true,
        }),
        LOG_FILE,
    );
    // Before the files are opened, so that each opens where it was left.
    state.load_session(
        &env,
        app_dir(DirOptions {
            dir: AppDir::State,
            create: false,
        }),
    );

    let mut next_file = ELispExp::nil();
    for file in options.files.into_iter().rev() {
        next_file = ELispExp::form(vec![
            ELispExp::symbol("find-file-async".into()),
            ELispExp::string(file.display().to_string()),
            ELispExp::form(vec![
                ELispExp::symbol("lambda".into()),
                ELispExp::proper_list(vec![ELispExp::symbol("opened".into())]),
                next_file,
            ]),
        ]);
    }
    if let Err(e) = eval(&next_file, env.clone(), &state) {
        state.set_echo_message(&format!(
            "Boot Error: {e:?}{}",
            state.take_backtrace_suffix()
        ));
    }

    let is_gui = !io::stdin().is_terminal() || options.gui;

    if !is_gui {
        tui_main(&state, env.clone())
    } else {
        gui_main(&state, env.clone(), options.font)
    }
}
