use crossterm::{cursor, execute, terminal, style::Print, QueueableCommand};
use crossterm::event::{read, Event, KeyCode as CrossKeyCode, KeyModifiers as CrossModifiers};
use std::io::{stdout, Write};
use rsedit_core::editor::{create_global_env, EditorState};
use rsedit_core::input::{KeyEvent, KeyCode, KeyModifiers};

pub fn render_screen(state: &EditorState) -> std::io::Result<()> {
    let mut stdout = stdout();
    execute!(stdout, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
    
    let text = state.current_buffer().text.to_string();
    let mut line_y = 0;
    for line in text.lines() {
        stdout.queue(cursor::MoveTo(0, line_y))?;
        stdout.queue(Print(line))?;
        line_y += 1;
    }

    let (_, rows) = terminal::size()?;
    let echo_row = if rows > 0 { rows - 1 } else { 0 };
    stdout.queue(cursor::MoveTo(0, echo_row))?;
    stdout.queue(Print(&state.echo_message))?;

    let cursor_x = text.chars().count() as u16;
    stdout.queue(cursor::MoveTo(cursor_x, 0))?;

    stdout.flush()
}

pub fn tui_main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut state, mut env) = create_global_env();
    terminal::enable_raw_mode()?;

    while state.running {
        render_screen(&state);

        if let Event::Key(key_event) = read()? {
            let mut event = KeyEvent {
                code: KeyCode::None,
                modifiers: KeyModifiers {
                    ctrl: key_event.modifiers.contains(CrossModifiers::CONTROL),
                    alt: key_event.modifiers.contains(CrossModifiers::ALT),
                    shift: key_event.modifiers.contains(CrossModifiers::SHIFT),
                    caps_lock_as_ctrl: false,
                }
            };

            event.code = match key_event.code {
                CrossKeyCode::Char(c) if key_event.modifiers.contains(CrossModifiers::SHIFT) => {
                    event.modifiers.shift = false;
                    KeyCode::Char(c.to_ascii_uppercase())
                },
                CrossKeyCode::Char(c) => KeyCode::Char(c),
                CrossKeyCode::Left => KeyCode::Left,
                CrossKeyCode::Right => KeyCode::Right,
                CrossKeyCode::Backspace => KeyCode::Backspace,
                CrossKeyCode::Enter => KeyCode::Enter,
                _ => continue,
            };
            state.handle_key_event(event, &mut env);
        }
    }

    execute!(stdout(), terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
    terminal::disable_raw_mode()?;
    Ok(())
}
