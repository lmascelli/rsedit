//! Filling from Lisp: `fill-paragraph`, `fill-region`, `set-fill-column` and
//! `do-auto-fill`, with the paragraph, the comment and their prefixes worked
//! out from the buffer and its mode. How words are laid out, and where a line
//! breaks, is [`crate::text::fill`].
//!
//! Design: `doc/architettura-editor.typ, <riempimento>`.
use super::*;
use crate::buffer::Buffer;
use crate::modes::CommentStyle;
use crate::primitives::edits::{delete_range, insert_text};
use crate::primitives::movement::{goto_offset, line_bounds};
use crate::text::fill::{Measure, break_at, comment_prefix, indentation, lay_out};
use std::sync::Arc;

/// The name of the Lisp variable, and of the mode's property and the buffer's
/// value, holding the column lines are filled to.
pub(crate) const FILL_COLUMN: &str = "fill-column";

/// The column lines are filled to when nothing says otherwise: Emacs' own.
pub(crate) const DEFAULT_FILL_COLUMN: usize = 70;

/// The key a buffer's own fill column is kept under, the one
/// `(buffer-get 'fill-column)` reads.
fn buffer_key() -> String {
    super::buffers::lisp_key(FILL_COLUMN)
}

/// Returns VALUE as a fill column: a number of at least one.
fn column_value<B: BufferTrait>(value: Option<ELispExp<B>>) -> Option<usize> {
    match value {
        Some(ELispExp::Number(n)) if n.is_finite() && n >= 1.0 => Some(n as usize),
        _ => None,
    }
}

/// How the current buffer is filled: the measure, and the openers of its
/// mode's line comments, none for a mode that has no comments.
struct Setup {
    measure: Measure<fn(char) -> usize>,
    openers: Vec<String>,
}

/// Returns how the current buffer is filled: to its own `fill-column`, set by
/// `set-fill-column`, or its mode's property, or the variable, or 70.
///
/// One lock at a time: the buffer's, then the environment's, then the modes'.
fn setup<B: BufferTrait>(ctx: &EditorState<B>, env: &Arc<Env<EditorState<B>>>) -> Setup {
    let key = buffer_key();
    let (mode, own) = ctx.with_current_buffer(|buf| {
        (
            buf.current_mode.clone(),
            buf.data::<ELispExp<B>>(&key).cloned(),
        )
    });
    let width = column_value(own)
        .or_else(|| column_value(env.get_property(&mode, FILL_COLUMN)))
        .or_else(|| column_value(env.get_variable(FILL_COLUMN)))
        .unwrap_or(DEFAULT_FILL_COLUMN);
    let tab = crate::editor::tab_widths(env, [mode.clone()]).of(&mode);
    let openers = ctx
        .syntax_table(&mode)
        .comments()
        .iter()
        .filter_map(|style| match style {
            CommentStyle::Line { opener } => Some(opener.clone()),
            CommentStyle::Block { .. } => None,
        })
        .collect();
    Setup {
        measure: Measure {
            width,
            tab,
            cells: crate::ui::width::char_cells,
        },
        openers,
    }
}

/// What a line is to filling. A mode with comments is code, and only its
/// comments are filled; in one without, every line is text.
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    /// Nothing to fill: blank, or a comment with nothing after its opener.
    Blank,
    /// Code, which is never filled.
    Code,
    /// Text behind its indentation.
    Text(String),
    /// A comment's text behind its prefix.
    Comment(String),
}

impl Kind {
    /// Returns what LINE is, in a buffer whose mode comments with OPENERS.
    fn of(line: &str, openers: &[String]) -> Kind {
        if line.trim().is_empty() {
            return Kind::Blank;
        }
        if let Some(prefix) = comment_prefix(line, openers) {
            if line[prefix.len()..].trim().is_empty() {
                return Kind::Blank;
            }
            return Kind::Comment(prefix.to_string());
        }
        if !openers.is_empty() {
            return Kind::Code;
        }
        Kind::Text(indentation(line).to_string())
    }

    /// Returns the prefix the line's words are behind.
    fn prefix(&self) -> &str {
        match self {
            Kind::Text(prefix) | Kind::Comment(prefix) => prefix,
            Kind::Blank | Kind::Code => "",
        }
    }

    /// Returns whether a line of this kind and one of OTHER belong to one
    /// paragraph: text with the same indentation, or comments with the same
    /// prefix, the blanks after the opener aside, so that a continuation
    /// indented under a list item is the same comment.
    fn joins(&self, other: &Kind) -> bool {
        match (self, other) {
            (Kind::Text(a), Kind::Text(b)) => a == b,
            (Kind::Comment(a), Kind::Comment(b)) => a.trim_end() == b.trim_end(),
            _ => false,
        }
    }
}

/// A paragraph: its first and last lines, and what each of them is.
struct Paragraph {
    first: usize,
    last: usize,
    kinds: Vec<Kind>,
}

/// Returns the paragraph LINE of TEXT is in, or `None` when LINE is blank or
/// code.
fn paragraph_at<B: BufferTrait>(text: &B, line: usize, openers: &[String]) -> Option<Paragraph> {
    let line_text = |line: usize| {
        let (start, end) = line_bounds(text, line);
        text.slice(start, end)
    };
    let kind = Kind::of(&line_text(line), openers);
    if matches!(kind, Kind::Blank | Kind::Code) {
        return None;
    }
    let mut first = line;
    let mut before = Vec::new();
    while first > 0 {
        let above = Kind::of(&line_text(first - 1), openers);
        if !kind.joins(&above) {
            break;
        }
        before.push(above);
        first -= 1;
    }
    before.reverse();
    let mut kinds = before;
    kinds.push(kind.clone());
    let mut last = line;
    while last + 1 < text.line_count() {
        let below = Kind::of(&line_text(last + 1), openers);
        if !kind.joins(&below) {
            break;
        }
        kinds.push(below);
        last += 1;
    }
    Some(Paragraph { first, last, kinds })
}

/// Returns OLD, the paragraph's text, filled: the words after each line's
/// prefix laid out behind the first line's prefix, and the second's after it.
fn filled<F: Fn(char) -> usize>(old: &str, kinds: &[Kind], measure: &Measure<F>) -> String {
    let words: Vec<&str> = old
        .split('\n')
        .zip(kinds)
        .flat_map(|(line, kind)| line[kind.prefix().len()..].split_whitespace())
        .collect();
    let first = kinds[0].prefix();
    let rest = kinds.get(1).map_or(first, Kind::prefix);
    lay_out(&words, first, rest, measure)
}

/// Returns the prefix of each line of TEXT, a paragraph's, given the first
/// line's and the others'.
fn prefixes<'a>(text: &str, first: &'a str, rest: &'a str) -> Vec<&'a str> {
    (0..text.split('\n').count())
        .map(|index| if index == 0 { first } else { rest })
        .collect()
}

/// Returns how many characters of words, prefixes left out, TEXT has before
/// the character AT.
fn words_before(text: &str, prefixes: &[&str], at: usize) -> usize {
    let mut seen = 0;
    let mut counted = 0;
    for (line, prefix) in text.split('\n').zip(prefixes) {
        let prefix_chars = prefix.chars().count();
        for (column, c) in line.chars().enumerate() {
            if seen == at {
                return counted;
            }
            if column >= prefix_chars && !c.is_whitespace() {
                counted += 1;
            }
            seen += 1;
        }
        if seen == at {
            return counted;
        }
        seen += 1;
    }
    counted
}

/// Returns the character of TEXT after WORDS characters of words, prefixes
/// left out: where point goes back to.
fn after_words(text: &str, prefixes: &[&str], words: usize) -> usize {
    let mut seen = 0;
    let mut counted = 0;
    for (line, prefix) in text.split('\n').zip(prefixes) {
        let prefix_chars = prefix.chars().count();
        for (column, c) in line.chars().enumerate() {
            if column >= prefix_chars && !c.is_whitespace() {
                if counted == words {
                    return seen;
                }
                counted += 1;
            }
            seen += 1;
        }
        if counted == words {
            return seen;
        }
        seen += 1;
    }
    seen.saturating_sub(1)
}

/// What filling a paragraph did.
enum Refilled {
    /// Nothing: it was filled already.
    Same,
    /// Its text changed by DELTA characters, and point, when it was inside,
    /// went to POINT.
    Changed { delta: isize, point: Option<usize> },
    /// The buffer refused the change.
    Refused,
}

/// Fills PARAGRAPH of BUF, with POINT the point to carry through it.
fn refill<B: BufferTrait, F: Fn(char) -> usize>(
    buf: &mut Buffer<B>,
    paragraph: &Paragraph,
    point: usize,
    measure: &Measure<F>,
) -> Refilled {
    let start = line_bounds(&buf.text, paragraph.first).0;
    let end = line_bounds(&buf.text, paragraph.last).1;
    let old = buf.text.slice(start, end);
    let new = filled(&old, &paragraph.kinds, measure);
    if new == old {
        return Refilled::Same;
    }
    let old_prefixes: Vec<&str> = paragraph.kinds.iter().map(Kind::prefix).collect();
    let first = paragraph.kinds[0].prefix();
    let rest = paragraph.kinds.get(1).map_or(first, Kind::prefix);
    let new_prefixes = prefixes(&new, first, rest);
    let inside = (start..=end).contains(&point);
    let moved = inside.then(|| {
        let words = words_before(&old, &old_prefixes, point - start);
        // Within the first line's prefix, which filling keeps as it was.
        if words == 0 && point - start < first.chars().count() {
            point
        } else {
            start + after_words(&new, &new_prefixes, words)
        }
    });
    if !delete_range(buf, start, end) || !insert_text(buf, start, &new) {
        return Refilled::Refused;
    }
    Refilled::Changed {
        delta: new.chars().count() as isize - old.chars().count() as isize,
        point: moved,
    }
}

/// Fills the paragraphs of the current buffer from line FIRST to line LAST,
/// whole even where they begin before FIRST or end after LAST, and returns
/// how many were changed, or `None` when the buffer refused.
fn fill_lines<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    first: usize,
    last: usize,
) -> Option<usize> {
    let setup = setup(ctx, env);
    ctx.with_current_buffer_mut(|buf| {
        let mut paragraphs = Vec::new();
        let mut line = first;
        while line <= last && line < buf.text.line_count() {
            match paragraph_at(&buf.text, line, &setup.openers) {
                Some(paragraph) => {
                    line = paragraph.last + 1;
                    paragraphs.push(paragraph);
                }
                None => line += 1,
            }
        }
        // Last first, so that each paragraph's lines are still where they
        // were found.
        let mut point = buf.text.cursor_pos_1d();
        let mut changed = 0;
        for paragraph in paragraphs.iter().rev() {
            match refill(buf, paragraph, point, &setup.measure) {
                Refilled::Same => {}
                Refilled::Refused => return None,
                Refilled::Changed {
                    delta,
                    point: moved,
                } => {
                    changed += 1;
                    point = match moved {
                        Some(moved) => moved,
                        None if line_bounds(&buf.text, paragraph.first).0 < point => {
                            (point as isize + delta).max(0) as usize
                        }
                        None => point,
                    };
                }
            }
        }
        goto_offset(&mut buf.text, point);
        Some(changed)
    })
}

pub const FILL_PARAGRAPH_DOC: &str = "(fill-paragraph): Lay the words of the paragraph point is \
         in out again in lines that end at `fill-column' or before, keeping point on its word. \
         In a mode with comments, only the comment point is in is filled, behind its opener. \
         M-q.";

primitive!(fill_paragraph, _args, env, ctx, {
    let setup = setup(ctx, &env);
    let line = ctx.with_current_buffer(|buf| buf.text.cursor_pos().0);
    let found =
        ctx.with_current_buffer(|buf| paragraph_at(&buf.text, line, &setup.openers).is_some());
    if !found {
        let why = if setup.openers.is_empty() {
            "No paragraph here"
        } else {
            "Not in a comment: only comments are filled in this mode"
        };
        ctx.set_echo_message(why);
        return Ok(ELispExp::nil());
    }
    Ok(match fill_lines(ctx, &env, line, line) {
        Some(_) => ELispExp::t(),
        None => ELispExp::nil(),
    })
});

pub const FILL_REGION_DOC: &str = "(fill-region FROM TO): Fill every paragraph, and every comment \
         in a mode with comments, that has a line between FROM and TO, as `fill-paragraph' fills \
         one. Return how many were changed.";

primitive!(fill_region, args, env, ctx, {
    let (from, to) = match (args.first(), args.get(1)) {
        (Some(ELispExp::Number(from)), Some(ELispExp::Number(to))) => {
            (from.max(0.0) as usize, to.max(0.0) as usize)
        }
        _ => {
            return Err(EvalError::WrongArgumentType {
                expected: "two Numbers".into(),
                got: args.first().cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    let (first, last) = ctx.with_current_buffer(|buf| {
        let len = buf.text.len();
        let (low, high) = (from.min(to).min(len), from.max(to).min(len));
        // A region ending at the start of a line has not reached that line.
        let high = if high > low { high - 1 } else { high };
        (
            buf.text.cursor_1d_to_2d(low).0,
            buf.text.cursor_1d_to_2d(high).0,
        )
    });
    Ok(match fill_lines(ctx, &env, first, last) {
        Some(changed) => ELispExp::number(changed as f64),
        None => ELispExp::nil(),
    })
});

pub const SET_FILL_COLUMN_DOC: &str = "(set-fill-column &optional N): Fill the current buffer to \
         column N from now on, or to the column point is at when N is not a number, and return \
         it. C-x f, and C-u 72 C-x f for 72.";

primitive!(set_fill_column, args, env, ctx, {
    let column = match args.first() {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 1.0 => *n as usize,
        _ => {
            let setup = setup(ctx, &env);
            let before = ctx.with_current_buffer(|buf| {
                let point = buf.text.cursor_pos_1d();
                let (start, _) = line_bounds(&buf.text, buf.text.cursor_pos().0);
                buf.text.slice(start, point)
            });
            setup.measure.column_after(&before, 0).max(1)
        }
    };
    let was = setup(ctx, &env).measure.width;
    let key = buffer_key();
    ctx.with_current_buffer_mut(|buf| buf.put_data(&key, ELispExp::<B>::number(column as f64)));
    ctx.set_echo_message(&format!("Fill column set to {column} (was {was})"));
    Ok(ELispExp::number(column as f64))
});

pub const DO_AUTO_FILL_DOC: &str = "(do-auto-fill): Break the line point is on, if what is \
         before point goes past `fill-column', at the last blank whose word ends at the column, \
         carrying its indentation or its comment's opener to the new line. Return t when it \
         broke it. Code is never broken: only text, and comments. What `auto-fill-mode' runs \
         after a space is typed.";

primitive!(do_auto_fill, _args, env, ctx, {
    let setup = setup(ctx, &env);
    let broke = ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        let (start, end) = line_bounds(&buf.text, buf.text.cursor_pos().0);
        let line = buf.text.slice(start, end);
        let kind = Kind::of(&line, &setup.openers);
        let prefix = match &kind {
            Kind::Text(prefix) | Kind::Comment(prefix) => prefix.clone(),
            Kind::Blank | Kind::Code => return false,
        };
        let before = buf.text.slice(start, point);
        if !before.starts_with(&prefix) {
            return false;
        }
        let Some((from, to)) = break_at(&before, &prefix, &setup.measure) else {
            return false;
        };
        let from = start + before[..from].chars().count();
        let to = start + before[..to].chars().count();
        let new_line = format!("\n{prefix}");
        if !delete_range(buf, from, to) || !insert_text(buf, from, &new_line) {
            return false;
        }
        goto_offset(
            &mut buf.text,
            point - (to - from) + new_line.chars().count(),
        );
        true
    });
    Ok(if broke {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

/// Registers the primitives that fill.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("fill-paragraph", fill_paragraph, &[], FILL_PARAGRAPH_DOC);
    into.command("fill-region", fill_region, &["r"], FILL_REGION_DOC);
    into.command(
        "set-fill-column",
        set_fill_column,
        &["P"],
        SET_FILL_COLUMN_DOC,
    );
    into.function("do-auto-fill", do_auto_fill, DO_AUTO_FILL_DOC);
}
