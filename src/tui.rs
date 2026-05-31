use crossterm::event::{Event, KeyCode as CrossKeyCode, KeyModifiers as CrossModifiers, read};
use crossterm::{QueueableCommand, cursor, execute, style::Print, terminal};
use rsedit_core::buffer::BufferTrait;
use rsedit_core::editor::{ELispExp, EditorState, create_global_env};
use rsedit_core::gap_buffer::GapBuffer;
use rsedit_core::input::{KeyCode, KeyEvent, KeyModifiers};
use rsedit_core::lisp::eval;
use std::io::{Write, stdout};

type BufferType = GapBuffer;

pub fn render_screen<B: BufferTrait>(state: &EditorState<B>, cols: u16, rows: u16) -> std::io::Result<()> {
    let mut stdout = stdout();
    execute!(
        stdout,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    let text_area_rows = if rows > 1 { (rows - 1) as usize } else { 0 };
    let buf = state.current_buffer();
    let text = buf.text.to_string();
    let (cursor_line, cursor_col) = buf.text.cursor_pos();

    for (i, line) in text
        .split('\n')
        .skip(buf.scroll_y)
        .take(text_area_rows)
        .enumerate()
    {
        stdout.queue(cursor::MoveTo(0, i as u16))?;

        let display_line: String = line
            .chars()
            .skip(buf.scroll_x)
            .take(cols as usize)
            .collect();
        stdout.queue(Print(display_line))?;
    }

    let echo_row = if rows > 0 { rows - 1 } else { 0 };
    stdout.queue(cursor::MoveTo(0, echo_row))?;
    stdout.queue(Print(&state.echo_message))?;

    // CURSOR CALCULATION
    let screen_cursor_y = cursor_line.saturating_sub(buf.scroll_y) as u16;
    let screen_cursor_x = cursor_col.saturating_sub(buf.scroll_x) as u16;

    stdout.queue(cursor::MoveTo(screen_cursor_x, screen_cursor_y))?;

    stdout.flush()
}

pub fn tui_main(file_to_open: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let (mut state, mut env) = create_global_env::<BufferType>();

    if let Some(path) = file_to_open {
        let ast = ELispExp::List(vec![
            ELispExp::Symbol("find-file".into()),
            ELispExp::String(path),
        ]);
        if let Err(e) = eval(&ast, &mut env, &mut state) {
            state.echo_message = format!("Boot Error: {:?}", e);
        }
    }

    terminal::enable_raw_mode()?;

    while state.running {
        let (cols, rows) = terminal::size()?;
        state.adjust_scroll(cols as usize, rows as usize);
        render_screen(&state, cols, rows)?;

        if let Event::Key(key_event) = read()? {
            let mut event = KeyEvent {
                code: KeyCode::None,
                modifiers: KeyModifiers {
                    ctrl: key_event.modifiers.contains(CrossModifiers::CONTROL),
                    alt: key_event.modifiers.contains(CrossModifiers::ALT),
                    shift: key_event.modifiers.contains(CrossModifiers::SHIFT),
                    caps_lock_as_ctrl: false,
                },
            };

            event.code = match key_event.code {
                CrossKeyCode::Char(c) if key_event.modifiers.contains(CrossModifiers::SHIFT) => {
                    event.modifiers.shift = false;
                    KeyCode::Char(c.to_ascii_uppercase())
                }
                CrossKeyCode::Char(c) => KeyCode::Char(c),
                CrossKeyCode::Left => KeyCode::Left,
                CrossKeyCode::Right => KeyCode::Right,
                CrossKeyCode::Up => KeyCode::Up,
                CrossKeyCode::Down => KeyCode::Down,
                CrossKeyCode::Backspace => KeyCode::Backspace,
                CrossKeyCode::Enter => KeyCode::Enter,
                _ => continue,
            };
            state.handle_key_event(event, &mut env);
        }
    }

    execute!(
        stdout(),
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;
    terminal::disable_raw_mode()?;
    Ok(())
}
