//! The commands on a rectangle, the block of columns between point and the
//! mark. The geometry is in [`crate::text::rectangle`]; here every command
//! applies its edits bottom line first, and one that changes the buffer
//! leaves point at the top-left corner.
//!
//! Design: `doc/architettura-editor.typ, <modifiche-multiple>`.
use super::*;
use crate::buffer::{Buffer, Mark};
use crate::primitives::edits::{delete_range, insert_text};
use crate::primitives::movement::goto_offset;
use crate::text::rectangle::{Rectangle, Span, spans, text_of};

/// Returns the rectangle between point and the mark, or `None` when the mark
/// is not active.
fn active_rectangle<B: BufferTrait>(buf: &Buffer<B>) -> Option<Rectangle> {
    let mark = buf.mark.filter(|mark| mark.active)?;
    let point = buf.text.cursor_pos_1d();
    Some(Rectangle::between_offsets(
        &buf.text,
        mark.at,
        point,
        mark.inclusive,
    ))
}

/// Returns the rectangle between point and the mark, or the error a command
/// gives when the mark is not active, worded as `region` words its own.
fn require_rectangle<B: BufferTrait>(
    buf: &Buffer<B>,
) -> Result<Rectangle, EvalError<EditorState<B>>> {
    active_rectangle(buf).ok_or_else(|| {
        EvalError::RuntimeMessage("The mark is not set now, so there is no rectangle".into())
    })
}

/// Puts point at RECT's top-left corner.
fn go_to_corner<B: BufferTrait>(buf: &mut Buffer<B>, rect: &Rectangle) {
    let at = buf.text.cursor_2d_to_1d(rect.top, rect.left);
    goto_offset(&mut buf.text, at);
}

/// Calls F with BUF and the span of every line of RECT, bottom line first.
///
/// The spans are worked out before anything changes, so F sees the offsets
/// each line had when the rectangle was drawn; a span says what is there and
/// how far short of the left edge its line stops.
fn each_line<B: BufferTrait, F>(buf: &mut Buffer<B>, rect: &Rectangle, mut f: F)
where
    F: FnMut(&mut Buffer<B>, &Span),
{
    let mut lines = spans(&buf.text, rect);
    lines.reverse();
    for span in &lines {
        f(buf, span);
    }
}

// TODO(BUF-4): a refused insertion is ignored, and the commands that insert
// answer as though they had.
/// Inserts CONTENT on SPAN's line at the rectangle's left edge, padding the
/// line out to that column first when it stops short of it: a column that
/// does not exist yet is what `open-rectangle`, `yank-rectangle` and
/// `string-rectangle` were asked to fill.
fn insert_at_edge<B: BufferTrait>(buf: &mut Buffer<B>, span: &Span, content: &str) {
    if span.padding > 0 {
        let padding = " ".repeat(span.padding);
        let _ = insert_text(buf, span.start, &padding);
    }
    let _ = insert_text(buf, span.start + span.padding, content);
}

// ---------------------------------------------------------------------------
// Seeing one
// ---------------------------------------------------------------------------

pub const RECTANGLE_MARK_MODE_DOC: &str = "(rectangle-mark-mode): Show the region as the block of \
         columns between point and the mark, or stop; with no mark set, set it at point first. \
         Return t while it is on. Any edit ends it.";

primitive!(rectangle_mark_mode, _args, _env, ctx, {
    Ok(ELispExp::boolean(ctx.with_current_buffer_mut(|buf| {
        match buf.mark.filter(|mark| mark.active) {
            // On, and being asked again: off, leaving the region as an
            // ordinary one rather than deactivating it. `C-x SPC' twice
            // should give back the selection you had, not no selection.
            Some(mark) if mark.rectangle => {
                buf.mark = Some(Mark {
                    rectangle: false,
                    ..mark
                });
                false
            }
            Some(mark) => {
                buf.mark = Some(Mark {
                    rectangle: true,
                    ..mark
                });
                true
            }
            None => {
                let at = buf.text.cursor_pos_1d();
                buf.mark = Some(Mark {
                    at,
                    active: true,
                    inclusive: false,
                    rectangle: true,
                });
                true
            }
        }
    })))
});

pub const RECTANGLE_BOUNDS_DOC: &str = "(rectangle-bounds): Return the block between point and \
         the mark as (TOP-LINE BOTTOM-LINE LEFT-COLUMN RIGHT-COLUMN), or nil when there is no \
         active mark. Lines count from 1 and columns from 0, and RIGHT-COLUMN is one past the \
         block, so the width is RIGHT-COLUMN minus LEFT-COLUMN.";

primitive!(rectangle_bounds, _args, _env, ctx, {
    Ok(ctx.with_current_buffer(|buf| match active_rectangle(buf) {
        None => ELispExp::nil(),
        Some(rect) => ELispExp::proper_list(vec![
            ELispExp::number(rect.top as f64 + 1.0),
            ELispExp::number(rect.bottom as f64 + 1.0),
            ELispExp::number(rect.left as f64),
            ELispExp::number(rect.right as f64),
        ]),
    }))
});

// ---------------------------------------------------------------------------
// Taking one out
// ---------------------------------------------------------------------------

// TODO(BUF-4): a refused deletion is ignored: in a read-only buffer
// `kill-rectangle` keeps the block although it stayed, and the commands
// answer with the number of lines as though they had cut it.
/// Deletes RECT's text and returns what was there, a string per line, with
/// point left at the corner.
fn cut<B: BufferTrait>(ctx: &EditorState<B>, rect: &Rectangle) -> Vec<String> {
    ctx.with_current_buffer_mut(|buf| {
        let taken = text_of(&buf.text, rect);
        each_line(buf, rect, |buf, span| {
            if !span.is_empty() {
                let _ = delete_range(buf, span.start, span.end);
            }
        });
        go_to_corner(buf, rect);
        taken
    })
}

pub const KILL_RECTANGLE_DOC: &str = "(kill-rectangle): Delete the block between point and the \
         mark, and keep it for `yank-rectangle', not on the kill ring. Return the number of \
         lines. Point ends at the top-left corner.";

primitive!(kill_rectangle, _args, _env, ctx, {
    let rect = ctx.with_current_buffer(require_rectangle)?;
    let taken = cut(ctx, &rect);
    let height = taken.len();
    ctx.kill_yank_mut(|kill_yank| kill_yank.set_rectangle(taken));
    Ok(ELispExp::number(height as f64))
});

pub const DELETE_RECTANGLE_DOC: &str = "(delete-rectangle): Delete the block between point and \
         the mark, keeping nothing. Return the number of lines. Point ends at the top-left \
         corner.";

primitive!(delete_rectangle, _args, _env, ctx, {
    let rect = ctx.with_current_buffer(require_rectangle)?;
    Ok(ELispExp::number(cut(ctx, &rect).len() as f64))
});

pub const COPY_RECTANGLE_AS_KILL_DOC: &str = "(copy-rectangle-as-kill): Keep the block between \
         point and the mark for `yank-rectangle', without deleting it. Return the number of \
         lines.";

primitive!(copy_rectangle_as_kill, _args, _env, ctx, {
    let taken = ctx.with_current_buffer(|buf| {
        let rect = require_rectangle(buf)?;
        Ok::<_, EvalError<EditorState<B>>>(text_of(&buf.text, &rect))
    })?;
    let height = taken.len();
    ctx.kill_yank_mut(|kill_yank| kill_yank.set_rectangle(taken));
    Ok(ELispExp::number(height as f64))
});

pub const KILLED_RECTANGLE_DOC: &str = "(killed-rectangle): Return the block kept by the last \
         rectangle kill or copy, as a list of strings, one per line, or nil when none is kept.";

primitive!(killed_rectangle, _args, _env, ctx, {
    Ok(match ctx.kill_yank(|kill_yank| kill_yank.rectangle()) {
        None => ELispExp::nil(),
        Some(lines) => ELispExp::proper_list(lines.into_iter().map(ELispExp::string).collect()),
    })
});

// ---------------------------------------------------------------------------
// Putting one in
// ---------------------------------------------------------------------------

/// Inserts LINES down the page from point, each at point's column, and
/// returns how many: what `yank-rectangle` does with the block kept. Point
/// stays where it was.
fn insert_block<B: BufferTrait>(ctx: &EditorState<B>, lines: &[String]) -> usize {
    if lines.is_empty() {
        return 0;
    }
    ctx.with_current_buffer_mut(|buf| {
        let (line, column) = buf.text.cursor_1d_to_2d(buf.text.cursor_pos_1d());
        // A rectangle of the block's height, at point's column, so the same
        // padding rule applies as everywhere else: a line the block reaches
        // past is filled out to the column first.
        let rect = Rectangle {
            top: line,
            bottom: line + lines.len() - 1,
            left: column,
            right: column,
        };
        // The buffer may not have enough lines for the block. Adding them is
        // part of yanking one: a block yanked at the end of a buffer must
        // arrive whole rather than be cut off by where the text happens to
        // stop.
        let missing = (line + lines.len()).saturating_sub(buf.text.line_count());
        if missing > 0 {
            let at = buf.text.len();
            let _ = insert_text(buf, at, &"\n".repeat(missing));
        }
        let mut placed = spans(&buf.text, &rect);
        placed.reverse();
        for span in &placed {
            let content = &lines[span.line - line];
            if content.is_empty() && span.padding > 0 {
                // Nothing to put here, so nothing is: padding a line out to a
                // column and then writing nothing at it leaves trailing
                // spaces that were never in the block.
                continue;
            }
            insert_at_edge(buf, span, content);
        }
        let back = buf.text.cursor_2d_to_1d(line, column);
        goto_offset(&mut buf.text, back);
        lines.len()
    })
}

pub const YANK_RECTANGLE_DOC: &str = "(yank-rectangle): Insert the block kept by the last \
         rectangle kill or copy, its top-left corner at point, pushing the text there to the \
         right. Return the number of lines, or nil when none is kept. Point does not move.";

primitive!(yank_rectangle, _args, _env, ctx, {
    let Some(lines) = ctx.kill_yank(|kill_yank| kill_yank.rectangle()) else {
        return Ok(ELispExp::nil());
    };
    Ok(ELispExp::number(insert_block(ctx, &lines) as f64))
});

pub const OPEN_RECTANGLE_DOC: &str = "(open-rectangle): Fill the block between point and the mark \
         with spaces, pushing its text to the right. Return the number of lines. Point ends at \
         the top-left corner.";

primitive!(open_rectangle, _args, _env, ctx, {
    let rect = ctx.with_current_buffer(require_rectangle)?;
    let filling = " ".repeat(rect.width());
    let height = ctx.with_current_buffer_mut(|buf| {
        let lines = spans(&buf.text, &rect).len();
        each_line(buf, &rect, |buf, span| {
            insert_at_edge(buf, span, &filling);
        });
        go_to_corner(buf, &rect);
        lines
    });
    Ok(ELispExp::number(height as f64))
});

pub const CLEAR_RECTANGLE_DOC: &str = "(clear-rectangle): Replace the text of the block between \
         point and the mark with spaces, so that the text to its right keeps its column. Return \
         the number of lines. Point ends at the top-left corner.";

primitive!(clear_rectangle, _args, _env, ctx, {
    let rect = ctx.with_current_buffer(require_rectangle)?;
    let height = ctx.with_current_buffer_mut(|buf| {
        let lines = spans(&buf.text, &rect).len();
        each_line(buf, &rect, |buf, span| {
            // `insert_text` at the span's own start, not `insert_at_edge`:
            // the one command that pads nothing, since it blanks what is
            // there, and a short line has nothing there to blank.
            if span.is_empty() {
                return;
            }
            let width = span.end - span.start;
            let _ = delete_range(buf, span.start, span.end);
            let _ = insert_text(buf, span.start, &" ".repeat(width));
        });
        go_to_corner(buf, &rect);
        lines
    });
    Ok(ELispExp::number(height as f64))
});

pub const STRING_RECTANGLE_DOC: &str = "(string-rectangle STRING): Replace the block between \
         point and the mark with STRING on every line; with a block of no width, insert STRING at \
         that column on every line. Return the number of lines. Point ends at the top-left \
         corner.";

primitive!(string_rectangle, args, _env, ctx, {
    let Some(ELispExp::String(text)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let content = text.to_string();
    let rect = ctx.with_current_buffer(require_rectangle)?;
    let height = ctx.with_current_buffer_mut(|buf| {
        let lines = spans(&buf.text, &rect).len();
        each_line(buf, &rect, |buf, span| {
            if !span.is_empty() {
                let _ = delete_range(buf, span.start, span.end);
            }
            insert_at_edge(buf, span, &content);
        });
        go_to_corner(buf, &rect);
        lines
    });
    Ok(ELispExp::number(height as f64))
});

/// Registers the commands on rectangles.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command(
        "rectangle-mark-mode",
        rectangle_mark_mode,
        &[],
        RECTANGLE_MARK_MODE_DOC,
    );
    into.command("kill-rectangle", kill_rectangle, &[], KILL_RECTANGLE_DOC);
    into.command(
        "delete-rectangle",
        delete_rectangle,
        &[],
        DELETE_RECTANGLE_DOC,
    );
    into.command(
        "copy-rectangle-as-kill",
        copy_rectangle_as_kill,
        &[],
        COPY_RECTANGLE_AS_KILL_DOC,
    );
    into.command("yank-rectangle", yank_rectangle, &[], YANK_RECTANGLE_DOC);
    into.command("open-rectangle", open_rectangle, &[], OPEN_RECTANGLE_DOC);
    into.command("clear-rectangle", clear_rectangle, &[], CLEAR_RECTANGLE_DOC);
    into.command(
        "string-rectangle",
        string_rectangle,
        &["sString rectangle: "],
        STRING_RECTANGLE_DOC,
    );
    into.function("rectangle-bounds", rectangle_bounds, RECTANGLE_BOUNDS_DOC);
    into.function("killed-rectangle", killed_rectangle, KILLED_RECTANGLE_DOC);
}
