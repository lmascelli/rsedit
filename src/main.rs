mod tui;
use rsedit_core::{
    AppDir, DirOptions, ELispExp, EditorState, GapBuffer, app_dir, create_global_env, eval,
};
use std::io::{self, IsTerminal};
use std::time::Duration;
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

/// Shuts the editor's background work down when it goes: when `main` returns,
/// and when a panic unwinds through it. Either way every job is told, every
/// dedicated thread is asked to stop -- a shell command does not outlive the
/// editor -- and the log's last lines reach the file, which a panic is what
/// they are most likely to be about.
struct ShutDownOnExit<'a>(&'a EditorState<BufferType>);

impl Drop for ShutDownOnExit<'_> {
    fn drop(&mut self) {
        self.0.shutdown(SHUTDOWN_WAIT);
    }
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    let (state, env) = match create_global_env::<BufferType>() {
        Ok((state, env)) => (state, env),
        Err(err) => {
            eprintln!("{:?}", err);
            return Ok(());
        }
    };
    let _shut_down = ShutDownOnExit(&state);
    // The log goes in the cache directory, made if it is not there yet --
    // no longer in whatever directory the editor was started from, where it
    // left a file in every project and, in one that could not be written,
    // stopped the editor from starting at all.
    //
    // TODO: a cache directory that cannot be had, or a log file that cannot
    // be written in it, is ignored for now: the editor runs without a log
    // file, and nothing says so. It should say so where the user will see it.
    if let Ok(cache) = app_dir(DirOptions {
        dir: AppDir::Cache,
        create: true,
    }) {
        let _ = state.enable_log_file(cache.join(LOG_FILE));
    }

    if let Some(path) = args.get(1).cloned() {
        let ast = ELispExp::form(vec![
            ELispExp::symbol("find-file".into()),
            ELispExp::string(path),
        ]);
        if let Err(e) = eval(&ast, env.clone(), &state) {
            state.set_echo_message(&format!(
                "Boot Error: {:?}{}",
                e,
                state.take_backtrace_suffix()
            ));
        }
    }

    if io::stdin().is_terminal() {
        tui_main(&state, env.clone())
    } else {
        Ok(())
    }
}
