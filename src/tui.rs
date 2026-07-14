use crossterm::event::{Event, KeyCode as CrossKeyCode, KeyModifiers as CrossModifiers, read};
use crossterm::{QueueableCommand, cursor, execute, style::Print, terminal};
use rsedit_core::buffer::BufferTrait;
use rsedit_core::editor::EditorState;
use rsedit_core::input::{KeyCode, KeyEvent, KeyModifiers};
use rsedit_core::lisp::Env;
use rsedit_core::ui::Rect;
use std::{
    io::{Write, stdout},
    sync::Arc,
};

pub fn render_screen<B: BufferTrait>(
    state: &mut EditorState<B>,
    cols: u16,
    rows: u16,
) -> std::io::Result<()> {
    let mut stdout = stdout();

    let views = state.compose_layout(cols as usize, rows as usize);

    execute!(
        stdout,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    for view in views {
        if view.has_border {
            draw_window_border(&mut stdout, &view.rect, &Some(view.buffer_name))?;
        }

        for (offset_y, line) in view.lines.iter().enumerate() {
            let target_y = view.rect.y + offset_y;
            if target_y >= rows as usize {
                break;
            }

            stdout.queue(cursor::MoveTo(view.rect.x as u16, target_y as u16))?;
            stdout.queue(Print(line))?;

            if view.is_focused {
                if let Some((cx, cy)) = view.cursor_rel_pos {
                    let absolute_cx = (view.rect.x + cx) as u16;
                    let absolute_cy = (view.rect.y + cy) as u16;
                    stdout.queue(cursor::MoveTo(absolute_cx, absolute_cy))?;
                }
            }
        }
    }

    stdout.flush()
}

fn draw_window_border(
    _stdout: &mut std::io::Stdout,
    _rect: &Rect,
    _title: &Option<String>,
) -> std::io::Result<()> {
    Ok(())
}

pub fn tui_main<B: BufferTrait>(
    state: &mut EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal::enable_raw_mode()?;

    while state.is_running() {
        let (cols, rows) = terminal::size()?;
        render_screen(state, cols, rows)?;

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
            state.handle_key_event(event, &env);
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
