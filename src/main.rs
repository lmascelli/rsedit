mod tui;
use rsedit_core::{ELispExp, EditorState, GapBuffer, create_global_env, eval};
use std::io::{self, IsTerminal};
use std::time::Duration;
use tui::tui_main;

#[cfg(test)]
mod tests;

type BufferType = GapBuffer;

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
    state.enable_log_file("rsedit.log")?;

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
