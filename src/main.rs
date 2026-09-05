mod tui;
use rsedit_core::{ELispExp, buffer::gap_buffer::GapBuffer, create_global_env, lisp::eval};
use std::io::{self, IsTerminal};
use tui::tui_main;

type BufferType = GapBuffer;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    let (mut state, env) =
        create_global_env::<BufferType>().expect("Failed to create the editor environment");
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
        tui_main(&state, env.clone())?;
    } else {
    }

    Ok(())
}
