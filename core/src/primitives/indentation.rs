//! Indentation's language-independent machinery: asking a mode for a column,
//! cycling its alternatives on consecutive Tabs, and editing through the two
//! doors. Language rules remain Lisp functions which only read the buffer.
//!
//! Design: `doc/architettura-editor.typ, <espressioni-indentazione>`.
use super::*;
use crate::primitives::edits::{delete_range, edited, insert_text};
use crate::primitives::movement::{goto_offset, is_blank_line};
use risp::call_callable;

type IndentResult<B> = Result<ELispExp<B>, EvalError<EditorState<B>>>;

/// A rule may move point while reading, including on its error path. Never
/// keep the buffer's lock across Lisp: the rule needs to read it itself.
fn call_rule<B: BufferTrait>(
    rule: &ELispExp<B>,
    env: std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> IndentResult<B> {
    let point = ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d());
    let result = call_callable(rule, &[], env, ctx);
    ctx.with_current_buffer_mut(|buf| goto_offset(&mut buf.text, point));
    result
}

fn column<B: BufferTrait>(value: &ELispExp<B>) -> Result<usize, EvalError<EditorState<B>>> {
    match value {
        ELispExp::Number(n) => args::width(*n),
        other => Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: other.clone(),
        }),
    }
}

/// Returns whether the current buffer's mode has an indentation rule of its
/// own, an `indent-function`: without one, Tab works as Emacs'
/// `indent-relative` rather than reindenting.
fn has_rule<B: BufferTrait>(
    env: &std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> bool {
    let mode = ctx.with_current_buffer(|buf| buf.current_mode.clone());
    env.get_property(&mode, "indent-function")
        .is_some_and(|rule| rule.is_truthy())
}

/// Returns the first tab stop past COLUMN: the next multiple of WIDTH.
fn next_tab_stop(column: usize, width: usize) -> usize {
    let width = width.max(1);
    column + width - column % width
}

fn preferred_column<B: BufferTrait>(
    env: &std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> Result<usize, EvalError<EditorState<B>>> {
    let mode = ctx.with_current_buffer(|buf| buf.current_mode.clone());
    let value = match env.get_property(&mode, "indent-function") {
        Some(rule) if rule.is_truthy() => call_rule(&rule, env.clone(), ctx)?,
        _ => previous_indentation(&[], env.clone(), ctx)?,
    };
    column(&value)
}

/// Alternatives are opt-in: braces and string contents must not acquire
/// arbitrary outer indentation levels just because a language uses spaces.
/// An empty list disables cycling for this particular context.
fn alternative_columns<B: BufferTrait>(
    preferred: usize,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> Result<Option<Vec<usize>>, EvalError<EditorState<B>>> {
    let mode = ctx.with_current_buffer(|buf| buf.current_mode.clone());
    let Some(rule) = env
        .get_property(&mode, "indent-levels-function")
        .filter(|rule| rule.is_truthy())
    else {
        return Ok(None);
    };
    let value = call_rule(&rule, env.clone(), ctx)?;
    let items = match &value {
        ELispExp::Form(items) => items.to_vec(),
        _ => {
            let (items, tail) = value.split_list();
            if !tail.is_nil() {
                return Err(EvalError::WrongArgumentType {
                    expected: "proper list of indentation columns".into(),
                    got: value,
                });
            }
            items
        }
    };
    if items.is_empty() {
        return Ok(None);
    }
    let mut columns = Vec::with_capacity(items.len() + 1);
    for item in items {
        match item {
            ELispExp::Number(n) if n.is_finite() && n >= 0.0 && n.fract() == 0.0 => {
                columns.push(args::width(n)?);
            }
            other => {
                return Err(EvalError::WrongArgumentType {
                    expected: "non-negative integer indentation column".into(),
                    got: other,
                });
            }
        }
    }
    columns.push(preferred);
    columns.sort_unstable_by(|a, b| b.cmp(a));
    columns.dedup();
    Ok(Some(columns))
}

pub const INDENT_WIDTH_DOC: &str = "(indent-width): Return the mode's tab-width property, or \
         the variable tab-width when the mode has none.";

primitive!(indent_width, args, env, ctx, {
    exact_arity(args, 0)?;
    let mode = ctx.with_current_buffer(|buf| buf.current_mode.clone());
    Ok(env
        .get_property(&mode, "tab-width")
        .filter(|value| value.is_truthy())
        .or_else(|| env.get_variable("tab-width"))
        .unwrap_or_else(|| ELispExp::number(4.0)))
});

pub const INDENT_LINE_DOC: &str = "(indent-line): Indent the current line at the column its \
         mode's indent-function returns, or like the previous nonblank line. Return t if the \
         line changed, nil otherwise. Never cycle indentation levels.";

primitive!(indent_line, args, env, ctx, {
    exact_arity(args, 0)?;
    let target = preferred_column(&env, ctx)?;
    indent_line_to(&[ELispExp::number(target as f64)], env, ctx)
});

fn insert_padding<B: BufferTrait>(
    env: std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> IndentResult<B> {
    let width = column(&indent_width(&[], env, ctx)?)?;
    let happened = ctx.with_current_buffer_mut(|buf| {
        let at = buf.text.cursor_pos_1d();
        // Up to the next tab stop, as Emacs' `insert-tab': text after a Tab
        // lines up with text after another, wherever each began.
        let col = buf.text.cursor_pos().1;
        let count = next_tab_stop(col, width) - col;
        let padding = " ".repeat(count);
        if !insert_text(buf, at, &padding) {
            return false;
        }
        goto_offset(&mut buf.text, at + count);
        true
    });
    Ok(edited(ctx, happened))
}

pub const INDENT_FOR_TAB_COMMAND_DOC: &str = "(indent-for-tab-command): Indent the region or \
         current line where the mode's indent-function says, wherever point is on it, as Emacs \
         does with tab-always-indent t, the default. Consecutive Tabs cycle the mode's \
         indent-levels-function columns from right to left, wrapping to the maximum; other \
         commands end the cycle. With complete, try completion when indentation and point did \
         not change. With nil, insert spaces up to the next tab stop after text, and on a \
         repeated Tab without alternatives. A mode with no indent-function indents as Emacs' \
         indent-relative: in the leading whitespace, up to the previous line's indentation or \
         else the next tab stop, never less than the line has; after text, spaces up to the \
         next tab stop.";

primitive!(indent_for_tab_command, args, env, ctx, {
    exact_arity(args, 0)?;
    if super::region::use_region_p(&[], env.clone(), ctx)?.is_truthy() {
        return call_callable(&ELispExp::symbol("indent-region".into()), &[], env, ctx);
    }
    let (inside, existing, point) = ctx.with_current_buffer(|buf| {
        let (line, col) = buf.text.cursor_pos();
        let indent = indentation_of(&buf.text, line);
        (col <= indent, indent, buf.text.cursor_pos_1d())
    });
    // t when indent.lisp has not said otherwise, as in Emacs.
    let policy = env
        .get_variable("tab-always-indent")
        .unwrap_or_else(ELispExp::t);
    let complete = matches!(&policy, ELispExp::Symbol(name) if name.as_str() == "complete");

    // No rule to ask: the previous line is all there is to go by, and a line
    // indented past it is not to be pulled back to it, as it was when this
    // copied the previous line's indentation whatever the line had.
    if !has_rule(&env, ctx) {
        if !inside {
            return if complete {
                call_callable(
                    &ELispExp::symbol("completion-at-point".into()),
                    &[],
                    env,
                    ctx,
                )
            } else {
                insert_padding(env, ctx)
            };
        }
        let previous = column(&previous_indentation(&[], env.clone(), ctx)?)?;
        let width = column(&indent_width(&[], env.clone(), ctx)?)?;
        let target = if existing < previous {
            previous
        } else {
            next_tab_stop(existing, width)
        };
        return indent_line_to(&[ELispExp::number(target as f64)], env, ctx);
    }

    if policy.is_nil() && !inside {
        return insert_padding(env, ctx);
    }

    let preferred = preferred_column(&env, ctx)?;
    let alternatives = alternative_columns(preferred, &env, ctx)?;
    let repeated = ctx.last_command_is("indent-for-tab-command");
    if policy.is_nil() && repeated && alternatives.is_none() {
        return insert_padding(env, ctx);
    }
    let target = match &alternatives {
        Some(levels) if repeated => levels
            .iter()
            .copied()
            .find(|level| *level < existing)
            .unwrap_or(levels[0]),
        _ => preferred,
    };
    let changed = indent_line_to(&[ELispExp::number(target as f64)], env.clone(), ctx)?;
    if changed.is_truthy() || (inside && alternatives.is_some()) {
        return Ok(changed);
    }
    let same_point = ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d() == point);
    if same_point && complete {
        return call_callable(
            &ELispExp::symbol("completion-at-point".into()),
            &[],
            env,
            ctx,
        );
    }
    Ok(changed)
});

pub const NEWLINE_AND_INDENT_DOC: &str = "(newline-and-indent): Delete horizontal whitespace \
         just before point, insert a newline, and indent the new line using the mode's rule. \
         From a blank line, keep its chosen indentation. Never cycle indentation levels.";

primitive!(newline_and_indent, args, env, ctx, {
    exact_arity(args, 0)?;
    let (happened, keep) = ctx.with_current_buffer_mut(|buf| {
        let (line, _) = buf.text.cursor_pos();
        let keep = is_blank_line(&buf.text, line).then(|| indentation_of(&buf.text, line));
        let point = buf.text.cursor_pos_1d();
        let start = buf.text.cursor_2d_to_1d(line, 0);
        let spaces = buf
            .text
            .slice(start, point)
            .chars()
            .rev()
            .take_while(|c| *c == ' ' || *c == '\t')
            .count();
        let at = point - spaces;
        if !delete_range(buf, at, point) || !insert_text(buf, at, "\n") {
            return (false, keep);
        }
        goto_offset(&mut buf.text, at + 1);
        (true, keep)
    });
    if !happened {
        return Ok(edited(ctx, false));
    }
    match keep {
        Some(column) => indent_line_to(&[ELispExp::number(column as f64)], env, ctx),
        None => indent_line(&[], env, ctx),
    }
});

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
         changed, nil if it was already so; a COLUMN past a million is out of range. Point keeps its character, or goes to the end of the \
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
    // Bounded: COLUMN spaces are about to be made into one string.
    let column = crate::primitives::args::width(*column)?;

    // `None` when the line has that indentation already, `Some(false)` when
    // the buffer refused, `Some(true)` when it changed: the closure answers,
    // and the answer is reported once the buffer's lock is given back.
    let outcome = ctx.with_current_buffer_mut(|buf| {
        let (line, _) = buf.text.cursor_pos();
        let start = buf.text.cursor_2d_to_1d(line, 0);
        let width = indentation_of(&buf.text, line);
        if width == column {
            if buf.text.cursor_pos_1d() < start + width {
                goto_offset(&mut buf.text, start + width);
            }
            return None;
        }

        let point = buf.text.cursor_pos_1d();
        if !delete_range(buf, start, start + width) {
            return Some(false);
        }
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
    into.function("indent-width", indent_width, INDENT_WIDTH_DOC);
    into.command("indent-line", indent_line, &[], INDENT_LINE_DOC);
    into.command(
        "indent-for-tab-command",
        indent_for_tab_command,
        &[],
        INDENT_FOR_TAB_COMMAND_DOC,
    );
    into.command(
        "newline-and-indent",
        newline_and_indent,
        &[],
        NEWLINE_AND_INDENT_DOC,
    );
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
