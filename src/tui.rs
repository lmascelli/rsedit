use crossterm::event::{
    Event, KeyCode as CrossKeyCode, KeyModifiers as CrossModifiers, poll, read,
};
use crossterm::{
    QueueableCommand, cursor, execute,
    style::{
        Attribute, Color as CrossColor, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal,
};
use rsedit_core::BufferTrait;
use rsedit_core::ELispExp;
use rsedit_core::EditorState;
use rsedit_core::input::{KeyCode, KeyEvent, KeyModifiers};
use rsedit_core::lisp::{Env, LispContext};
use rsedit_core::ui::{Color, Face, FrameSnapshot, Highlight, NAMED_COLORS, Rect, Style, Theme};
use std::{
    io::{Write, stdout},
    sync::Arc,
};

pub fn translate_key(key_event: crossterm::event::KeyEvent) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers {
        ctrl: key_event.modifiers.contains(CrossModifiers::CONTROL),
        alt: key_event.modifiers.contains(CrossModifiers::ALT),
        shift: key_event.modifiers.contains(CrossModifiers::SHIFT),
        caps_lock_as_ctrl: false,
    };

    let code = match key_event.code {
        CrossKeyCode::Char(c) if key_event.modifiers.contains(CrossModifiers::SHIFT) => {
            modifiers.shift = false;
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
        CrossKeyCode::Esc => KeyCode::Esc,
        _ => return None,
    };

    Some(KeyEvent { code, modifiers })
}

// ---------------------------------------------------------------------------
// Resolving a colour request into something this terminal can draw
// ---------------------------------------------------------------------------
//
// The editor says what it wants -- "blue", or "#3a5fcd" -- and stops there.
// What blue *is*, and what to do about an exact colour on a terminal that
// cannot show it, are decisions that belong to whatever is actually drawing.
// This is the whole of that decision for the TUI; another frontend answers it
// differently and shares nothing with this file.

/// How much colour this terminal can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    /// The sixteen palette entries and nothing else.
    Ansi16,
    /// The 256-colour cube.
    Ansi256,
    /// 24-bit colour.
    TrueColor,
}

impl ColorDepth {
    /// What the environment claims. `COLORTERM` is the conventional signal for
    /// 24-bit; `TERM` carries the 256-colour one.
    ///
    /// Guessing low is the safe direction: a truecolor terminal sent a palette
    /// index shows an approximate colour, while a 16-colour terminal sent a
    /// 24-bit escape shows nothing useful at all.
    pub fn detect() -> ColorDepth {
        ColorDepth::from_env(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        )
    }

    /// The decision itself, separated from reading the environment so it can
    /// be checked without one.
    pub fn from_env(colorterm: Option<&str>, term: Option<&str>) -> ColorDepth {
        let colorterm = colorterm.unwrap_or_default();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return ColorDepth::TrueColor;
        }
        if term.unwrap_or_default().contains("256color") {
            return ColorDepth::Ansi256;
        }
        ColorDepth::Ansi16
    }
}

fn distance(a: Color, b: Color) -> u32 {
    let d = |x: u8, y: u8| {
        let diff = x as i32 - y as i32;
        (diff * diff) as u32
    };
    d(a.r, b.r) + d(a.g, b.g) + d(a.b, b.b)
}

/// The nearest of the sixteen palette slots to a requested colour.
///
/// The table comes from the core rather than being restated here: it is
/// already the list of what the colour *names* mean, and the palette slots are
/// in the same order. One table, so the two cannot drift apart.
///
/// What each slot actually shows is still the user's own configured colour --
/// this only picks which slot to ask for.
fn nearest_palette_slot(color: Color) -> u8 {
    NAMED_COLORS
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, entry))| distance(color, *entry))
        .map(|(i, _)| i as u8)
        .expect("the palette is not empty")
}

/// The nearest entry in the 256-colour cube to a requested colour.
///
/// 16-231 is a 6x6x6 cube on uneven levels; 232-255 is a grey ramp. A grey
/// asked for as `#808080` lands far better on the ramp than in the cube, so
/// both are tried and the closer wins.
fn nearest_cube_slot(color: Color) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let axis = |v: u8| {
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, level)| (v as i32 - **level as i32).abs())
            .map(|(i, _)| i)
            .expect("LEVELS is not empty")
    };
    let (r, g, b) = (axis(color.r), axis(color.g), axis(color.b));
    let cube_index = 16 + 36 * r + 6 * g + b;
    let cube_rgb = Color::rgb(LEVELS[r], LEVELS[g], LEVELS[b]);

    let grey_step = ((color.r as u32 + color.g as u32 + color.b as u32) / 3).clamp(8, 238);
    let grey_slot = ((grey_step - 8) / 10).min(23) as u8;
    let grey_level = (8 + 10 * grey_slot as u32) as u8;
    let grey_rgb = Color::rgb(grey_level, grey_level, grey_level);

    if distance(color, grey_rgb) < distance(color, cube_rgb) {
        232 + grey_slot
    } else {
        cube_index as u8
    }
}

/// Turn a colour request into a colour this terminal can actually draw.
///
/// A pure function of the request and the terminal's depth, so what the
/// frontend decides is inspectable without a terminal in sight. This is the
/// whole of the decision the editor delegates: it asked for a colour, and this
/// is where "and here is what you actually get" is answered.
pub fn resolve(color: Color, depth: ColorDepth) -> CrossColor {
    match depth {
        ColorDepth::TrueColor => CrossColor::Rgb {
            r: color.r,
            g: color.g,
            b: color.b,
        },
        ColorDepth::Ansi256 => CrossColor::AnsiValue(nearest_cube_slot(color)),
        ColorDepth::Ansi16 => CrossColor::AnsiValue(nearest_palette_slot(color)),
    }
}

/// Draw one frame from an already-captured [`FrameSnapshot`].
///
/// Takes the snapshot rather than the editor deliberately: this function does
/// terminal I/O, which is slow and can block, and it must not do that while
/// holding a lock on editor state. Capture is `EditorState::snapshot`, which
/// takes every lock once in a fixed order and releases them all before
/// returning; by the time control reaches here there is nothing shared left to
/// touch. That separation is also what lets this move to its own thread later
/// without an audit of every draw call.
pub fn render_frame(frame: &FrameSnapshot, depth: ColorDepth) -> std::io::Result<()> {
    let mut stdout = stdout();
    render_to(&mut stdout, frame, depth)
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
pub fn render_to<W: Write>(
    out: &mut W,
    frame: &FrameSnapshot,
    depth: ColorDepth,
) -> std::io::Result<()> {
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
            // The window's own title when it has one -- for the minibuffer that
            // is the prompt. Falling back to the buffer name keeps an untitled
            // floating window labelled rather than bare.
            let label = view
                .title
                .clone()
                .or_else(|| Some(view.buffer_name.clone()));
            draw_window_border(
                out,
                &view.rect,
                &label,
                frame.width as u16,
                frame.height as u16,
            )?;
        }

        for (offset_y, line) in view.lines.iter().enumerate() {
            let target_y = view.rect.y + offset_y as isize;
            draw_clipped_row(out, view.rect.x, target_y, line, frame_w, frame_h)?;
        }

        // Highlights are drawn *over* the rows rather than woven into them, so
        // the ordinary case -- a row with nothing special about it -- still
        // costs one `Print` of one string. Re-printing the few runs that do
        // differ is cheaper than styling every cell, and it keeps the plain
        // path exactly as it was.
        for highlight in &view.highlights {
            draw_highlight(out, view, highlight, &frame.theme, depth, frame_w, frame_h)?;
        }

        // The status line sits on the row below the text, outside the rect --
        // the same arrangement as a border, and for the same reason: the rect
        // is what the buffer gets, and decoration goes around it.
        if let Some(mode_line) = &view.mode_line {
            let face = if view.is_focused {
                Face::MODE_LINE
            } else {
                Face::MODE_LINE_INACTIVE
            };
            // Padded across the window so the status line reads as a bar
            // rather than as a piece of reversed text floating on the row.
            let width = view.rect.width;
            let mut text: String = mode_line.chars().take(width).collect();
            text.push_str(&" ".repeat(width.saturating_sub(text.chars().count())));

            let style = frame.theme.style(face);
            apply_style(out, &style, depth)?;
            draw_clipped_row(
                out,
                view.rect.x,
                view.rect.y + view.rect.height as isize,
                &text,
                frame_w,
                frame_h,
            )?;
            out.queue(SetAttribute(Attribute::Reset))?;
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

    for separator in &frame.separators {
        let style = frame.theme.style(separator.face);
        apply_style(out, &style, depth)?;
        let run: String = std::iter::repeat_n(separator.ch, separator.rect.width).collect();
        for offset in 0..separator.rect.height {
            draw_clipped_row(out, separator.rect.x,
                separator.rect.y + offset as isize,
                &run, frame_w, frame_h)?;
        }
        out.queue(SetAttribute(Attribute::Reset))?;
    }

    // Echo area: the very last row of the frame, drawn on top of
    // whatever's under it -- left free by the default minibuffer window,
    // which docks to the 3 rows just above it, so `message`/error output
    // always has somewhere visible to land without covering the prompt.
    //
    // What a transient keymap is offering or asking displaces the message:
    // they share one row, and of the two it is the offer that is still true.
    let echo_line = if frame.prompt.is_empty() {
        &frame.echo_message
    } else {
        &frame.prompt
    };
    if !echo_line.is_empty() {
        draw_clipped_row(out, 0, frame_h - 1, echo_line, frame_w, frame_h)?;
    }

    // What the editor is waiting for, at the right-hand end of the same row.
    // Right-aligned so it never collides with a message growing from the left,
    // and drawn after one so it wins if they ever meet.
    if !frame.pending_input.is_empty() {
        let start = frame_w - frame.pending_input.chars().count() as isize;
        draw_clipped_row(
            out,
            start.max(0),
            frame_h - 1,
            &frame.pending_input,
            frame_w,
            frame_h,
        )?;
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

/// Redraw one run of an already-drawn row with its face applied.
///
/// Reads the text back out of the view's own lines rather than being told it,
/// so a highlight can never disagree with what was drawn underneath it.
fn draw_highlight<W: Write>(
    out: &mut W,
    view: &rsedit_core::ui::RenderableWindowView,
    highlight: &Highlight,
    theme: &Theme,
    depth: ColorDepth,
    frame_w: isize,
    frame_h: isize,
) -> std::io::Result<()> {
    let Some(line) = view.lines.get(highlight.row) else {
        return Ok(());
    };
    // A face bound to nothing would be redrawn exactly as it already looks, so
    // there is nothing to do -- and no attribute reset to leak.
    let style = theme.style(highlight.face);
    if style.is_plain() {
        return Ok(());
    }

    let chars: Vec<char> = line.chars().collect();
    let start = highlight.start_col.min(chars.len());
    let end = highlight.end_col.min(chars.len());

    // A selection that runs past the end of a short line still has to look
    // selected out to where it ends, or a multi-line region appears to have
    // ragged holes in it. Padding with spaces is what fills that in.
    let padding = highlight.end_col.saturating_sub(end);
    let mut text: String = chars[start..end].iter().collect();
    text.push_str(&" ".repeat(padding));
    if text.is_empty() {
        return Ok(());
    }

    apply_style(out, &style, depth)?;
    draw_clipped_row(
        out,
        view.rect.x + highlight.start_col as isize,
        view.rect.y + highlight.row as isize,
        &text,
        frame_w,
        frame_h,
    )?;
    // `Attribute::Reset` is SGR 0, which clears colours as well as attributes,
    // so one sequence puts the terminal back however the style was built.
    out.queue(SetAttribute(Attribute::Reset))?;
    Ok(())
}

/// Turn a [`Style`] into escape sequences.
///
/// The one place in the frontend that knows what a face looks like. Everything
/// upstream of here names a face and leaves the appearance to the theme, so
/// another frontend re-implements this function and nothing else.
fn apply_style<W: Write>(out: &mut W, style: &Style, depth: ColorDepth) -> std::io::Result<()> {
    for (on, attribute) in [
        (style.reverse, Attribute::Reverse),
        (style.bold, Attribute::Bold),
        (style.italic, Attribute::Italic),
        (style.underline, Attribute::Underlined),
    ] {
        if on {
            out.queue(SetAttribute(attribute))?;
        }
    }
    if let Some(color) = style.fg {
        out.queue(SetForegroundColor(resolve(color, depth)))?;
    }
    if let Some(color) = style.bg {
        out.queue(SetBackgroundColor(resolve(color, depth)))?;
    }
    Ok(())
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
    // Asked once: the terminal's capabilities do not change while it runs, and
    // a frame should not be re-reading the environment.
    let depth = ColorDepth::detect();
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
        render_frame(&state.snapshot(&env, cols as usize, rows as usize), depth)?;

        // An echo message with a timeout in force is the one thing that
        // changes the screen without the user doing anything, so it is the one
        // thing this loop cannot simply block through: waiting on `read` alone
        // would leave the message up until the next keystroke, which is not a
        // timeout but a coincidence. Waiting only as long as the message has
        // left, and looping back to redraw when nothing arrives, keeps the
        // loop event-driven -- there is no polling when no message is pending,
        // and at most one extra wake-up when one is.
        if let Some(remaining) = state.echo_expiry_in(&env)
            && !poll(remaining)?
        {
            continue;
        }

        match read()? {
            Event::Key(key_event) => match translate_key(key_event) {
                Some(event) => state.handle_key_event(event, &env),
                None => state.log_diagnostic(&format!(
                    "[INFO] no translation for the key {:?}",
                    key_event.code
                )),
            },

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
