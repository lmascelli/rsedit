//! Cutting, filling and moving blocks of text.
//!
//! The geometry is in [`crate::text::rectangle`]; this is the door Lisp comes
//! through, and the two rules the commands share.
//!
//! # Bottom line first, always
//!
//! Every one of these changes several lines, and a change to one line moves
//! every offset after it. So the spans are computed once, against the buffer
//! as it is, and then applied from the bottom up -- exactly as
//! [`super::comments`] applies its edits, and for exactly the same reason.
//! Computing them again as you go would work and would be one lock and one
//! walk per line.
//!
//! # Where point ends up
//!
//! The top-left corner, for every command that *changes* the buffer.
//!
//! It is where the block you just acted on begins, which is where the next
//! thing you do to it starts from, and it is the same answer after all of
//! them -- a set of commands that leave the cursor somewhere you have to look
//! for is a set you check after every use.
//!
//! `copy-rectangle-as-kill` is the exception, and has to be. It changes
//! nothing, so the mark survives it -- and moving point to the corner would
//! move it *onto* the mark and leave a selection with no size, so that the
//! next rectangle command found nothing to work on. `M-w` leaves point alone
//! for the same reason, and this is the same command.
use super::*;
use crate::buffer::{Buffer, Mark, mark::rectangle_corners};
use crate::primitives::edits::{delete_range, goto_offset, insert_text};
use crate::text::rectangle::{Rectangle, Span, spans, text_of};

/// The rectangle between point and the mark, or the error a command should
/// give when there is no mark.
///
/// The same message `region` gives, and for the same reason: a command that
/// quietly does nothing when the user thought they had a selection is worse
/// than one that says so.
fn require_rectangle<B: BufferTrait>(
    buf: &Buffer<B>,
) -> Result<Rectangle, EvalError<EditorState<B>>> {
    rectangle_corners(buf.mark, &buf.text).ok_or_else(|| {
        EvalError::RuntimeMessage("The mark is not set now, so there is no rectangle".into())
    })
}

/// Put point at the rectangle's top-left corner. See the module docs.
fn go_to_corner<B: BufferTrait>(buf: &mut Buffer<B>, rect: &Rectangle) {
    let at = buf.text.cursor_2d_to_1d(rect.top, rect.left);
    goto_offset(&mut buf.text, at);
}

/// Apply F to every line of RECT, bottom line first.
///
/// The spans are worked out before anything changes, so F sees the offsets
/// the line had when the rectangle was drawn. F is handed the line's span and
/// the rectangle, because the two things it may need -- what is there, and
/// how far short of the left edge the line stops -- are one each.
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

/// Insert CONTENT at the rectangle's left edge on SPAN's line, padding the
/// line out to that column first if it stops short of it.
///
/// The padding is what makes `open`, `yank` and `string-rectangle` work on
/// ragged text: they were asked to put something at a column, and a column
/// that does not exist yet is the case, not the exception.
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
    Ok(
        ctx.with_current_buffer(|buf| match rectangle_corners(buf.mark, &buf.text) {
            None => ELispExp::nil(),
            Some(rect) => ELispExp::proper_list(vec![
                ELispExp::number(rect.top as f64 + 1.0),
                ELispExp::number(rect.bottom as f64 + 1.0),
                ELispExp::number(rect.left as f64),
                ELispExp::number(rect.right as f64),
            ]),
        }),
    )
});

// ---------------------------------------------------------------------------
// Taking one out
// ---------------------------------------------------------------------------

/// Delete RECT's text, and hand back what was there if it was wanted.
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
    let (rect, taken) = ctx.with_current_buffer(|buf| {
        let rect = require_rectangle(buf)?;
        let taken = text_of(&buf.text, &rect);
        Ok::<_, EvalError<EditorState<B>>>((rect, taken))
    })?;
    let _ = rect;
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

/// Insert LINES down the page from point, each at point's column.
///
/// Shared by `yank-rectangle` and `string-rectangle`, which differ only in
/// where the lines come from -- one block from the store, or the same string
/// on every line of the marked block.
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
            // that is the whole of why this is the one command that does
            // not pad a short line. Going through the edge would fill the
            // line out to a column in order to blank something that was
            // never written there.
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

/// Register this module's primitives: the column-aligned block two corners describe.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Blocks of text. See `primitives::rectangle`, and `rectangle` for the
    // arithmetic of saying which characters a pair of columns covers when the
    // lines are not all long enough.
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
