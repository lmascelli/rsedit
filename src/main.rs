mod tui;
use rsedit_core::{ELispExp, GapBuffer, create_global_env, eval};
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

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    let (state, env) = match create_global_env::<BufferType>() {
        Ok((state, env)) => (state, env),
        Err(err) => {
            eprintln!("{:?}", err);
            return Ok(());
        }
    };
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

    let ran = if io::stdin().is_terminal() {
        tui_main(&state, env.clone())
    } else {
        Ok(())
    };
    // However the loop ended, the background work stops before the process
    // does: every job is told, and every dedicated thread is asked to stop.
    state.shutdown(SHUTDOWN_WAIT);
    ran
}
