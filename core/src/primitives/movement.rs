//! Moving point by characters, lines, words and paragraphs, to the ends of
//! the line and of the buffer, or to a position; and where point is. The
//! answers to where a word, a line or a paragraph ends are here, for the
//! commands that delete or change those units as well.
//!
//! Design: `doc/architettura-editor.typ, <movimento>`.
use super::*;
use crate::primitives::args::movement_count;

/// Moves point to OFFSET, clamped to the text.
pub(crate) fn goto_offset<B: BufferTrait>(text: &mut B, offset: usize) {
    let target = offset.min(text.len());
    let (line, col) = text.cursor_1d_to_2d(target);
    text.cursor_move(line, col);
}

/// Returns whether C belongs to a word: a letter, a digit or an underscore,
/// which is what most modes mean, with no syntax table to ask.
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Returns whether LINE has nothing but whitespace on it, which is what
/// separates paragraphs; a line past the end is blank.
pub(crate) fn is_blank_line<B: BufferTrait>(text: &B, line: usize) -> bool {
    text.get_lines(line, line + 1)
        .first()
        .map(|l| l.trim().is_empty())
        .unwrap_or(true)
}

/// Returns the offset COUNT words forward of FROM, each word reached past the
/// characters that are not part of one and left at its end.
///
/// `forward-word`, `kill-word` and the verbs all ask here, so that a deletion
/// takes exactly what the motion crosses.
pub(crate) fn word_forward<B: BufferTrait>(text: &B, from: usize, count: usize) -> usize {
    let len = text.len();
    let mut pos = from;
    for _ in 0..count {
        if pos >= len {
            break;
        }
        while pos < len && !text.at(pos).map(is_word_char).unwrap_or(false) {
            pos += 1;
        }
        while pos < len && text.at(pos).map(is_word_char).unwrap_or(false) {
            pos += 1;
        }
    }
    pos
}

/// Returns the offset COUNT words back of FROM, at the start of the last word
/// reached.
pub(crate) fn word_backward<B: BufferTrait>(text: &B, from: usize, count: usize) -> usize {
    let mut pos = from;
    for _ in 0..count {
        if pos == 0 {
            break;
        }
        while pos > 0 && !text.at(pos - 1).map(is_word_char).unwrap_or(false) {
            pos -= 1;
        }
        while pos > 0 && text.at(pos - 1).map(is_word_char).unwrap_or(false) {
            pos -= 1;
        }
    }
    pos
}

/// Returns the line `forward-paragraph` lands on from LINE: past any blank
/// lines, then to the blank line after the paragraph, or to the last line.
pub(super) fn paragraph_forward<B: BufferTrait>(text: &B, line: usize) -> usize {
    let last = text.line_count().saturating_sub(1);
    let mut line = line;
    while line < last && is_blank_line(text, line) {
        line += 1;
    }
    while line < last && !is_blank_line(text, line) {
        line += 1;
    }
    line
}

/// Returns the line `backward-paragraph` lands on from LINE: back past any
/// blank lines, then to the blank line before the paragraph, or to the first.
pub(super) fn paragraph_backward<B: BufferTrait>(text: &B, line: usize) -> usize {
    let mut line = line;
    while line > 0 && is_blank_line(text, line) {
        line -= 1;
    }
    while line > 0 && !is_blank_line(text, line) {
        line -= 1;
    }
    line
}

/// Returns how many characters LINE has, its newline not counted; 0 for a
/// line past the end.
pub(crate) fn line_length<B: BufferTrait>(text: &B, line: usize) -> usize {
    text.get_lines(line, line + 1)
        .first()
        .map(|l| l.chars().count())
        .unwrap_or(0)
}

/// Returns the offsets where LINE starts and ends, its newline not counted.
///
/// A line past the end of the buffer gives the clamped offset twice, so the
/// span is empty rather than invalid.
pub(crate) fn line_bounds<B: BufferTrait>(text: &B, line: usize) -> (usize, usize) {
    let start = text.cursor_2d_to_1d(line, 0);
    (start, start + line_length(text, line))
}

// ---------------------------------------------------------------------------
// Characters and lines
// ---------------------------------------------------------------------------

pub const FORWARD_CHAR_DOC: &str = "(forward-char &optional N): Move point forward N characters, \
         1 by default, across line ends, stopping at the end of the buffer. A negative N moves \
         back.";

primitive!(forward_char, args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        let step = movement_count(args)?;
        // By offset, so that it crosses a line end as C-f does.
        let target = buf.text.cursor_pos_1d().saturating_add_signed(step);
        goto_offset(&mut buf.text, target);
        Ok(ELispExp::nil())
    })
});

pub const BACKWARD_CHAR_DOC: &str = "(backward-char &optional N): Move point back N characters, 1 \
         by default, across line ends, stopping at the start of the buffer. A negative N moves \
         forward.";

primitive!(backward_char, args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        let step = movement_count(args)?.saturating_neg();
        let target = buf.text.cursor_pos_1d().saturating_add_signed(step);
        goto_offset(&mut buf.text, target);
        Ok(ELispExp::nil())
    })
});

pub const PREVIOUS_LINE_DOC: &str = "(previous-line &optional N): Move point up N lines, 1 by \
         default, keeping its column where the line is long enough, and stopping at the first \
         line. A negative N moves down.";

primitive!(previous_line, args, _env, ctx, {
    move_line(ctx, movement_count(args)?.saturating_neg())
});

pub const NEXT_LINE_DOC: &str = "(next-line &optional N): Move point down N lines, 1 by default, \
         keeping its column where the line is long enough. A negative N moves up.";

primitive!(next_line, args, _env, ctx, {
    move_line(ctx, movement_count(args)?)
});

/// Moves point DELTA lines down, up when negative, keeping the column the
/// user is aiming for.
///
/// The goal column is kept across a run of vertical moves, and read again
/// from point when the command before was something else (`last-command`):
/// read on every move, a short line on the way would clamp it for good.
fn move_line<B: BufferTrait>(
    ctx: &EditorState<B>,
    delta: isize,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    // Both asked before the buffer is locked: they are other compartments,
    // and the closure holds the buffer's work and nothing else.
    let continuing = ctx.last_command_is("next-line") || ctx.last_command_is("previous-line");
    let remembered = ctx.goal_column();
    let goal = ctx.with_current_buffer_mut(|buf| {
        let (line, col) = buf.text.cursor_pos();
        let goal = match (continuing, remembered) {
            (true, Some(goal)) => goal,
            _ => col,
        };

        let last_line = buf.text.line_count().saturating_sub(1);
        let target = if delta < 0 {
            line.saturating_sub(delta.unsigned_abs())
        } else {
            line.saturating_add(delta as usize).min(last_line)
        };
        // `cursor_move` clamps the column to the line, so a short line in the
        // middle of a run does not disturb the goal we are steering by.
        buf.text.cursor_move(target, goal);
        goal
    });
    ctx.set_goal_column(Some(goal));
    Ok(ELispExp::nil())
}

pub const BEGINNING_OF_LINE_DOC: &str = "(beginning-of-line): Move point to the start of the line \
         it is on.";

primitive!(beginning_of_line, _args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        let (line, _) = buf.text.cursor_pos();
        buf.text.cursor_move(line, 0);
        Ok(ELispExp::nil())
    })
});

pub const END_OF_LINE_DOC: &str = "(end-of-line): Move point to the end of the line it is on, \
         before the newline.";

primitive!(end_of_line, _args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        let (line, _) = buf.text.cursor_pos();
        let end = line_length(&buf.text, line);
        buf.text.cursor_move(line, end);
        Ok(ELispExp::nil())
    })
});

// ---------------------------------------------------------------------------
// Words, paragraphs and the whole buffer
// ---------------------------------------------------------------------------

pub const FORWARD_WORD_DOC: &str = "(forward-word &optional N): Move point past the end of the \
         Nth word after it, 1 by default; a negative N moves back. A word is a run of letters, \
         digits and underscores.";

primitive!(forward_word, args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        let count = movement_count(args)?;
        let from = buf.text.cursor_pos_1d();
        let target = if count < 0 {
            word_backward(&buf.text, from, count.unsigned_abs())
        } else {
            word_forward(&buf.text, from, count as usize)
        };
        goto_offset(&mut buf.text, target);
        Ok(ELispExp::nil())
    })
});

pub const BACKWARD_WORD_DOC: &str = "(backward-word &optional N): Move point back to the start of \
         the Nth word before it, 1 by default; a negative N moves forward.";

primitive!(backward_word, args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        let count = movement_count(args)?;
        let from = buf.text.cursor_pos_1d();
        let target = if count < 0 {
            word_forward(&buf.text, from, count.unsigned_abs())
        } else {
            word_backward(&buf.text, from, count as usize)
        };
        goto_offset(&mut buf.text, target);
        Ok(ELispExp::nil())
    })
});

pub const FORWARD_PARAGRAPH_DOC: &str = "(forward-paragraph &optional N): Move point forward N \
         paragraphs, 1 by default, to the blank line after each, or to the last line. A negative \
         N moves back.";

primitive!(forward_paragraph, args, _env, ctx, {
    move_paragraph(ctx, movement_count(args)?)
});

pub const BACKWARD_PARAGRAPH_DOC: &str = "(backward-paragraph &optional N): Move point back N \
         paragraphs, 1 by default, to the blank line before each, or to the first line. A \
         negative N moves forward.";

primitive!(backward_paragraph, args, _env, ctx, {
    move_paragraph(ctx, movement_count(args)?.saturating_neg())
});

/// Moves point DELTA paragraphs forward, back when negative, to the start of
/// the line the last one lands on; at either end of the buffer it stops.
fn move_paragraph<B: BufferTrait>(
    ctx: &EditorState<B>,
    delta: isize,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    if delta == 0 {
        return Ok(ELispExp::nil());
    }
    ctx.with_current_buffer_mut(|buf| {
        let mut line = buf.text.cursor_pos().0;
        for _ in 0..delta.unsigned_abs() {
            let target = if delta < 0 {
                paragraph_backward(&buf.text, line)
            } else {
                paragraph_forward(&buf.text, line)
            };
            if target == line {
                break;
            }
            line = target;
        }
        buf.text.cursor_move(line, 0);
        Ok(ELispExp::nil())
    })
}

pub const BEGINNING_OF_BUFFER_DOC: &str = "(beginning-of-buffer): Move point to the start of the \
         buffer.";

primitive!(beginning_of_buffer, _args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        buf.text.cursor_move(0, 0);
        Ok(ELispExp::nil())
    })
});

pub const END_OF_BUFFER_DOC: &str = "(end-of-buffer): Move point to the end of the buffer.";

primitive!(end_of_buffer, _args, _env, ctx, {
    ctx.with_current_buffer_mut(|buf| {
        let end = buf.text.len();
        goto_offset(&mut buf.text, end);
        Ok(ELispExp::nil())
    })
});

pub const GOTO_LINE_DOC: &str = "(goto-line N): Move point to the start of line N, counting from \
         1; a number past the end goes to the last line. M-x goto-line asks for N.\n\n\
         Example:\n\
         (goto-line 42)";

primitive!(goto_line, args, _env, ctx, {
    let requested = match args.first() {
        Some(ELispExp::Number(n)) => *n,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.clone(),
            });
        }
        None => {
            return Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: 0,
            });
        }
    };
    ctx.with_current_buffer_mut(|buf| {
        // 1-based for the user, 0-based inside, and clamped so that a number
        // past the end lands on the last line rather than failing.
        let last = buf.text.line_count().saturating_sub(1);
        let line = (requested.max(1.0) as usize - 1).min(last);
        buf.text.cursor_move(line, 0);
        Ok(ELispExp::nil())
    })
});

// ---------------------------------------------------------------------------
// Point, as a number
// ---------------------------------------------------------------------------
//
// For code that has computed a position rather than described a move: a
// search result, a saved place. In characters, as point, the mark, the
// region and the undo history all are.

pub const LINE_NUMBER_AT_POINT_DOC: &str = "(line-number-at-point): Return the number of the line \
         point is on, counting from 1 as `goto-line' does.\n\n\
         Example:\n\
         (goto-line (+ (line-number-at-point) 10))";

primitive!(line_number_at_point, _args, _env, ctx, {
    ctx.with_current_buffer(|buf| {
        let point = buf.text.cursor_pos_1d();
        let line = buf.text.cursor_1d_to_2d(point).0;
        Ok(ELispExp::number(line as f64 + 1.0))
    })
});

pub const CURRENT_LINE_DOC: &str = "(current-line): Return the text of the line point is on, \
         without its newline, or \"\" in an empty buffer.\n\n\
         Example:\n\
         (current-line) => \"src/\"";

primitive!(current_line, _args, _env, ctx, {
    ctx.with_current_buffer(|buf| {
        let point = buf.text.cursor_pos_1d();
        let line = buf.text.cursor_1d_to_2d(point).0;
        let text = buf
            .text
            .get_lines(line, line + 1)
            .into_iter()
            .next()
            .unwrap_or_default();
        Ok(ELispExp::string(text))
    })
});

pub const POINT_DOC: &str = "(point): Return the position of point: the number of characters \
         before it, so the first position is 0.";

primitive!(point, _args, _env, ctx, {
    ctx.with_current_buffer(|buf| Ok(ELispExp::number(buf.text.cursor_pos_1d() as f64)))
});

pub const POINT_MIN_DOC: &str = "(point-min): Return the first position in the buffer, which is \
         0.";

primitive!(point_min, _args, _env, _ctx, { Ok(ELispExp::number(0.0)) });

pub const POINT_MAX_DOC: &str = "(point-max): Return the position after the last character in the \
         buffer.";

primitive!(point_max, _args, _env, ctx, {
    ctx.with_current_buffer(|buf| Ok(ELispExp::number(buf.text.len() as f64)))
});

pub const GOTO_CHAR_DOC: &str = "(goto-char POSITION): Move point to POSITION, a number of \
         characters from the start of the buffer, and return where it went. A position outside \
         the buffer goes to its nearer end.\n\n\
         Example:\n\
         (goto-char (point-max))";

primitive!(goto_char, args, _env, ctx, {
    let requested = match args.first() {
        Some(ELispExp::Number(n)) => *n,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.clone(),
            });
        }
        None => {
            return Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: 0,
            });
        }
    };
    Ok(ELispExp::number(ctx.with_current_buffer_mut(|buf| {
        // Clamped at both ends: a position outside the buffer is almost
        // always one computed before an edit, and putting point somewhere
        // real is more useful than refusing.
        let target = requested.max(0.0) as usize;
        let target = target.min(buf.text.len());
        let (line, col) = buf.text.cursor_1d_to_2d(target);
        buf.text.cursor_move(line, col);
        target as f64
    })))
});

pub const CURRENT_COLUMN_DOC: &str = "(current-column): Return the column point is on, counting \
         from 0. Every character is one column, a tab included.";

primitive!(current_column, _args, _env, ctx, {
    ctx.with_current_buffer(|buf| Ok(ELispExp::number(buf.text.cursor_pos().1 as f64)))
});

/// Registers the primitives that move point and say where it is.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("backward-char", backward_char, &["p"], BACKWARD_CHAR_DOC);
    into.command("forward-char", forward_char, &["p"], FORWARD_CHAR_DOC);
    into.command("previous-line", previous_line, &["p"], PREVIOUS_LINE_DOC);
    into.command("next-line", next_line, &["p"], NEXT_LINE_DOC);
    into.command(
        "beginning-of-line",
        beginning_of_line,
        &[],
        BEGINNING_OF_LINE_DOC,
    );
    into.command("end-of-line", end_of_line, &[], END_OF_LINE_DOC);
    into.command("forward-word", forward_word, &["p"], FORWARD_WORD_DOC);
    into.command("backward-word", backward_word, &["p"], BACKWARD_WORD_DOC);
    into.command(
        "forward-paragraph",
        forward_paragraph,
        &["p"],
        FORWARD_PARAGRAPH_DOC,
    );
    into.command(
        "backward-paragraph",
        backward_paragraph,
        &["p"],
        BACKWARD_PARAGRAPH_DOC,
    );
    into.command(
        "beginning-of-buffer",
        beginning_of_buffer,
        &[],
        BEGINNING_OF_BUFFER_DOC,
    );
    into.command("end-of-buffer", end_of_buffer, &[], END_OF_BUFFER_DOC);
    into.command("goto-line", goto_line, &["nGoto line: "], GOTO_LINE_DOC);
    // Point as a number: functions, since nothing is usefully reached by
    // typing `M-x point', and every move by a described amount is a command.
    into.function("point", point, POINT_DOC);
    into.function("point-min", point_min, POINT_MIN_DOC);
    into.function("point-max", point_max, POINT_MAX_DOC);
    into.function("goto-char", goto_char, GOTO_CHAR_DOC);
    into.function(
        "line-number-at-point",
        line_number_at_point,
        LINE_NUMBER_AT_POINT_DOC,
    );
    into.function("current-line", current_line, CURRENT_LINE_DOC);
    into.function("current-column", current_column, CURRENT_COLUMN_DOC);
}
