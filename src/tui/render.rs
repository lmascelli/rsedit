//! Drawing a frame on the terminal: the rules, the windows with their text,
//! gutter, highlights and status line, the echo area, the cursor last, and
//! the clipboard's escape.
//!
//! Design: `doc/architettura-editor.typ, <tui>`.
use super::colour::{ColorDepth, resolve};
use crossterm::{
    QueueableCommand, cursor, execute,
    style::{Attribute, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal,
};
use rsedit_core::width::{cells, cut, fit};
use rsedit_core::{Face, FrameSnapshot, Highlight, Rect, RenderableWindowView, Style, Theme};
use std::io::{Write, stdout};

/// Draws FRAME on the terminal, for DEPTH colours. Takes a frame already
/// captured, never the editor: the I/O holds no lock.
pub fn render_frame(frame: &FrameSnapshot, depth: ColorDepth) -> std::io::Result<()> {
    let mut stdout = stdout();
    render_to(&mut stdout, frame, depth)
}

/// Draws FRAME into OUT, for DEPTH colours: any writer, so that a test reads
/// the escapes. The cursor is placed last, since every row drawn moves it.
pub fn render_to<W: Write>(
    out: &mut W,
    frame: &FrameSnapshot,
    depth: ColorDepth,
) -> std::io::Result<()> {
    let frame_w = frame.width as isize;
    let frame_h = frame.height as isize;

    // The page: the colours of the face `default`, which every cell has
    // unless a face of its own says otherwise.
    let base = page(&frame.theme);

    // Hidden while drawing, so that it is not seen moving. The colours before
    // the clear: a terminal erases in the background colour in force.
    out.queue(cursor::Hide)?;
    if !base.is_plain() {
        reset(out, &base, depth)?;
    }
    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    // Where the cursor should end up, remembered rather than applied.
    let mut cursor_at: Option<(u16, u16)> = None;

    // The rules before the windows, so that a floating window covers them.
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
        reset(out, &base, depth)?;
    }

    for view in &frame.views {
        if view.has_border {
            // Its title, the prompt for the minibuffer, or its buffer's name.
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

        // Every row of the rectangle, padded to its width: what makes a
        // floating window opaque over the one beneath.
        for offset_y in 0..view.rect.height {
            let target_y = view.rect.y + offset_y as isize;
            let text = view.lines.get(offset_y).map(String::as_str).unwrap_or("");
            // Cut to the window by cells, so that a long line stays in it.
            let row = fit(text, view.rect.width);
            draw_clipped_row(out, view.rect.x, target_y, &row, frame_w, frame_h)?;
        }

        // The gutter, left of the rectangle, cell by cell: point's line has a
        // face of its own.
        for (offset_y, cell) in view.gutter.iter().enumerate() {
            let style = frame.theme.style(cell.face);
            apply_style(out, &style, depth)?;
            let text = fit(&cell.text, view.gutter_width);
            draw_clipped_row(
                out,
                view.rect.x - view.gutter_width as isize,
                view.rect.y + offset_y as isize,
                &text,
                frame_w,
                frame_h,
            )?;
            reset(out, &base, depth)?;
        }

        // The highlights, drawn again over the rows: a plain row is one
        // `Print`.
        for highlight in &view.highlights {
            draw_highlight(
                out,
                view,
                highlight,
                &frame.theme,
                &base,
                depth,
                frame_w,
                frame_h,
            )?;
        }

        // The status line, on the row below the rectangle.
        if let Some(mode_line) = &view.mode_line {
            let face = if view.is_focused {
                Face::MODE_LINE
            } else {
                Face::MODE_LINE_INACTIVE
            };
            // Padded across the window and its gutter, as a bar.
            let width = view.rect.width + view.gutter_width;
            let text = fit(mode_line, width);

            let style = frame.theme.style(face);
            apply_style(out, &style, depth)?;
            draw_clipped_row(
                out,
                view.rect.x - view.gutter_width as isize,
                view.rect.y + view.rect.height as isize,
                &text,
                frame_w,
                frame_h,
            )?;
            reset(out, &base, depth)?;
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

    // The echo area, on the last row, which no window takes: the message, or
    // what a transient keymap offers in its place.
    let echo_line = if frame.prompt.is_empty() {
        &frame.echo_message
    } else {
        &frame.prompt
    };
    if base.bg.is_some() {
        // Painted whole, where the page has a colour: a terminal that does
        // not erase in the background colour would leave the row its own.
        let row = fit(echo_line, frame.width);
        draw_clipped_row(out, 0, frame_h - 1, &row, frame_w, frame_h)?;
    } else if !echo_line.is_empty() {
        draw_clipped_row(out, 0, frame_h - 1, echo_line, frame_w, frame_h)?;
    }

    // What the editor is waiting for, at the right end of the same row, over
    // the message where they meet.
    if !frame.pending_input.is_empty() {
        let start = frame_w - cells(&frame.pending_input) as isize;
        draw_clipped_row(
            out,
            start.max(0),
            frame_h - 1,
            &frame.pending_input,
            frame_w,
            frame_h,
        )?;
    }

    // The cursor, at point, or hidden when point is off screen.
    if let Some((x, y)) = cursor_at {
        out.queue(cursor::MoveTo(x, y))?;
        out.queue(cursor::Show)?;
    }

    // The text owed to the clipboard, on the frames that carry it.
    if let Some(text) = &frame.clipboard {
        out.queue(Print(osc52_copy(text)))?;
    }

    out.flush()
}

/// Returns the OSC 52 escape that asks the terminal to put TEXT in the system
/// clipboard, ended by ST, `ESC \`, which tmux passes on.
pub(crate) fn osc52_copy(text: &str) -> String {
    format!("\x1b]52;c;{}\x1b\\", base64_encode(text.as_bytes()))
}

/// Returns BYTES in standard base64, with no line breaks: twenty lines, where
/// a crate would be one more dependency for one alphabet.
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        // Three bytes, four six-bit groups; a short last chunk is padded
        // with zero bits here and with `=` below.
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

/// Draws again the run of VIEW's row HIGHLIGHT covers, in its face, reading
/// the text from the view's own rows so that the two agree.
fn draw_highlight<W: Write>(
    out: &mut W,
    view: &RenderableWindowView,
    highlight: &Highlight,
    theme: &Theme,
    base: &Style,
    depth: ColorDepth,
    frame_w: isize,
    frame_h: isize,
) -> std::io::Result<()> {
    let Some(line) = view.lines.get(highlight.row) else {
        return Ok(());
    };
    // A face bound to nothing looks as the row already does.
    let mut style = theme.style(highlight.face);
    if style.bg.is_none()
        && !style.reverse
        && view.highlights.iter().any(|base| {
            base.face == Face::HL_LINE
                && base.row == highlight.row
                && base.start_col <= highlight.start_col
                && base.end_col >= highlight.end_col
        })
    {
        style.bg = theme.style(Face::HL_LINE).bg;
    }
    if style.is_plain() {
        return Ok(());
    }

    // The cells of the line under it, by cells and not by characters, and
    // spaces past the end of the line, for a region that runs past it.
    let span = highlight.end_col.saturating_sub(highlight.start_col);
    let mut text = cut(line, highlight.start_col, span);
    let drawn = cells(&text);
    text.push_str(&" ".repeat(span.saturating_sub(drawn)));
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
    reset(out, base, depth)?;
    Ok(())
}

/// Returns the colours of the page in THEME: those of the face `default`,
/// without its attributes, which belong to no cell that does not ask.
fn page(theme: &Theme) -> Style {
    let default = theme.style(Face::DEFAULT);
    Style {
        fg: default.fg,
        bg: default.bg,
        ..Style::plain()
    }
}

/// Ends a run drawn in a face: clears the attributes and the colours, SGR 0
/// doing both, and puts back those of the page, BASE, for what is drawn next.
fn reset<W: Write>(out: &mut W, base: &Style, depth: ColorDepth) -> std::io::Result<()> {
    out.queue(SetAttribute(Attribute::Reset))?;
    if let Some(color) = base.fg {
        out.queue(SetForegroundColor(resolve(color, depth)))?;
    }
    if let Some(color) = base.bg {
        out.queue(SetBackgroundColor(resolve(color, depth)))?;
    }
    Ok(())
}

/// Writes the escapes that draw in STYLE, its colours resolved for DEPTH: the
/// one place the terminal learns what a face looks like. A colour STYLE does
/// not name stays the page's, which [`reset`] left in force.
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

/// Prints CONTENT from cell START_X of row Y, cut to the frame, FRAME_W by
/// FRAME_H: START_X and Y may be off it, and nothing is drawn of what is.
/// Cut by cells, as the editor counted them, so that a wide character takes
/// its two. Every control character in it is printed as [`printable`] has it:
/// this is the one way text reaches the terminal, so nothing a file, a
/// command's output or a message holds can be obeyed by it.
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
    let clip_left = (-start_x).max(0) as usize;
    let visible_start_x = start_x.max(0);
    let room = (frame_w - visible_start_x).max(0) as usize;
    if room == 0 {
        return Ok(());
    }
    let shown: String = content.chars().map(printable).collect();
    let visible = cut(&shown, clip_left, room);
    if visible.is_empty() {
        return Ok(());
    }
    stdout.queue(cursor::MoveTo(visible_start_x as u16, y as u16))?;
    stdout.queue(Print(visible))?;
    Ok(())
}

/// Returns C as the terminal is given it. A control character, which the
/// terminal would obey rather than show -- ESC starting a sequence that can
/// set the clipboard or the title, CR and BS moving the cursor -- becomes a
/// picture of itself one column wide, so that every column stays where the
/// editor counted it: `␛` for ESC, `␡` for DEL, and the replacement
/// character for the C1 controls, which have no picture. A Tab of the text
/// has been drawn as spaces by then; one anywhere else, in a message or a
/// buffer's name, is `␉`, one column, as the terminal would have moved it to
/// a stop of its own.
pub(crate) fn printable(c: char) -> char {
    match c {
        '\u{0}'..='\u{1f}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{fffd}'),
        '\u{7f}' => '\u{2421}',
        '\u{80}'..='\u{9f}' => '\u{fffd}',
        _ => c,
    }
}

/// Draws the border of a floating window around RECT, outside it, TITLE in
/// its top edge; an edge that falls off the frame, COLS by ROWS, is not
/// drawn.
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
        let title = cut(title, 0, rect.width.saturating_sub(1));
        let fill = rect.width.saturating_sub(cells(&title) + 1);
        format!("─{}{}", title, "─".repeat(fill))
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
