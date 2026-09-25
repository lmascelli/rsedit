use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    KeyCode as CrossKeyCode, KeyEventKind, KeyModifiers as CrossModifiers,
    MouseButton as CrossMouseButton, MouseEventKind as CrossMouseKind, poll, read,
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
use rsedit_core::input::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseKind};
use rsedit_core::lisp::{Env, LispContext};
use rsedit_core::mouse_mode;
use rsedit_core::ui::{Color, Face, FrameSnapshot, Highlight, NAMED_COLORS, Rect, Style, Theme};
use std::{
    io::{Write, stdout},
    sync::Arc,
};

/// A crossterm mouse event as the editor's own.
///
/// `None` for the kinds the editor has no word for -- a bare move with no
/// button down, and the horizontal wheel. Dropping them here rather than
/// carrying them inwards keeps the editor's vocabulary to the things something
/// actually acts on.
pub fn translate_mouse(event: crossterm::event::MouseEvent) -> Option<MouseEvent> {
    let button = |button: CrossMouseButton| match button {
        CrossMouseButton::Left => MouseButton::Left,
        CrossMouseButton::Middle => MouseButton::Middle,
        CrossMouseButton::Right => MouseButton::Right,
    };
    let kind = match event.kind {
        CrossMouseKind::Down(b) => MouseKind::Down(button(b)),
        CrossMouseKind::Up(b) => MouseKind::Up(button(b)),
        CrossMouseKind::Drag(b) => MouseKind::Drag(button(b)),
        CrossMouseKind::ScrollUp => MouseKind::ScrollUp,
        CrossMouseKind::ScrollDown => MouseKind::ScrollDown,
        _ => return None,
    };
    Some(MouseEvent {
        kind,
        column: event.column,
        row: event.row,
        modifiers: KeyModifiers {
            ctrl: event.modifiers.contains(CrossModifiers::CONTROL),
            alt: event.modifiers.contains(CrossModifiers::ALT),
            shift: event.modifiers.contains(CrossModifiers::SHIFT),
            caps_lock_as_ctrl: false,
        },
    })
}

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

        // Every row of the rect, padded to its full width -- not just the rows
        // that have text, and not just as far as the text goes.
        //
        // A tiled window would get away without this, because the screen is
        // cleared before any of it is drawn. A floating window is drawn *over*
        // what is already there, so every cell it leaves unpainted shows the
        // window underneath: a prompt three lines tall over a full buffer used
        // to have that buffer's text running through the gaps and past the ends
        // of its own lines. Painting the whole rectangle is what makes a
        // floating window opaque, and it costs no extra `Print` for the rows
        // that do have text -- the padding goes into the same string.
        for offset_y in 0..view.rect.height {
            let target_y = view.rect.y + offset_y as isize;
            let text = view.lines.get(offset_y).map(String::as_str).unwrap_or("");
            // Clipped to the window as well as padded to it: a line longer than
            // the window it is in belongs to that window, not to its neighbour.
            let mut row: String = text.chars().take(view.rect.width).collect();
            let padding = view.rect.width.saturating_sub(row.chars().count());
            row.push_str(&" ".repeat(padding));
            draw_clipped_row(out, view.rect.x, target_y, &row, frame_w, frame_h)?;
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
            draw_clipped_row(
                out,
                separator.rect.x,
                separator.rect.y + offset as isize,
                &run,
                frame_w,
                frame_h,
            )?;
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

    // The one thing here that is not drawing. See `FrameSnapshot::clipboard`:
    // the editor cannot reach the clipboard because it does not know it has a
    // terminal, so it leaves the text on the frame and this puts it where a
    // terminal can see it. Emitted after everything else and before the flush,
    // so it costs one write on the frames that carry it and nothing at all on
    // the ones that do not.
    if let Some(text) = &frame.clipboard {
        out.queue(Print(osc52_copy(text)))?;
    }

    out.flush()
}

/// The escape sequence that asks the terminal to put `text` in the system
/// clipboard.
///
/// # Why an escape sequence and not a program
///
/// The alternative is spawning `pbcopy`, `wl-copy` or `xclip`, which means
/// knowing which one this machine has, and which fails outright over SSH --
/// the clipboard those talk to is the remote machine's, and the one the user
/// is looking at is local. An escape travels the same pty as the text does, so
/// it arrives wherever the terminal is. That is the whole argument for it.
///
/// Terminated with `ESC \` (ST) rather than BEL: both are accepted, and ST is
/// what tmux passes through without special-casing.
///
/// The read direction of OSC 52 is deliberately not implemented. Terminals
/// disable it by default -- it would let any program that can write to the
/// terminal read the user's clipboard -- and where it is allowed the reply
/// arrives on stdin interleaved with keystrokes. Text comes *in* by bracketed
/// paste instead, which is the terminal volunteering the same content through
/// a channel that already exists.
pub(crate) fn osc52_copy(text: &str) -> String {
    format!("\x1b]52;c;{}\x1b\\", base64_encode(text.as_bytes()))
}

/// Standard base64, no line breaks.
///
/// Hand-written because the alternative is a dependency for one alphabet and
/// twenty lines: `core` has exactly one dependency today, and a clipboard is
/// not the reason to make it two.
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        // Three bytes become four six-bit groups. A short final chunk is
        // padded with zero bits here and with `=` below, which is what tells
        // the decoder how many of those bits were never data.
        let mut block = [0u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let packed = u32::from(block[0]) << 16 | u32::from(block[1]) << 8 | u32::from(block[2]);
        for group in 0..4 {
            if group <= chunk.len() {
                let index = (packed >> (18 - 6 * group)) & 0x3f;
                out.push(ALPHABET[index as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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
    // With this on, a paste arrives as one `Event::Paste` instead of as the
    // keystrokes it looks like. That is the difference between pasting a
    // function and typing it: one undo step rather than one per character, one
    // run of the hooks rather than hundreds, and no auto-pairing of brackets
    // that were already balanced in the pasted text.
    //
    // It is also how text gets *in* from the system clipboard at all. The read
    // direction of OSC 52 is refused by most terminals, so rather than ask for
    // the clipboard the editor accepts it when the terminal offers it -- which
    // is what the user's own paste key already does.
    execute!(stdout(), EnableBracketedPaste)?;
    // Asked once: the terminal's capabilities do not change while it runs, and
    // a frame should not be re-reading the environment.
    let depth = ColorDepth::detect();
    // Tell the editor the starting frame size
    {
        let (cols, rows) = terminal::size()?;
        env.set_variable("frame-width".into(), ELispExp::number(cols as f64));
        env.set_variable("frame-height".into(), ELispExp::number(rows as f64));
    }

    // Whether the terminal is currently reporting the mouse. Followed rather
    // than set once, so `M-x mouse-mode-toggle` takes effect on the next key
    // instead of the next run -- and so that a terminal is never left
    // reporting the mouse to an editor that has stopped listening.
    let mut capturing = false;

    // Whether anything has happened that the screen does not already show.
    //
    // # Why the loop is not simply "draw, then wait"
    //
    // It was, and it could be while only keys, pastes and resizes woke it:
    // every one of those changes something, so every one deserved a frame. A
    // terminal reporting the mouse also delivers motion and drag events, dozens
    // a second, and almost none of them mean anything here. Redrawing for each
    // is a screen repainted continuously with a frame identical to the last,
    // which is visible as flicker and is pure waste.
    //
    // So an event that changed nothing leaves this false, and the next pass
    // goes straight back to waiting without composing or drawing anything.
    let mut dirty = true;
    let (cols, rows) = terminal::size()?;
    let mut frame = state.snapshot(&env, cols as usize, rows as usize);

    while state.is_running() {
        // Before the frame rather than after: a capture switched on here is on
        // for the click that comes with this loop's `read`.
        let wanted = mouse_mode(&env);
        if wanted != capturing {
            if wanted {
                execute!(stdout(), EnableMouseCapture)?;
            } else {
                execute!(stdout(), DisableMouseCapture)?;
            }
            capturing = wanted;
        }
        if dirty {
            let (cols, rows) = terminal::size()?;
            // Capture, then draw. Two steps on purpose: the capture holds locks
            // and does no I/O, the draw does I/O and holds no locks.
            frame = state.snapshot(&env, cols as usize, rows as usize);
            render_frame(&frame, depth)?;
            dirty = false;
        }

        // Some things change the screen without the user doing anything -- an
        // echo message expiring on its timer, colour arriving from the
        // highlighter's thread -- and blocking on `read` alone sleeps straight
        // through them. Which of those are outstanding is a question about the
        // editor, so it answers it; this loop only has to wait no longer than
        // it is told and redraw when nothing arrives.
        //
        // Still event-driven: with nothing pending the answer is `None` and
        // this blocks indefinitely, so an idle editor wakes for nothing at all.
        if let Some(remaining) = state.next_redraw_in(&env, &frame)
            && !poll(remaining)?
        {
            // The timer expired rather than an event arriving: something the
            // editor was waiting on -- a message going stale, colour arriving
            // -- has changed the frame without anybody touching a key.
            dirty = true;
            continue;
        }

        match read()? {
            Event::Key(key_event) => match translate_key(key_event) {
                Some(event) => {
                    if key_event.kind == KeyEventKind::Press {
                        state.handle_key_event(event, &env);
                        dirty = true;
                    }
                }
                None => state.log_diagnostic(&format!(
                    "[INFO] no translation for the key {:?}",
                    key_event.code
                )),
            },

            Event::Resize(width, height) => {
                state.resize(env.clone(), width as usize, height as usize);
                dirty = true;
            }

            Event::Paste(text) => {
                state.handle_paste(text, &env);
                dirty = true;
            }

            Event::Mouse(mouse_event) => {
                // Only a mouse event the editor acted on is worth a frame.
                // Everything else -- a bare move, a button nothing is bound
                // to, a click on a float -- leaves the screen as it was.
                if let Some(event) = translate_mouse(mouse_event)
                    && state.handle_mouse_event(event, &env)
                {
                    dirty = true;
                }
            }

            // Focus changes, mouse events and anything a future crossterm
            // adds. Ignored rather than `todo!()`: this arm used to panic, so
            // enabling bracketed paste without handling it would have crashed
            // the editor on the first paste -- and a terminal that starts
            // reporting focus would have crashed it for no reason at all.
            _ => (),
        }
    }

    if capturing {
        execute!(stdout(), DisableMouseCapture)?;
    }
    execute!(
        stdout(),
        DisableBracketedPaste,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;
    terminal::disable_raw_mode()?;
    Ok(())
}
