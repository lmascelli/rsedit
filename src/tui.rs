use crossterm::{cursor, execute, terminal, style::Print};
use crossterm::event::{read, Event, KeyCode, KeyModifiers};
use std::io::{stdout, Write};
use rsedit_core::lisp::{Env, eval};
use rsedit_core::editor::{create_env, EditorState, ELispExp};

pub fn render_screen(state: &EditorState) -> std::io::Result<()> {
    let mut stdout = stdout();
    execute!(stdout, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
    
    stdout.flush()
}

pub fn tui_main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut state, mut env) = create_env();
    terminal::enable_raw_mode()?;

    while state.running {
        render_screen(&state);

        if let Event::Key(key_event) = read()? {
            let command = match key_event.code {
                KeyCode::Char('q') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    ELispExp::List(vec![ELispExp::Symbol("quit".into())])
                }

                _ => ELispExp::List(vec![])
            };
                eval(&command, &mut env, &mut state);
        }
    }

    terminal::disable_raw_mode()?;
    Ok(())
}
