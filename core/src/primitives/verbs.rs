//! The small verbs: case, whitespace, lines, transposition, and zapping.
//!
//! # What these have in common
//!
//! Each is a few characters of surgery that everybody does twenty times a day
//! and nobody wants to think about. They live here rather than in a module for
//! the reason `kill-line` does: they are what an editor *is*, and a user who
//! commented out one `.lisp` file should not find that `M-u` has stopped
//! working.
//!
//! # Where a word begins and ends
//!
//! Nowhere in this file. `word_forward` and `word_backward` are the same two
//! functions `forward-word` and `kill-word` move by, so `M-t` transposes
//! exactly what `M-f` would have stepped over. A second notion of a word,
//! written here because it was easier, is the kind of difference that is only
//! ever noticed as "sometimes it eats the underscore".
//!
//! # Point, afterwards
//!
//! Every verb says in its documentation where point ends up, because that is
//! half of what makes a verb repeatable: `M-u M-u M-u` walks up three words
//! only because each one leaves point after the word it changed. The
//! whitespace verbs leave point where the whitespace was, so that typing
//! carries on from there.
use super::*;
use crate::buffer::{Buffer, mark::region_bounds};
use crate::primitives::edits::{
    cut_out, delete_range, goto_offset, insert_text, is_word_char, line_bounds, word_backward,
    word_forward,
};
use crate::text::kill_ring::Direction;

/// Replace `[from, to)` with TEXT and put point at POINT.
///
/// Nothing at all happens when the text is already what it would become. That
/// is not an optimisation: an edit that changed no characters would still mark
/// the buffer modified, push an undo entry and wake the colouring, so
/// upcasing something already upcased would leave a file that has to be saved.
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

/// The region, or the span from point to the end of the next word.
///
/// The one rule the case verbs share: with a selection they work on it, and
/// without one they work on a word. `(usize, usize, bool)` -- the flag says
/// whether it was a region, which is what decides where point is left.
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
    /// TEXT recased. `Capitalise` works per word, so it does the same thing to
    /// a selection of six words as `M-c` six times would.
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

pub const UPCASE_WORD_DOC: &str = "(upcase-word): Upcase the word after point and leave point \
         after it, so pressing it again takes the next word. With a region, upcases the region \
         and leaves point where it was.\n\n\
         A word is what `forward-word' steps over -- letters, digits and `_' -- so what this \
         changes is what that key would have walked past.\n\n\
         Example:\n\
         (define-key nil \"M-u\" 'upcase-word)";

primitive!(upcase_word, _args, _env, ctx, {
    Ok(recase(ctx, Casing::Up))
});

pub const DOWNCASE_WORD_DOC: &str = "(downcase-word): Downcase the word after point and leave \
         point after it. With a region, downcases the region.\n\n\
         Example:\n\
         (define-key nil \"M-l\" 'downcase-word)";

primitive!(downcase_word, _args, _env, ctx, {
    Ok(recase(ctx, Casing::Down))
});

pub const CAPITALIZE_WORD_DOC: &str = "(capitalize-word): Upcase the first letter of the word \
         after point, downcase the rest, and leave point after it. With a region, does the same \
         to every word in it.\n\n\
         Example:\n\
         (define-key nil \"M-c\" 'capitalize-word)";

primitive!(capitalize_word, _args, _env, ctx, {
    Ok(recase(ctx, Casing::Capitalise))
});

// ---------------------------------------------------------------------------
// Whitespace
// ---------------------------------------------------------------------------

/// The run of spaces and tabs around POINT, as offsets.
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
         tabs around point, on both sides of it. Point ends where they were.\n\n\
         Newlines are not whitespace for this purpose: it works within one line, so it cannot \
         join two by accident. `just-one-space' is the same thing leaving a single space \
         behind.\n\n\
         Example:\n\
         (define-key nil \"M-\\\\\" 'delete-horizontal-space)";

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

pub const JUST_ONE_SPACE_DOC: &str = "(just-one-space): Leave exactly one space where the spaces \
         and tabs around point were. Point ends after it.\n\n\
         With no whitespace at point, inserts one -- which is what makes it the verb for \
         `there should be a space here' rather than only for tidying up.\n\n\
         Example:\n\
         (define-key nil \"M-<space>\" 'just-one-space)";

primitive!(just_one_space, _args, _env, ctx, {
    let changed = ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        let (from, to) = horizontal_space(&buf.text, point);
        replace_span(buf, from, to, " ", from + 1)
    });
    Ok(ELispExp::boolean(changed))
});

/// Whether LINE has nothing but whitespace on it.
fn blank_line<B: BufferTrait>(text: &B, line: usize) -> bool {
    let (from, to) = line_bounds(text, line);
    text.slice(from, to).trim().is_empty()
}

pub const DELETE_BLANK_LINES_DOC: &str = "(delete-blank-lines): On a blank line in a run of them, \
         leave one and delete the rest. On a blank line by itself, delete it. On a line with \
         text, delete the blank lines after it.\n\n\
         Three rules rather than one because it is one key pressed in three situations: in a \
         gap that is too big, in a gap that should not be there, and at the end of a paragraph \
         that has collected blank lines under it. Emacs' `C-x C-o', and its behaviour.\n\n\
         Example:\n\
         (define-key nil \"C-x C-o\" 'delete-blank-lines)";

primitive!(delete_blank_lines, _args, _env, ctx, {
    let changed = ctx.with_current_buffer_mut(|buf| {
        let line = buf.text.cursor_pos().0;
        let last = buf.text.line_count().saturating_sub(1);
        if blank_line(&buf.text, line) {
            // The whole run this line is part of.
            let mut first = line;
            while first > 0 && blank_line(&buf.text, first - 1) {
                first -= 1;
            }
            let mut end = line;
            while end < last && blank_line(&buf.text, end + 1) {
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
            while end < last && blank_line(&buf.text, end + 1) {
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
         character on the line that is not a space or a tab. Returns where it went.\n\n\
         Where `beginning-of-line' goes is column zero; where you usually want to be is the \
         start of the code. On a line with nothing but whitespace, point goes to the end of \
         it.\n\n\
         Example:\n\
         (define-key nil \"M-m\" 'back-to-indentation)";

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

pub const JOIN_LINE_DOC: &str = "(join-line): Join this line onto the end of the one above it, \
         leaving one space where the line break was. Point ends at the join.\n\n\
         The indentation of this line goes with the newline -- joining `foo' and `    bar' gives \
         `foo bar' and not `foo     bar' -- and no space is left when the line above ends in an \
         opening delimiter or this one begins with a closing one, which the mode's syntax table \
         is asked about rather than a list of brackets kept here.\n\n\
         On the first line there is nothing above to join to, and it says so.\n\n\
         Example:\n\
         (define-key nil \"M-^\" 'join-line)";

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

pub const DUPLICATE_LINE_DOC: &str = "(duplicate-line): Put a copy of this line below it. Point \
         does not move, so the copy is the one underneath.\n\n\
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

pub const TRANSPOSE_LINES_DOC: &str = "(transpose-lines): Swap this line with the one above it. \
         Point ends on the line that moved down, which is where it was.\n\n\
         On the first line there is nothing above to swap with, and it says so.\n\n\
         Example:\n\
         (define-key nil \"C-x C-t\" 'transpose-lines)";

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

pub const TRANSPOSE_CHARS_DOC: &str = "(transpose-chars): Swap the two characters around point \
         and step over them. At the end of a line, swaps the two before point, which is what \
         makes it the fix for a typo you have just finished typing.\n\n\
         Example:\n\
         (define-key nil \"C-t\" 'transpose-chars)";

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
         word after it, and leave point after the second. Whatever is between them -- a space, \
         a comma, an arrow -- stays where it is.\n\n\
         Example:\n\
         (define-key nil \"M-t\" 'transpose-words);";

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

pub const ZAP_TO_CHAR_DOC: &str = "(zap-to-char): Read a character and kill everything from point \
         up to and including the next one of it. Returns t.\n\n\
         The text goes on the kill ring, so `yank' puts it back.\n\n\
         The character is read through the same mechanism `describe-key' uses -- the editor's \
         own key resolution, diverted for one keystroke -- so this cannot be answered with \
         `C-x' or a function key, and says so rather than zapping to something surprising.\n\n\
         Example:\n\
         (define-key nil \"M-z\" 'zap-to-char)";

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
         including the character KEYS names. What `zap-to-char' arms; not meant to be called \
         by hand.\n\n\
         TARGET and SOURCE are what the key would have run and where from -- see \
         `read-key-sequence' -- and are ignored here: a key is being read as a *character*, so \
         what it is bound to is beside the point.\n\n\
         Example:\n\
         (zap-to-char--do \"q\" nil nil)";

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

/// Register this module's primitives: the transformations a key applies to a word, a line, a region.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // The small verbs. See `primitives::verbs`.
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
