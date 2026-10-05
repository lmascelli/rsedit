//! Commenting out, and stopping doing so.
//!
//! # Why this is not a regular expression and a prefix
//!
//! A comment is the one piece of syntax whose spelling changes with every
//! language, and the editor already knows those spellings: `set-comment-syntax`
//! declared them per mode, and the colouring, `forward-sexp` and the scanner
//! all read them from the same place. So nothing here decides what a comment
//! looks like -- it asks -- and a mode that has said nothing gets told so
//! rather than getting `#` or `//` applied hopefully.
//!
//! # The one piece of arithmetic
//!
//! Every command here makes several edits at once, and each edit moves
//! everything after it. Working forwards means every offset after the first
//! edit is stale -- the classic way a "comment the region" ends up with a
//! prefix in the middle of the third line.
//!
//! So the edits are *computed* against the buffer as it is, sorted, and then
//! applied last-first, when an edit's own offsets have not been moved by any
//! edit still to come. Point is put back afterwards from the same list, since
//! the buffer has no idea that the characters it grew belong in front of where
//! the user was.
use super::*;
use crate::buffer::{Buffer, mark::region_bounds};
use crate::modes::CommentStyle;
use crate::primitives::edits::{delete_range, goto_offset, insert_text, line_bounds};

/// One change to make, in offsets taken before any of them were made.
enum Edit {
    Insert { at: usize, text: String },
    Delete { from: usize, to: usize },
}

impl Edit {
    fn at(&self) -> usize {
        match self {
            Edit::Insert { at, .. } => *at,
            Edit::Delete { from, .. } => *from,
        }
    }
}

/// Apply EDITS, which must be in ascending order and must not overlap.
///
/// Last first, so that each edit's offsets are still the ones it was worked
/// out against. Returns false if the buffer refused -- it is read-only -- in
/// which case nothing at all was done, since the first refusal stops the rest.
fn apply<B: BufferTrait>(buf: &mut Buffer<B>, edits: &[Edit]) -> bool {
    if edits.is_empty() {
        return false;
    }
    // The precondition, checked rather than described: applying last-first is
    // only correct while each edit's offsets are untouched by the ones after
    // it, and a list built out of order would come out subtly wrong in a way
    // no single test would name.
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

fn line_text<B: BufferTrait>(text: &B, line: usize) -> String {
    let (start, end) = line_bounds(text, line);
    text.slice(start, end)
}

/// The lines a comment command works over: the region's, or point's own.
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

/// How this buffer's mode writes comments.
fn styles<B: BufferTrait>(ctx: &EditorState<B>) -> Vec<CommentStyle> {
    let mode = ctx.with_current_buffer(|buf| buf.current_mode.clone());
    ctx.modes(|modes| modes.syntax_table(&mode).comments().to_vec())
}

/// The first line-comment opener, which is what a whole-line comment uses.
fn line_opener(styles: &[CommentStyle]) -> Option<String> {
    styles.iter().find_map(|style| match style {
        CommentStyle::Line { opener } => Some(opener.clone()),
        _ => None,
    })
}

/// The first block opener and closer, for a language that has no line comment.
fn block_style(styles: &[CommentStyle]) -> Option<(String, String)> {
    styles.iter().find_map(|style| match style {
        CommentStyle::Block { opener, closer, .. } => Some((opener.clone(), closer.clone())),
        _ => None,
    })
}

fn indentation(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

fn blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// Whether LINE is already commented with OPENER.
fn commented(line: &str, opener: &str) -> bool {
    line.trim_start().starts_with(opener)
}

/// Every line of the scope is a comment already, so the command undoes them.
///
/// Blank lines do not count either way: a region with a blank line in the
/// middle of it is commented if the lines with text on them are, which is what
/// somebody looking at it would say.
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

/// The edits that comment lines FIRST..=LAST with OPENER.
///
/// The opener goes at the indentation of the least-indented line rather than
/// at each line's own, so that a commented block keeps its shape: indenting
/// the openers with the code turns a nested `if` into a staircase of `//`.
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

/// The edits that take OPENER off lines FIRST..=LAST.
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

/// Wrap the scope in a block comment, for a language with no line comment.
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

/// The scope's text, trimmed, for deciding whether a block comment is around
/// it already.
fn scope_text<B: BufferTrait>(buf: &Buffer<B>, first: usize, last: usize) -> String {
    let (start, _) = line_bounds(&buf.text, first);
    let (_, end) = line_bounds(&buf.text, last);
    buf.text.slice(start, end)
}

/// Take a block comment off the scope, when one is around it.
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

/// What every command here does: work out the scope, toggle it, report.
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

pub const COMMENT_DWIM_DOC: &str = "(comment-dwim): Comment the region out, or uncomment it if it \
         is commented already. With no region, the line point is on.\n\n\
         Uncommenting happens when *every* line with text on it is a comment; a blank line in \
         the middle of a region counts as neither, since there is nothing on it to comment.\n\n\
         The opener goes at the indentation of the least-indented line, so a commented block \
         keeps its shape rather than turning a nested `if' into a staircase.\n\n\
         How a comment is spelt comes from the mode -- see `set-comment-syntax' -- so a mode \
         that has said nothing is told about rather than guessed at. A language with only block \
         comments gets the scope wrapped in one.\n\n\
         The region is deactivated by the edit, as it is by every edit in this editor. Use \
         `comment-line' for the line alone and `comment-indent' for a comment after the code.\n\n\
         Example:\n\
         (define-key nil \"M-;\" 'comment-dwim)";

primitive!(comment_dwim, _args, _env, ctx, { Ok(toggle(ctx, false)) });

pub const COMMENT_LINE_DOC: &str = "(comment-line): Comment the line point is on, or uncomment it \
         if it is a comment already. Ignores the region, which is the difference between this \
         and `comment-dwim'.\n\n\
         Example:\n\
         (define-key nil \"C-x C-;\" 'comment-line)";

primitive!(comment_line, _args, _env, ctx, { Ok(toggle(ctx, true)) });

pub const COMMENT_REGION_DOC: &str = "(comment-region FROM TO): Comment out the lines between FROM \
         and TO, whether or not they are commented already.\n\n\
         The half of `comment-dwim' that does not decide: useful from Lisp, where the caller \
         knows which way it wants to go. `uncomment-region' is the other half.\n\n\
         Example:\n\
         (comment-region (point-min) (point-max))";

primitive!(comment_region, args, _env, ctx, {
    Ok(over_region(ctx, args, false))
});

pub const UNCOMMENT_REGION_DOC: &str = "(uncomment-region FROM TO): Take the comment opener off \
         every commented line between FROM and TO. Lines that are not comments are left alone.\n\n\
         Example:\n\
         (uncomment-region (point-min) (point-max))";

primitive!(uncomment_region, args, _env, ctx, {
    Ok(over_region(ctx, args, true))
});

/// `comment-region` and `uncomment-region`, which differ only in direction.
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
         point is on, and put point in it. Returns t.\n\n\
         For the comment that goes *beside* code rather than instead of it. When the line \
         already ends in a comment, point goes into that one rather than a second being \
         started.\n\n\
         Unbound by default -- `M-;' is `comment-dwim', which is what the key is for most of \
         the time -- and reached by name until you give it one.\n\n\
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

pub const COMMENT_SYNTAX_DOC: &str = "(comment-syntax): How this buffer's mode writes comments, in \
         the shape `set-comment-syntax' takes: a list of (OPENER), (OPENER CLOSER) or \
         (OPENER CLOSER t) for a nesting one. nil when the mode has said nothing.\n\n\
         What the comment commands ask before they touch anything, and what anything else \
         writing its own should ask too -- the alternative is a module with its own table of \
         languages, which is a table that is wrong about the next one.\n\n\
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

/// Register this module's primitives: commenting and uncommenting.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Comments, and the span deletion Lisp was missing. See
    // `primitives::comments`.
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
