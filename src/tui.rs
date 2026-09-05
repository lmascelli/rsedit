use crossterm::event::{Event, KeyCode as CrossKeyCode, KeyModifiers as CrossModifiers, read};
use crossterm::{QueueableCommand, cursor, execute, style::Print, terminal};
use rsedit_core::BufferTrait;
use rsedit_core::ELispExp;
use rsedit_core::EditorState;
use rsedit_core::input::{KeyCode, KeyEvent, KeyModifiers};
use rsedit_core::lisp::Env;
use rsedit_core::ui::{FrameSnapshot, Rect};
use std::{
    io::{Write, stdout},
    sync::Arc,
};

/// Draw one frame from an already-captured [`FrameSnapshot`].
///
/// Takes the snapshot rather than the editor deliberately: this function does
/// terminal I/O, which is slow and can block, and it must not do that while
/// holding a lock on editor state. Capture is `EditorState::snapshot`, which
/// takes every lock once in a fixed order and releases them all before
/// returning; by the time control reaches here there is nothing shared left to
/// touch. That separation is also what lets this move to its own thread later
/// without an audit of every draw call.
pub fn render_frame(frame: &FrameSnapshot) -> std::io::Result<()> {
    let mut stdout = stdout();

    let frame_w = frame.width as isize;
    let frame_h = frame.height as isize;

    execute!(
        stdout,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    for view in &frame.views {
        if view.has_border {
            draw_window_border(
                &mut stdout,
                &view.rect,
                &Some(view.buffer_name.clone()),
                frame.width as u16,
                frame.height as u16,
            )?;
        }

        for (offset_y, line) in view.lines.iter().enumerate() {
            let target_y = view.rect.y + offset_y as isize;
            draw_clipped_row(&mut stdout, view.rect.x, target_y, line, frame_w, frame_h)?;
        }

        if view.is_focused {
            if let Some((cx, cy)) = view.cursor_rel_pos {
                let absolute_cx = view.rect.x + cx as isize;
                let absolute_cy = view.rect.y + cy as isize;
                if (0..frame_w).contains(&absolute_cx) && (0..frame_h).contains(&absolute_cy) {
                    stdout.queue(cursor::MoveTo(absolute_cx as u16, absolute_cy as u16))?;
                }
            }
        }
    }

    // Echo area: the very last row of the frame, drawn on top of
    // whatever's under it -- left free by the default minibuffer window,
    // which docks to the 3 rows just above it, so `message`/error output
    // always has somewhere visible to land without covering the prompt.
    if !frame.echo_message.is_empty() {
        draw_clipped_row(
            &mut stdout,
            0,
            frame_h - 1,
            &frame.echo_message,
            frame_w,
            frame_h,
        )?;
    }

    stdout.flush()
}

/// Prints `content` starting at screen column `start_x`, row `y`, clipping away
/// whatever part of it falls outside the `[0, frame_w) x [0, frame_h)` frame.
/// `start_x` and `y` may be negative or run past the frame edges; nothing is
/// drawn for the portions that don't land inside the visible area, and the
/// call is a no-op if the row is fully off-screen.
fn draw_clipped_row(
    stdout: &mut std::io::Stdout,
    start_x: isize,
    y: isize,
    content: &str,
    frame_w: isize,
    frame_h: isize,
) -> std::io::Result<()> {
    if y < 0 || y >= frame_h {
        return Ok(());
    }

    let chars: Vec<char> = content.chars().collect();
    let len = chars.len() as isize;

    let clip_left = if start_x < 0 { -start_x } else { 0 };
    if clip_left >= len {
        return Ok(());
    }

    let visible_start_x = start_x.max(0);
    let max_visible_len = (frame_w - visible_start_x).max(0);
    let clip_len = (len - clip_left).min(max_visible_len);
    if clip_len <= 0 {
        return Ok(());
    }

    let visible: String = chars[clip_left as usize..(clip_left + clip_len) as usize]
        .iter()
        .collect();
    stdout.queue(cursor::MoveTo(visible_start_x as u16, y as u16))?;
    stdout.queue(Print(visible))?;
    Ok(())
}

/// Draws the border of a floating window's `rect`. The border lives entirely
/// outside `rect` (it is drawn at `rect.x - 1`, `rect.y - 1`, `rect.x + width`
/// and `rect.y + height`), so a window sitting flush against a frame edge has
/// a border edge that falls outside the frame; that edge (and any corner that
/// depends on it) is simply not drawn rather than wrapping or panicking.
fn draw_window_border(
    stdout: &mut std::io::Stdout,
    rect: &Rect,
    title: &Option<String>,
    cols: u16,
    rows: u16,
) -> std::io::Result<()> {
    let frame_w = cols as isize;
    let frame_h = rows as isize;

    let top_content = if let Some(title) = title {
        let title_chars: Vec<char> = title.chars().take(rect.width.saturating_sub(1)).collect();
        let fill = rect.width.saturating_sub(title_chars.len() + 1);
        format!(
            "─{}{}",
            title_chars.into_iter().collect::<String>(),
            "─".repeat(fill)
        )
    } else {
        "─".repeat(rect.width)
    };
    let top_row = format!("┌{}┐", top_content);
    let bottom_row = format!("└{}┘", "─".repeat(rect.width));

    draw_clipped_row(stdout, rect.x - 1, rect.y - 1, &top_row, frame_w, frame_h)?;
    draw_clipped_row(
        stdout,
        rect.x - 1,
        rect.y + rect.height as isize,
        &bottom_row,
        frame_w,
        frame_h,
    )?;

    let left_x = rect.x - 1;
    let right_x = rect.x + rect.width as isize;
    let left_visible = (0..frame_w).contains(&left_x);
    let right_visible = (0..frame_w).contains(&right_x);
    for r in rect.y..(rect.y + rect.height as isize) {
        if r < 0 || r >= frame_h {
            continue;
        }
        if left_visible {
            stdout.queue(cursor::MoveTo(left_x as u16, r as u16))?;
            stdout.queue(Print('│'))?;
        }
        if right_visible {
            stdout.queue(cursor::MoveTo(right_x as u16, r as u16))?;
            stdout.queue(Print('│'))?;
        }
    }
    Ok(())
}

pub fn tui_main<B: BufferTrait>(
    state: &EditorState<B>,
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
        // Capture, then draw. Two steps on purpose: the capture holds locks and
        // does no I/O, the draw does I/O and holds no locks.
        render_frame(&state.snapshot(cols as usize, rows as usize))?;

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
                    CrossKeyCode::Char(c)
                        if key_event.modifiers.contains(CrossModifiers::SHIFT) =>
                    {
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
                    CrossKeyCode::Tab => KeyCode::Tab,
                    _ => continue,
                };
                state.handle_key_event(event, &env);
            }

            Event::Resize(width, height) => {
                state.resize(env.clone(), width as usize, height as usize);
            }

            _ => todo!(),
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
