use crossterm::event::{Event, KeyCode as CrossKeyCode, KeyModifiers as CrossModifiers, read};
use crossterm::{QueueableCommand, cursor, execute, style::Print, terminal};
use rsedit_core::ELispExp;
use rsedit_core::buffer::BufferTrait;
use rsedit_core::editor::{EditorState};
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
    stdout: &mut std::io::Stdout,
    rect: &Rect,
    title: &Option<String>,
) -> std::io::Result<()> {
    // TODO ensure that the rect.x and rect.y values start from 1
    let top_border = if let Some(title) = title {
        format!(
            "─{}{}",
            title,
            vec!['─'; rect.width - title.len() - 1]
                .iter()
                .collect::<String>()
        )
    } else {
        vec!['─'; rect.width].iter().collect::<String>()
    };
    stdout.queue(cursor::MoveTo((rect.x - 1) as u16, (rect.y - 1) as u16))?;
    stdout.queue(Print(format!("┌{}┐", top_border)))?;
    for r in (rect.y)..(rect.y + rect.height) {
        stdout.queue(cursor::MoveTo((rect.x - 1) as u16, r as u16))?;
        stdout.queue(Print('│'))?;
        stdout.queue(cursor::MoveTo((rect.x + rect.width) as u16, r as u16))?;
        stdout.queue(Print('│'))?;
    }
    stdout.queue(cursor::MoveTo(
        (rect.x - 1) as u16,
        (rect.y + rect.height) as u16,
    ))?;
    stdout.queue(Print(format!(
        "└{}┘",
        vec!['─'; rect.width].iter().collect::<String>()
    )))?;
    Ok(())
}

pub fn tui_main<B: BufferTrait>(
    state: &mut EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal::enable_raw_mode()?;
    // Tell the editor the starting frame size
    {
        let (cols, rows) = terminal::size()?;
        env.set_variable("frame-width".into(), ELispExp::number(cols as f64));
        env.set_variable("frame-height".into(), ELispExp::number(rows as f64));
    }

    while state.is_running() {
        let (cols, rows) = terminal::size()?;
        render_screen(state, cols, rows)?;

         match read()? {
             Event::Key(key_event) => {
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

             Event::Resize(width, height) => {
                 state.resize(env.clone(), width as usize, height as usize);

             }

             _ => todo!()
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
