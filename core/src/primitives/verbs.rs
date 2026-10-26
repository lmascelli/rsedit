//! The small verbs: case, whitespace, lines, transposition, and zapping.
//! Each says where it leaves point, and finds words with the functions the
//! motion commands move by.
//!
//! Design: `doc/architettura-editor.typ, <verbi>`.
use super::*;
use crate::buffer::{Buffer, mark::region_bounds};
use crate::primitives::edits::{delete_range, insert_text};
use crate::primitives::killing::cut_out;
use crate::primitives::movement::{
    goto_offset, is_blank_line, is_word_char, line_bounds, word_backward, word_forward,
};
use crate::text::kill_ring::Direction;

// TODO(BUF-4): a read-only buffer's refusal comes back as false, and nothing
// says so.
/// Replaces the text from FROM to TO with TEXT, puts point at POINT, and
/// returns whether the buffer changed.
///
/// When the text is TEXT already, only point moves: an edit that changed no
/// character would still mark the buffer modified, add an undo step and wake
/// the colouring.
fn replace_span<B: BufferTrait>(
    buf: &mut Buffer<B>,
    from: usize,
    to: usize,
    text: &str,
    point: usize,
) -> bool {
    if buf.text.slice(from, to) == text {
        goto_offset(&mut buf.text, point);
        return false;
    }
    if !delete_range(buf, from, to) {
        return false;
    }
    if !insert_text(buf, from, text) {
        return false;
    }
    goto_offset(&mut buf.text, point);
    true
}

/// Returns what a case verb works on: the region, or point to the end of the
/// next word; and whether it was the region, which decides where point is
/// left.
fn case_scope<B: BufferTrait>(buf: &Buffer<B>) -> (usize, usize, bool) {
    match region_bounds(buf.mark, buf.text.cursor_pos_1d(), buf.text.len()) {
        Some((from, to)) => (from, to, true),
        None => {
            let from = buf.text.cursor_pos_1d();
            (from, word_forward(&buf.text, from, 1), false)
        }
    }
}

/// How the case verbs change the letters they are given.
#[derive(Clone, Copy)]
enum Casing {
    Up,
    Down,
    Capitalise,
}

impl Casing {
    /// Returns TEXT recased. `Capitalise` works per word, so a selection of
    /// six words comes out as six `M-c` would leave it.
    fn apply(self, text: &str) -> String {
        match self {
            Casing::Up => text.to_uppercase(),
            Casing::Down => text.to_lowercase(),
            Casing::Capitalise => {
                let mut out = String::with_capacity(text.len());
                let mut starting = true;
                for c in text.chars() {
                    if is_word_char(c) {
                        if starting {
                            out.extend(c.to_uppercase());
                        } else {
                            out.extend(c.to_lowercase());
                        }
                        starting = false;
                    } else {
                        out.push(c);
                        starting = true;
                    }
                }
                out
            }
        }
    }
}

/// Recases what a case verb works on with CASING, and answers whether the
/// buffer changed.
fn recase<B: BufferTrait>(ctx: &EditorState<B>, casing: Casing) -> ELispExp<B> {
    let changed = ctx.with_current_buffer_mut(|buf| {
        let (from, to, was_region) = case_scope(buf);
        if from >= to {
            return false;
        }
        let text = casing.apply(&buf.text.slice(from, to));
        // A region keeps point where it was; a word leaves point after
        // itself, which is what makes the key repeatable.
        let point = if was_region {
            buf.text.cursor_pos_1d()
        } else {
            to
        };
        replace_span(buf, from, to, &text, point)
    });
    ELispExp::boolean(changed)
}

pub const UPCASE_WORD_DOC: &str = "(upcase-word): Upcase the word after point, and leave point \
         after it. With a region, upcase the region and leave point where it is.";

primitive!(upcase_word, _args, _env, ctx, {
    Ok(recase(ctx, Casing::Up))
});

pub const DOWNCASE_WORD_DOC: &str = "(downcase-word): Downcase the word after point, and leave \
         point after it. With a region, downcase the region and leave point where it is.";

primitive!(downcase_word, _args, _env, ctx, {
    Ok(recase(ctx, Casing::Down))
});

pub const CAPITALIZE_WORD_DOC: &str = "(capitalize-word): Capitalize the word after point, \
         upcasing its first letter and downcasing the rest, and leave point after it. With a \
         region, capitalize every word in it.";

primitive!(capitalize_word, _args, _env, ctx, {
    Ok(recase(ctx, Casing::Capitalise))
});

// ---------------------------------------------------------------------------
// Whitespace
// ---------------------------------------------------------------------------

/// Returns the offsets of the run of spaces and tabs around POINT.
fn horizontal_space<B: BufferTrait>(text: &B, point: usize) -> (usize, usize) {
    let horizontal = |c: char| c == ' ' || c == '\t';
    let mut from = point;
    while from > 0 && text.at(from - 1).is_some_and(horizontal) {
        from -= 1;
    }
    let mut to = point;
    while to < text.len() && text.at(to).is_some_and(horizontal) {
        to += 1;
    }
    (from, to)
}

pub const DELETE_HORIZONTAL_SPACE_DOC: &str = "(delete-horizontal-space): Delete the spaces and \
         tabs on both sides of point, within the line.";

primitive!(delete_horizontal_space, _args, _env, ctx, {
    let changed = ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        let (from, to) = horizontal_space(&buf.text, point);
        if from >= to {
            return false;
        }
        let done = delete_range(buf, from, to);
        goto_offset(&mut buf.text, from);
        done
    });
    Ok(ELispExp::boolean(changed))
});

pub const JUST_ONE_SPACE_DOC: &str = "(just-one-space): Replace the spaces and tabs around point \
         with one space, or insert one when there are none, and leave point after it.";

primitive!(just_one_space, _args, _env, ctx, {
    let changed = ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        let (from, to) = horizontal_space(&buf.text, point);
        replace_span(buf, from, to, " ", from + 1)
    });
    Ok(ELispExp::boolean(changed))
});

pub const DELETE_BLANK_LINES_DOC: &str = "(delete-blank-lines): On a blank line among others, \
         delete all of them but one; on a blank line alone, delete it; on a line with text, \
         delete the blank lines after it.";

primitive!(delete_blank_lines, _args, _env, ctx, {
    let changed = ctx.with_current_buffer_mut(|buf| {
        let line = buf.text.cursor_pos().0;
        let last = buf.text.line_count().saturating_sub(1);
        if is_blank_line(&buf.text, line) {
            // The whole run this line is part of.
            let mut first = line;
            while first > 0 && is_blank_line(&buf.text, first - 1) {
                first -= 1;
            }
            let mut end = line;
            while end < last && is_blank_line(&buf.text, end + 1) {
                end += 1;
            }
            let alone = first == end;
            let (keep_from, _) = line_bounds(&buf.text, first);
            let (_, run_end) = line_bounds(&buf.text, end);
            // One blank line survives a run; a single one goes altogether.
            // The newline goes with it, or deleting the only line of a buffer
            // would leave an empty line behind and look like nothing happened.
            let (from, to) = if alone {
                (keep_from, (run_end + 1).min(buf.text.len()))
            } else {
                let (_, first_end) = line_bounds(&buf.text, first);
                (
                    (first_end + 1).min(buf.text.len()),
                    (run_end + 1).min(buf.text.len()),
                )
            };
            if from >= to {
                return false;
            }
            let done = delete_range(buf, from, to);
            let landing = keep_from.min(buf.text.len());
            goto_offset(&mut buf.text, landing);
            done
        } else {
            // On a line with text: the blank ones under it go.
            let mut end = line;
            while end < last && is_blank_line(&buf.text, end + 1) {
                end += 1;
            }
            if end == line {
                return false;
            }
            let point = buf.text.cursor_pos_1d();
            let (_, from) = line_bounds(&buf.text, line);
            let (_, to) = line_bounds(&buf.text, end);
            let done = delete_range(buf, from, to.min(buf.text.len()));
            goto_offset(&mut buf.text, point);
            done
        }
    });
    Ok(ELispExp::boolean(changed))
});

pub const BACK_TO_INDENTATION_DOC: &str = "(back-to-indentation): Move point to the first \
         character of its line that is not a space or a tab, or to the end of a blank line, and \
         return the new position.";

primitive!(back_to_indentation, _args, _env, ctx, {
    let at = ctx.with_current_buffer_mut(|buf| {
        let line = buf.text.cursor_pos().0;
        let (from, to) = line_bounds(&buf.text, line);
        let mut at = from;
        while at < to && buf.text.at(at).is_some_and(|c| c == ' ' || c == '\t') {
            at += 1;
        }
        goto_offset(&mut buf.text, at);
        at
    });
    Ok(ELispExp::number(at as f64))
});

// ---------------------------------------------------------------------------
// Lines
// ---------------------------------------------------------------------------

pub const JOIN_LINE_DOC: &str = "(join-line): Join this line to the end of the one above, putting \
         one space where the line break and this line's indentation were, or none after an \
         opening delimiter or before a closing one. On the first line there is nothing to join \
         to, and it says so.";

primitive!(join_line, _args, _env, ctx, {
    let mode = ctx.with_current_buffer(|buf| buf.current_mode.clone());
    let syntax = ctx.modes(|modes| modes.syntax_table(&mode));
    let changed = ctx.with_current_buffer_mut(|buf| {
        let line = buf.text.cursor_pos().0;
        if line == 0 {
            return None;
        }
        let (_, above_end) = line_bounds(&buf.text, line - 1);
        let (start, end) = line_bounds(&buf.text, line);
        // Back over the previous line's trailing whitespace, forward over this
        // line's indentation: both belong to the break being removed.
        let mut from = above_end;
        while from > 0 && buf.text.at(from - 1).is_some_and(|c| c == ' ' || c == '\t') {
            from -= 1;
        }
        let mut to = start;
        while to < end && buf.text.at(to).is_some_and(|c| c == ' ' || c == '\t') {
            to += 1;
        }
        let before = (from > 0).then(|| buf.text.at(from - 1)).flatten();
        let after = buf.text.at(to);
        let opens = before
            .is_some_and(|c| matches!(syntax.class_of(c), crate::modes::SyntaxClass::Open(_)));
        let closes = after
            .is_some_and(|c| matches!(syntax.class_of(c), crate::modes::SyntaxClass::Close(_)));
        let joined = if before.is_none() || after.is_none() || opens || closes {
            ""
        } else {
            " "
        };
        let point = from + joined.chars().count();
        Some(replace_span(buf, from, to, joined, point))
    });
    match changed {
        Some(changed) => Ok(ELispExp::boolean(changed)),
        None => {
            ctx.set_echo_message("No line above this one to join to");
            Ok(ELispExp::nil())
        }
    }
});

pub const DUPLICATE_LINE_DOC: &str = "(duplicate-line): Put a copy of this line below it, leaving \
         point where it is.\n\n\
         Example:\n\
         (define-key nil \"C-c d\" 'duplicate-line)";

primitive!(duplicate_line, _args, _env, ctx, {
    let changed = ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        let line = buf.text.cursor_pos().0;
        let (start, end) = line_bounds(&buf.text, line);
        let text = buf.text.slice(start, end);
        // Inserted *after* this line's newline where there is one, so the copy
        // is a line rather than the rest of this one.
        let at = (end + 1).min(buf.text.len());
        let copy = if end < buf.text.len() {
            format!("{text}\n")
        } else {
            format!("\n{text}")
        };
        let done = insert_text(buf, at, &copy);
        goto_offset(&mut buf.text, point);
        done
    });
    Ok(ELispExp::boolean(changed))
});

pub const TRANSPOSE_LINES_DOC: &str = "(transpose-lines): Move this line above the one before it, \
         and point with it. On the first line there is nothing to swap with, and it says so.";

primitive!(transpose_lines, _args, _env, ctx, {
    let answer = ctx.with_current_buffer_mut(|buf| {
        let line = buf.text.cursor_pos().0;
        if line == 0 {
            return None;
        }
        let column = buf.text.cursor_pos().1;
        let (above_start, above_end) = line_bounds(&buf.text, line - 1);
        let (start, end) = line_bounds(&buf.text, line);
        let above = buf.text.slice(above_start, above_end);
        let here = buf.text.slice(start, end);
        let swapped = format!("{here}\n{above}");
        // Point follows the text it was in: it was on the lower line, which is
        // now the upper one, at the same column.
        let point = (above_start + column).min(above_start + here.chars().count());
        Some(replace_span(buf, above_start, end, &swapped, point))
    });
    match answer {
        Some(changed) => Ok(ELispExp::boolean(changed)),
        None => {
            ctx.set_echo_message("No line above this one to transpose with");
            Ok(ELispExp::nil())
        }
    }
});

// ---------------------------------------------------------------------------
// Transposition
// ---------------------------------------------------------------------------

pub const TRANSPOSE_CHARS_DOC: &str = "(transpose-chars): Swap the characters before and after \
         point, and leave point after both. At the end of a line, swap the two before point.";

primitive!(transpose_chars, _args, _env, ctx, {
    let changed = ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        let line = buf.text.cursor_pos().0;
        let (start, end) = line_bounds(&buf.text, line);
        // At the end of a line -- or of the buffer -- the two characters worth
        // swapping are behind point. Anywhere else they are either side of it.
        let at_end = point >= end;
        let first = if at_end {
            if point < start + 2 {
                return false;
            }
            point - 2
        } else {
            if point == start {
                return false;
            }
            point - 1
        };
        let pair = buf.text.slice(first, first + 2);
        let mut chars = pair.chars();
        let (Some(a), Some(b)) = (chars.next(), chars.next()) else {
            return false;
        };
        let swapped = format!("{b}{a}");
        let point = if at_end { point } else { point + 1 };
        replace_span(buf, first, first + 2, &swapped, point)
    });
    Ok(ELispExp::boolean(changed))
});

pub const TRANSPOSE_WORDS_DOC: &str = "(transpose-words): Swap the word before point with the \
         word after it, leaving what is between them in place, and leave point after both.";

primitive!(transpose_words, _args, _env, ctx, {
    let changed = ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        // The word behind: its end is where the letters stop going back, its
        // start is one `backward-word` from there.
        let mut first_end = point;
        while first_end > 0 && !buf.text.at(first_end - 1).is_some_and(is_word_char) {
            first_end -= 1;
        }
        if first_end == 0 {
            return false;
        }
        let first_start = word_backward(&buf.text, first_end, 1);
        let second_end = word_forward(&buf.text, point, 1);
        let mut second_start = second_end;
        while second_start > 0 && buf.text.at(second_start - 1).is_some_and(is_word_char) {
            second_start -= 1;
        }
        if second_start < first_end || second_start >= second_end {
            return false;
        }
        let first = buf.text.slice(first_start, first_end);
        let middle = buf.text.slice(first_end, second_start);
        let second = buf.text.slice(second_start, second_end);
        let swapped = format!("{second}{middle}{first}");
        replace_span(buf, first_start, second_end, &swapped, second_end)
    });
    Ok(ELispExp::boolean(changed))
});

// ---------------------------------------------------------------------------
// Zapping
// ---------------------------------------------------------------------------

/// The name the capture calls back into. See `zap-to-char`.
const ZAP_ANSWER: &str = "zap-to-char--do";

pub const ZAP_TO_CHAR_DOC: &str = "(zap-to-char): Read a character, and kill from point up to and \
         including the next one of it. Return t. A key that is not a character, such as C-x, is \
         refused with a message.";

primitive!(zap_to_char, _args, env, ctx, {
    ctx.set_echo_message("Zap to char:");
    // Armed with the *name*: the capture quotes what it is given before
    // calling it, so a symbol is looked up in the function namespace, exactly
    // as `help.lisp` arms its own.
    env.set_root_variable(
        crate::editor::KEY_CAPTURE_FUNCTION.into(),
        ELispExp::symbol(ZAP_ANSWER.into()),
    );
    Ok(ELispExp::t())
});

pub const ZAP_TO_CHAR_DO_DOC: &str = "(zap-to-char--do KEYS TARGET SOURCE): Kill up to and \
         including the character KEYS names: what `zap-to-char' runs when the key arrives. TARGET \
         and SOURCE are ignored.";

primitive!(zap_to_char_do, args, env, ctx, {
    let keys = match args.first() {
        Some(ELispExp::String(keys)) => keys.to_string(),
        _ => String::new(),
    };
    let mut chars = keys.chars();
    let (Some(target), None) = (chars.next(), chars.next()) else {
        ctx.set_echo_message(&format!("{keys} is not a character"));
        return Ok(ELispExp::nil());
    };
    let killed = ctx.with_current_buffer_mut(|buf| {
        let from = buf.text.cursor_pos_1d();
        let mut to = from;
        while to < buf.text.len() && buf.text.at(to) != Some(target) {
            to += 1;
        }
        if to >= buf.text.len() {
            return None;
        }
        // Up to *and including* it, which is what makes `M-z ;` end a
        // statement rather than leave the semicolon behind.
        Some(cut_out(buf, from, to + 1))
    });
    match killed {
        Some(killed) if !killed.is_empty() => {
            ctx.kill(killed, Direction::Forward, &env);
            Ok(ELispExp::t())
        }
        Some(_) => Ok(ELispExp::nil()),
        None => {
            ctx.set_echo_message(&format!("No {target} after point"));
            Ok(ELispExp::nil())
        }
    }
});

/// Registers the small verbs.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("upcase-word", upcase_word, &[], UPCASE_WORD_DOC);
    into.command("downcase-word", downcase_word, &[], DOWNCASE_WORD_DOC);
    into.command("capitalize-word", capitalize_word, &[], CAPITALIZE_WORD_DOC);
    into.command(
        "delete-horizontal-space",
        delete_horizontal_space,
        &[],
        DELETE_HORIZONTAL_SPACE_DOC,
    );
    into.command("just-one-space", just_one_space, &[], JUST_ONE_SPACE_DOC);
    into.command(
        "delete-blank-lines",
        delete_blank_lines,
        &[],
        DELETE_BLANK_LINES_DOC,
    );
    into.command(
        "back-to-indentation",
        back_to_indentation,
        &[],
        BACK_TO_INDENTATION_DOC,
    );
    into.command("join-line", join_line, &[], JOIN_LINE_DOC);
    into.command("duplicate-line", duplicate_line, &[], DUPLICATE_LINE_DOC);
    into.command("transpose-lines", transpose_lines, &[], TRANSPOSE_LINES_DOC);
    into.command("transpose-chars", transpose_chars, &[], TRANSPOSE_CHARS_DOC);
    into.command("transpose-words", transpose_words, &[], TRANSPOSE_WORDS_DOC);
    into.command("zap-to-char", zap_to_char, &[], ZAP_TO_CHAR_DOC);
    into.function("zap-to-char--do", zap_to_char_do, ZAP_TO_CHAR_DO_DOC);
}
