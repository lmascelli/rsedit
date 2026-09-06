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
    render_to(&mut stdout, frame)
}

/// Draw one frame into `out`.
///
/// Generic over the writer so a test can render into a `Vec<u8>` and inspect
/// the escape sequences, which is the only way to check something like "where
/// did the cursor end up" without a terminal attached.
///
/// # Ordering
///
/// Everything is drawn first and the cursor is placed **last**, immediately
/// before the flush. That ordering is the whole point rather than a detail:
/// every `draw_clipped_row` emits `MoveTo` followed by `Print`, so the terminal
/// cursor is left wherever the most recent piece of text ended. Positioning the
/// cursor in the middle of drawing -- as this used to, inside the window loop --
/// means the echo area, drawn afterwards, silently drags it to the end of the
/// message. The cursor then appears in the echo area instead of at point.
pub fn render_to<W: Write>(out: &mut W, frame: &FrameSnapshot) -> std::io::Result<()> {
    let frame_w = frame.width as isize;
    let frame_h = frame.height as isize;

    // Hidden for the duration of the redraw: without this the cursor is visibly
    // dragged across the screen by each row that gets printed.
    execute!(
        out,
        cursor::Hide,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    // Where the cursor should end up, remembered rather than applied.
    let mut cursor_at: Option<(u16, u16)> = None;

    for view in &frame.views {
        if view.has_border {
            draw_window_border(
                out,
                &view.rect,
                &Some(view.buffer_name.clone()),
                frame.width as u16,
                frame.height as u16,
            )?;
        }

        for (offset_y, line) in view.lines.iter().enumerate() {
            let target_y = view.rect.y + offset_y as isize;
            draw_clipped_row(out, view.rect.x, target_y, line, frame_w, frame_h)?;
        }

        if view.is_focused {
            if let Some((cx, cy)) = view.cursor_rel_pos {
                let absolute_cx = view.rect.x + cx as isize;
                let absolute_cy = view.rect.y + cy as isize;
                if (0..frame_w).contains(&absolute_cx) && (0..frame_h).contains(&absolute_cy) {
                    cursor_at = Some((absolute_cx as u16, absolute_cy as u16));
                }
            }
        }
    }

    // Echo area: the very last row of the frame, drawn on top of
    // whatever's under it -- left free by the default minibuffer window,
    // which docks to the 3 rows just above it, so `message`/error output
    // always has somewhere visible to land without covering the prompt.
    if !frame.echo_message.is_empty() {
        draw_clipped_row(out, 0, frame_h - 1, &frame.echo_message, frame_w, frame_h)?;
    }

    // Now, with nothing left to draw over it, put the cursor where it belongs.
    // If the focused window has no visible cursor cell it simply stays hidden,
    // which is better than showing it at whatever position drawing happened to
    // leave behind.
    if let Some((x, y)) = cursor_at {
        out.queue(cursor::MoveTo(x, y))?;
        out.queue(cursor::Show)?;
    }

    out.flush()
}

/// Prints `content` starting at screen column `start_x`, row `y`, clipping away
/// whatever part of it falls outside the `[0, frame_w) x [0, frame_h)` frame.
/// `start_x` and `y` may be negative or run past the frame edges; nothing is
/// drawn for the portions that don't land inside the visible area, and the
/// call is a no-op if the row is fully off-screen.
fn draw_clipped_row<W: Write>(
    stdout: &mut W,
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
fn draw_window_border<W: Write>(
    stdout: &mut W,
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

#[cfg(test)]
mod tests {
    use super::*;
    use rsedit_core::buffer::gap_buffer::GapBuffer;
    use rsedit_core::create_global_env;

    const COLS: u16 = 80;
    const ROWS: u16 = 24;

    /// Render one frame into memory and return the bytes.
    fn frame(state: &EditorState<GapBuffer>) -> String {
        let mut out: Vec<u8> = Vec::new();
        let snapshot = state.snapshot(COLS as usize, ROWS as usize);
        render_to(&mut out, &snapshot).expect("rendering must succeed");
        String::from_utf8(out).expect("crossterm emits valid UTF-8")
    }

    /// Every cursor placement in the stream, as (col, row), in order.
    ///
    /// `MoveTo(x, y)` is written as `ESC [ y+1 ; x+1 H`, so this reads the
    /// output the way a terminal would rather than trusting a hard-coded
    /// escape string.
    fn placements(rendered: &str) -> Vec<(u16, u16)> {
        let mut out = Vec::new();
        for chunk in rendered.split('\u{1b}').skip(1) {
            let Some(body) = chunk.strip_prefix('[') else {
                continue;
            };
            let Some(end) = body.find('H') else { continue };
            let coords = &body[..end];
            if let Some((row, col)) = coords.split_once(';') {
                if let (Ok(row), Ok(col)) = (row.parse::<u16>(), col.parse::<u16>()) {
                    out.push((col.saturating_sub(1), row.saturating_sub(1)));
                }
            }
        }
        out
    }

    fn expected_cursor(state: &EditorState<GapBuffer>) -> (u16, u16) {
        let snapshot = state.snapshot(COLS as usize, ROWS as usize);
        let view = snapshot
            .focused_view()
            .expect("a window must have focus")
            .clone();
        let (cx, cy) = view
            .cursor_rel_pos
            .expect("the focused window has a cursor");
        (
            (view.rect.x + cx as isize) as u16,
            (view.rect.y + cy as isize) as u16,
        )
    }

    /// The regression this guards.
    ///
    /// The cursor used to be positioned inside the window loop, so the echo
    /// area -- drawn afterwards, and like every other row emitting `MoveTo`
    /// then `Print` -- left the terminal cursor at the end of its message
    /// instead of at point.
    #[test]
    fn the_cursor_is_placed_after_the_echo_area_not_before_it() {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env");
        let _ = &env;
        state.set_echo_message("a message long enough to move the cursor");

        let rendered = frame(&state);
        let last = *placements(&rendered)
            .last()
            .expect("the frame must position the cursor at least once");

        assert_eq!(
            last,
            expected_cursor(&state),
            "the last cursor placement should be point, not wherever drawing ended"
        );
        assert_ne!(last.1, ROWS - 1, "the cursor was left on the echo-area row");
    }

    /// The cursor is hidden while the frame is drawn and shown again once it is
    /// in the right place, so it is never seen skittering across the screen.
    #[test]
    fn the_cursor_is_hidden_while_drawing_and_shown_at_the_end() {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env");
        let _ = &env;
        state.set_echo_message("hello");

        let rendered = frame(&state);
        let hide = rendered
            .find("\u{1b}[?25l")
            .expect("the cursor must be hidden first");
        let show = rendered
            .rfind("\u{1b}[?25h")
            .expect("the cursor must be shown again");
        assert!(hide < show, "Hide must come before Show");

        let tail = &rendered[show..];
        assert!(
            !tail.contains("hello"),
            "nothing may be drawn after the cursor is placed and shown"
        );
    }

    /// An empty echo area must not change where the cursor lands.
    #[test]
    fn the_cursor_lands_at_point_with_no_echo_message() {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env");
        let _ = &env;
        state.set_echo_message("");

        let rendered = frame(&state);
        let last = *placements(&rendered).last().expect("a cursor placement");
        assert_eq!(last, expected_cursor(&state));
    }
}
