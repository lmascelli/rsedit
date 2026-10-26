//! Reading a line's indentation and rewriting it. Which column a line should
//! start at is a question about a language, which `indent.lisp` asks; what is
//! here is the reading and the rewriting, through the two doors.
//!
//! Design: `doc/architettura-editor.typ, <espressioni-indentazione>`.
use super::*;
use crate::primitives::edits::{delete_range, edited, insert_text};
use crate::primitives::movement::is_blank_line;

/// Returns the column of the first character on LINE that is not a space or
/// a tab.
///
/// A line with nothing else on it is indented all the way across, as Emacs
/// reports it, so that `indent-line-to` does not keep its trailing whitespace
/// as text.
fn indentation_of<B: BufferTrait>(text: &B, line: usize) -> usize {
    text.get_lines(line, line + 1)
        .first()
        .map(|line| line.chars().take_while(|c| *c == ' ' || *c == '\t').count())
        .unwrap_or(0)
}

/// Returns the line the first of ARGS names, counting from 1, or the line
/// point is on when it names none.
fn line_argument<B: BufferTrait>(args: &[ELispExp<B>], text: &B) -> usize {
    match args.first() {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 1.0 => {
            (*n as usize - 1).min(text.line_count().saturating_sub(1))
        }
        _ => text.cursor_pos().0,
    }
}

pub const CURRENT_INDENTATION_DOC: &str = "(current-indentation &optional LINE): Return how many \
         spaces and tabs begin LINE, numbered from 1 as `goto-line' numbers lines, or the line \
         point is on. A blank line counts as indented all the way across.\n\n\
         Example:\n\
         (current-indentation 1)";

primitive!(current_indentation, args, _env, ctx, {
    ctx.with_current_buffer(|buf| {
        let line = line_argument(args, &buf.text);
        Ok(ELispExp::number(indentation_of(&buf.text, line) as f64))
    })
});

pub const PREVIOUS_INDENTATION_DOC: &str = "(previous-indentation): Return the indentation of the \
         nearest line above point's that is not blank, or 0 when there is none. A line in a mode \
         without an `indent-function' is indented to this.";

primitive!(previous_indentation, _args, _env, ctx, {
    ctx.with_current_buffer(|buf| {
        let mut line = buf.text.cursor_pos().0;
        while line > 0 {
            line -= 1;
            if !is_blank_line(&buf.text, line) {
                return Ok(ELispExp::number(indentation_of(&buf.text, line) as f64));
            }
        }
        Ok(ELispExp::number(0.0))
    })
});

pub const INDENT_LINE_TO_DOC: &str = "(indent-line-to COLUMN): Replace the spaces and tabs that \
         begin the line point is on with COLUMN spaces, as one undo step. Return t if the line \
         changed, nil if it was already so. Point keeps its character, or goes to the end of the \
         new indentation when it was inside the old.\n\n\
         Example:\n\
         (indent-line-to (previous-indentation))";

primitive!(indent_line_to, args, _env, ctx, {
    let Some(ELispExp::Number(column)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let column = column.max(0.0) as usize;

    // `None` when the line has that indentation already, `Some(false)` when
    // the buffer refused, `Some(true)` when it changed: the closure answers,
    // and the answer is reported once the buffer's lock is given back.
    let outcome = ctx.with_current_buffer_mut(|buf| {
        let (line, _) = buf.text.cursor_pos();
        let start = buf.text.cursor_2d_to_1d(line, 0);
        let width = indentation_of(&buf.text, line);
        if width == column {
            return None;
        }

        let point = buf.text.cursor_pos_1d();
        if !delete_range(buf, start, start + width) {
            return Some(false);
        }
        // TODO(CMD-3): COLUMN is not bounded, and a huge one is an allocation
        // that ends the process.
        let padding: String = " ".repeat(column);
        if !insert_text(buf, start, &padding) {
            return Some(false);
        }

        // Where point ends up. Inside the indentation it has no character of
        // its own to keep, so it goes to the end of the new one; past it, it
        // keeps whatever it was sitting on, which has moved by the difference.
        let target = if point <= start + width {
            start + column
        } else {
            (point + column).saturating_sub(width)
        };
        let target = target.min(buf.text.len());
        let (line, col) = buf.text.cursor_1d_to_2d(target);
        buf.text.cursor_move(line, col);
        Some(true)
    });
    // Out here, with the buffer's lock given back: `edited` puts a message in
    // the echo area, which is another compartment.
    Ok(match outcome {
        None => ELispExp::nil(),
        Some(false) => edited(ctx, false),
        Some(true) => ELispExp::t(),
    })
});

/// Registers the primitives that read and rewrite indentation.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function(
        "current-indentation",
        current_indentation,
        CURRENT_INDENTATION_DOC,
    );
    into.function(
        "previous-indentation",
        previous_indentation,
        PREVIOUS_INDENTATION_DOC,
    );
    into.function("indent-line-to", indent_line_to, INDENT_LINE_TO_DOC);
}
