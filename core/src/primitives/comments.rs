//! Commenting lines out and back in, with the spelling the buffer's mode
//! declared (`set-comment-syntax`). Every command works its edits out against
//! the text as it is, then applies them last first.
//!
//! Design: `doc/architettura-editor.typ, <modifiche-multiple>`.
use super::*;
use crate::buffer::{Buffer, mark::region_bounds};
use crate::modes::CommentStyle;
use crate::primitives::edits::{delete_range, insert_text};
use crate::primitives::movement::{goto_offset, line_bounds};

/// One change to make, in offsets taken before any of them were made.
enum Edit {
    Insert { at: usize, text: String },
    Delete { from: usize, to: usize },
}

impl Edit {
    /// Returns where the change starts.
    fn at(&self) -> usize {
        match self {
            Edit::Insert { at, .. } => *at,
            Edit::Delete { from, .. } => *from,
        }
    }
}

// TODO(BUF-4): the refusal is not said: the commands answer nil, and the
// echo area stays empty.
/// Applies EDITS, which must be in ascending order and must not overlap, last
/// first, so that each edit's offsets are still those it was worked out
/// against; then puts point back where the user was, in the text as it is.
///
/// Returns false when there was nothing to do, or when the buffer refused:
/// then nothing was done, since a read-only buffer refuses the first edit.
fn apply<B: BufferTrait>(buf: &mut Buffer<B>, edits: &[Edit]) -> bool {
    if edits.is_empty() {
        return false;
    }
    // The order is the precondition of applying last first, checked rather
    // than described: a list out of order comes out subtly wrong.
    debug_assert!(
        edits.windows(2).all(|pair| pair[0].at() <= pair[1].at()),
        "comment edits must be in ascending order"
    );
    let point = buf.text.cursor_pos_1d();
    for edit in edits.iter().rev() {
        let done = match edit {
            Edit::Insert { at, text } => insert_text(buf, *at, text),
            Edit::Delete { from, to } => delete_range(buf, *from, *to),
        };
        if !done {
            return false;
        }
    }
    // Where the user was, in the text as it is now. An edit before point moves
    // it; an edit that swallowed point leaves it where the deletion began.
    let mut moved = point as isize;
    for edit in edits {
        match edit {
            Edit::Insert { at, text } if *at <= point => {
                moved += text.chars().count() as isize;
            }
            Edit::Delete { from, to } if *to <= point => {
                moved -= (*to - *from) as isize;
            }
            Edit::Delete { from, to } if *from < point && point < *to => {
                moved -= (point - *from) as isize;
            }
            _ => {}
        }
    }
    goto_offset(&mut buf.text, moved.max(0) as usize);
    true
}

/// Returns the text of LINE, without its newline.
fn line_text<B: BufferTrait>(text: &B, line: usize) -> String {
    let (start, end) = line_bounds(text, line);
    text.slice(start, end)
}

/// Returns the lines a comment command works over: the region's, or point's
/// own.
fn scope<B: BufferTrait>(buf: &Buffer<B>) -> (usize, usize) {
    match region_bounds(buf.mark, buf.text.cursor_pos_1d(), buf.text.len()) {
        Some((from, to)) => {
            let first = buf.text.cursor_1d_to_2d(from).0;
            // A region ending at the very start of a line has not reached that
            // line: selecting three lines by dragging down the left margin
            // would otherwise comment a fourth.
            let last_offset = if to > from { to - 1 } else { to };
            (first, buf.text.cursor_1d_to_2d(last_offset).0)
        }
        None => {
            let line = buf.text.cursor_pos().0;
            (line, line)
        }
    }
}

/// Returns how the current buffer's mode writes comments.
fn styles<B: BufferTrait>(ctx: &EditorState<B>) -> Vec<CommentStyle> {
    let mode = ctx.with_current_buffer(|buf| buf.current_mode.clone());
    ctx.modes(|modes| modes.syntax_table(&mode).comments().to_vec())
}

/// Returns the first line-comment opener, the one a whole-line comment uses.
fn line_opener(styles: &[CommentStyle]) -> Option<String> {
    styles.iter().find_map(|style| match style {
        CommentStyle::Line { opener } => Some(opener.clone()),
        _ => None,
    })
}

/// Returns the first block opener and closer, for a language with no line
/// comment.
fn block_style(styles: &[CommentStyle]) -> Option<(String, String)> {
    styles.iter().find_map(|style| match style {
        CommentStyle::Block { opener, closer, .. } => Some((opener.clone(), closer.clone())),
        _ => None,
    })
}

/// Returns how many whitespace characters begin LINE.
fn indentation(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// Returns whether LINE is nothing but whitespace.
fn blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// Returns whether LINE is already commented with OPENER.
fn commented(line: &str, opener: &str) -> bool {
    line.trim_start().starts_with(opener)
}

/// Returns whether every line of FIRST..=LAST with text on it is commented
/// with OPENER already, so that the command takes the comments off; false
/// when no line has text.
///
/// Blank lines count neither way: a region is commented if its lines with
/// text are, which is what somebody looking at it would say.
fn all_commented<B: BufferTrait>(buf: &Buffer<B>, first: usize, last: usize, opener: &str) -> bool {
    let mut seen = false;
    for line in first..=last {
        let text = line_text(&buf.text, line);
        if blank(&text) {
            continue;
        }
        seen = true;
        if !commented(&text, opener) {
            return false;
        }
    }
    seen
}

/// Returns the edits that comment lines FIRST..=LAST with OPENER.
///
/// The opener goes at the indentation of the least indented line, not at each
/// line's own, so that a commented block keeps its shape: openers indented
/// with the code turn a nested `if` into a staircase of `//`.
fn comment_edits<B: BufferTrait>(
    buf: &Buffer<B>,
    first: usize,
    last: usize,
    opener: &str,
) -> Vec<Edit> {
    let single = first == last;
    let column = (first..=last)
        .map(|line| line_text(&buf.text, line))
        .filter(|text| single || !blank(text))
        .map(|text| indentation(&text))
        .min()
        .unwrap_or(0);
    let mut edits = Vec::new();
    for line in first..=last {
        let text = line_text(&buf.text, line);
        // A blank line inside a region is left alone -- there is nothing on it
        // to comment out, and a prefix would leave trailing whitespace behind.
        // A blank line *is* commented when it is the whole scope, because then
        // it is what was asked for.
        if blank(&text) && !single {
            continue;
        }
        let (start, end) = line_bounds(&buf.text, line);
        let at = (start + column).min(end.max(start));
        edits.push(Edit::Insert {
            at,
            text: format!("{opener} "),
        });
    }
    edits
}

/// Returns the edits that take OPENER, and the space after it, off lines
/// FIRST..=LAST.
fn uncomment_edits<B: BufferTrait>(
    buf: &Buffer<B>,
    first: usize,
    last: usize,
    opener: &str,
) -> Vec<Edit> {
    let mut edits = Vec::new();
    for line in first..=last {
        let text = line_text(&buf.text, line);
        if !commented(&text, opener) {
            continue;
        }
        let (start, _) = line_bounds(&buf.text, line);
        let at = start + indentation(&text);
        let mut width = opener.chars().count();
        // The space this command put in when it commented, and only that one:
        // `//   indented` keeps the indentation that was inside the comment.
        if text.trim_start().chars().nth(width) == Some(' ') {
            width += 1;
        }
        edits.push(Edit::Delete {
            from: at,
            to: at + width,
        });
    }
    edits
}

/// Returns the edits that wrap lines FIRST..=LAST in a block comment, for a
/// language with no line comment.
fn wrap_edits<B: BufferTrait>(
    buf: &Buffer<B>,
    first: usize,
    last: usize,
    opener: &str,
    closer: &str,
) -> Vec<Edit> {
    let first_text = line_text(&buf.text, first);
    let last_text = line_text(&buf.text, last);
    let (first_start, _) = line_bounds(&buf.text, first);
    let (last_start, last_end) = line_bounds(&buf.text, last);
    let open_at = first_start + indentation(&first_text);
    let close_at = last_end.max(last_start + last_text.trim_end().chars().count());
    vec![
        Edit::Insert {
            at: open_at,
            text: format!("{opener} "),
        },
        Edit::Insert {
            at: close_at,
            text: format!(" {closer}"),
        },
    ]
}

/// Returns the text of lines FIRST..=LAST, for deciding whether a block
/// comment is around it already.
fn scope_text<B: BufferTrait>(buf: &Buffer<B>, first: usize, last: usize) -> String {
    let (start, _) = line_bounds(&buf.text, first);
    let (_, end) = line_bounds(&buf.text, last);
    buf.text.slice(start, end)
}

/// Returns the edits that take a block comment off lines FIRST..=LAST, or
/// `None` when no block comment is around them.
fn unwrap_edits<B: BufferTrait>(
    buf: &Buffer<B>,
    first: usize,
    last: usize,
    opener: &str,
    closer: &str,
) -> Option<Vec<Edit>> {
    let text = scope_text(buf, first, last);
    let trimmed = text.trim();
    if !trimmed.starts_with(opener) || !trimmed.ends_with(closer) || trimmed.len() < opener.len() {
        return None;
    }
    let (start, _) = line_bounds(&buf.text, first);
    let (_, end) = line_bounds(&buf.text, last);
    let lead = text.chars().take_while(|c| c.is_whitespace()).count();
    let trail = text.chars().rev().take_while(|c| c.is_whitespace()).count();
    let open_at = start + lead;
    let mut open_width = opener.chars().count();
    if text.chars().nth(lead + open_width) == Some(' ') {
        open_width += 1;
    }
    let close_end = end - trail;
    let mut close_start = close_end - closer.chars().count();
    if close_start > open_at + open_width
        && text
            .chars()
            .nth(close_start - start - 1)
            .is_some_and(|c| c == ' ')
    {
        close_start -= 1;
    }
    Some(vec![
        Edit::Delete {
            from: open_at,
            to: open_at + open_width,
        },
        Edit::Delete {
            from: close_start,
            to: close_end,
        },
    ])
}

/// Comments the scope out, or takes its comments off when it is all commented
/// already, and answers whether the buffer changed. The scope is the region
/// or point's line; with WHOLE_LINE, point's line whatever the region.
fn toggle<B: BufferTrait>(ctx: &EditorState<B>, whole_line: bool) -> ELispExp<B> {
    let styles = styles(ctx);
    if styles.is_empty() {
        ctx.set_echo_message("This mode has not said how it writes comments");
        return ELispExp::nil();
    }
    let changed = ctx.with_current_buffer_mut(|buf| {
        let (first, last) = if whole_line {
            let line = buf.text.cursor_pos().0;
            (line, line)
        } else {
            scope(buf)
        };
        let edits = match line_opener(&styles) {
            Some(opener) => {
                if all_commented(buf, first, last, &opener) {
                    uncomment_edits(buf, first, last, &opener)
                } else {
                    comment_edits(buf, first, last, &opener)
                }
            }
            None => {
                let (opener, closer) = block_style(&styles).expect("styles are not empty");
                match unwrap_edits(buf, first, last, &opener, &closer) {
                    Some(edits) => edits,
                    None => wrap_edits(buf, first, last, &opener, &closer),
                }
            }
        };
        apply(buf, &edits)
    });
    ELispExp::boolean(changed)
}

pub const COMMENT_DWIM_DOC: &str = "(comment-dwim): Comment out the region, or the line when \
         there is no region; take the comments off instead when every line with text in it is \
         commented already. The comment opener goes at the indentation of the least indented \
         line. A mode with only block comments gets the lines wrapped in one.";

primitive!(comment_dwim, _args, _env, ctx, { Ok(toggle(ctx, false)) });

pub const COMMENT_LINE_DOC: &str = "(comment-line): Comment out the line point is on, or \
         uncomment it when it is a comment, whatever the region.";

primitive!(comment_line, _args, _env, ctx, { Ok(toggle(ctx, true)) });

pub const COMMENT_REGION_DOC: &str = "(comment-region FROM TO): Comment out the lines between \
         FROM and TO, whether or not they are comments already. It needs a mode with line \
         comments.\n\n\
         Example:\n\
         (comment-region (point-min) (point-max))";

primitive!(comment_region, args, _env, ctx, {
    Ok(over_region(ctx, args, false))
});

pub const UNCOMMENT_REGION_DOC: &str = "(uncomment-region FROM TO): Take the comment opener off \
         each commented line between FROM and TO, leaving the others alone. It needs a mode with \
         line comments.";

primitive!(uncomment_region, args, _env, ctx, {
    Ok(over_region(ctx, args, true))
});

/// Comments out the lines between the two positions in ARGS, or with
/// UNDO_THEM takes their comments off, and answers whether the buffer
/// changed: `comment-region` and `uncomment-region`.
fn over_region<B: BufferTrait>(
    ctx: &EditorState<B>,
    args: &[ELispExp<B>],
    undo_them: bool,
) -> ELispExp<B> {
    let (from, to) = match (args.first(), args.get(1)) {
        (Some(ELispExp::Number(from)), Some(ELispExp::Number(to))) => {
            (*from as usize, *to as usize)
        }
        _ => {
            ctx.set_echo_message("comment-region needs two positions");
            return ELispExp::nil();
        }
    };
    let styles = styles(ctx);
    let Some(opener) = line_opener(&styles) else {
        ctx.set_echo_message("This mode has no line comment");
        return ELispExp::nil();
    };
    let changed = ctx.with_current_buffer_mut(|buf| {
        let (from, to) = (from.min(to), from.max(to).min(buf.text.len()));
        let first = buf.text.cursor_1d_to_2d(from).0;
        let last = buf
            .text
            .cursor_1d_to_2d(if to > from { to - 1 } else { to })
            .0;
        let edits = if undo_them {
            uncomment_edits(buf, first, last, &opener)
        } else {
            comment_edits(buf, first, last, &opener)
        };
        apply(buf, &edits)
    });
    ELispExp::boolean(changed)
}

pub const COMMENT_INDENT_DOC: &str = "(comment-indent): Start a comment at the end of the line \
         point is on and put point in it, or go into the comment already there. Return t. It \
         needs a mode with line comments.\n\n\
         Example:\n\
         (define-key nil \"C-c ;\" 'comment-indent)";

primitive!(comment_indent, _args, _env, ctx, {
    let styles = styles(ctx);
    let Some(opener) = line_opener(&styles) else {
        ctx.set_echo_message("This mode has no line comment");
        return Ok(ELispExp::nil());
    };
    let done = ctx.with_current_buffer_mut(|buf| {
        let line = buf.text.cursor_pos().0;
        let text = line_text(&buf.text, line);
        let (start, end) = line_bounds(&buf.text, line);
        // Already one there: go to it rather than start a second.
        if let Some(column) = text.find(&opener) {
            let at = start + text[..column].chars().count() + opener.chars().count();
            let at = if buf.text.at(at) == Some(' ') {
                at + 1
            } else {
                at
            };
            goto_offset(&mut buf.text, at);
            return true;
        }
        let trimmed = text.trim_end().chars().count();
        let at = start + trimmed;
        let separator = if trimmed == 0 { "" } else { " " };
        let inserted = format!("{separator}{opener} ");
        let length = inserted.chars().count();
        // The trailing whitespace goes as well: a comment started after it
        // would sit in the middle of the line for no reason anybody chose.
        let edits = if end > at {
            vec![
                Edit::Delete { from: at, to: end },
                Edit::Insert {
                    at: end,
                    text: inserted,
                },
            ]
        } else {
            vec![Edit::Insert { at, text: inserted }]
        };
        if !apply(buf, &edits) {
            return false;
        }
        goto_offset(&mut buf.text, at + length);
        true
    });
    Ok(ELispExp::boolean(done))
});

pub const COMMENT_SYNTAX_DOC: &str = "(comment-syntax): Return how this buffer's mode writes \
         comments, the way `set-comment-syntax' takes it: a list of (OPENER), (OPENER CLOSER), or \
         (OPENER CLOSER t) for one that nests; nil when the mode has said nothing.\n\n\
         Example:\n\
         (comment-syntax) => ((\"//\") (\"/*\" \"*/\" t))";

primitive!(comment_syntax, _args, _env, ctx, {
    let found: Vec<ELispExp<B>> = styles(ctx)
        .into_iter()
        .map(|style| match style {
            CommentStyle::Line { opener } => ELispExp::proper_list(vec![ELispExp::string(opener)]),
            CommentStyle::Block {
                opener,
                closer,
                nestable,
            } => {
                let mut parts = vec![ELispExp::string(opener), ELispExp::string(closer)];
                if nestable {
                    parts.push(ELispExp::t());
                }
                ELispExp::proper_list(parts)
            }
        })
        .collect();
    Ok(ELispExp::proper_list(found))
});

/// Registers the primitives that comment and uncomment.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("comment-dwim", comment_dwim, &[], COMMENT_DWIM_DOC);
    into.command("comment-line", comment_line, &[], COMMENT_LINE_DOC);
    into.command("comment-indent", comment_indent, &[], COMMENT_INDENT_DOC);
    into.command("comment-region", comment_region, &["r"], COMMENT_REGION_DOC);
    into.command(
        "uncomment-region",
        uncomment_region,
        &["r"],
        UNCOMMENT_REGION_DOC,
    );
    into.function("comment-syntax", comment_syntax, COMMENT_SYNTAX_DOC);
}
