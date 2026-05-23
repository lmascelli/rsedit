use crossterm::{cursor, execute, terminal, style::Print};
use crossterm::event::{read, Event, KeyCode, KeyModifiers};
use std::io::{stdout, Write};
use rsedit_core::lisp::Env;
use rsedit_core::editor::EditorState;

pub fn render_screen(state: &EditorState) -> std::io::Result<()> {
    let mut stdout = stdout();
    execute!(stdout, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;

    stdout.flush()
}

pub fn tui_main() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = EditorState::new();
    let mut env = Env::<()>::new();

    terminal::enable_raw_mode()?;

    while state.running {
        render_screen(&state);

        if let Event::Key(key_event) = read()? {
            match key_event.code {
                KeyCode::Char('q') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.running = false;
                }

                _ => ()
            }
        }
    }

    terminal::disable_raw_mode()?;
    Ok(())
}
