use super::*;
use crate::buffer::{Buffer, Mark, undo};
use crate::kill_ring::Direction;

// ---------------------------------------------------------------------------
// The recording editing layer
// ---------------------------------------------------------------------------
//
// Every primitive that changes buffer text goes through one of the two
// functions below. That is the whole reason undo works: history is not
// something each command has to remember to write, it is a property of the
// only two doors into the text. A new editing command gets undo by being
// unable to avoid it.
//
// They take the whole `Buffer` rather than its text, because recording needs
// the history that sits beside the text -- and taking both together is what
// makes it impossible to hold one without the other.

/// Delete `[from, to)` from BUF, recording it so it can be undone.
///
/// `delete()` removes the character *before* point, so the application step
/// positions at the far end and deletes backwards. Each of those is O(1) once
/// the gap is there, so the whole range costs one gap move rather than one per
/// character.
pub(crate) fn delete_range<B: BufferTrait>(buf: &mut Buffer<B>, from: usize, to: usize) {
    let start = from.min(to);
    let end = from.max(to).min(buf.text.len());
    if start >= end {
        return;
    }
    let point = buf.text.cursor_pos_1d();
    let whole_buffer = start == 0 && end == buf.text.len();
    // Captured before the deletion, since afterwards there is nothing left to
    // read. Clearing the whole buffer is the one case worth special casing:
    // `to_string` reads the text in one pass, where the general path pays a
    // lookup per character.
    let removed: String = if whole_buffer {
        buf.text.to_string()
    } else {
        (start..end).filter_map(|i| buf.text.at(i)).collect()
    };
    buf.undo.record_delete(start, removed, point);
    buf.mark = deactivated(buf.mark);
    if whole_buffer {
        // `clear` exists precisely so that emptying a large buffer is not
        // 60,000 gap-moving deletions -- see `BufferTrait::clear`.
        buf.text.clear();
    } else {
        undo::apply_delete(&mut buf.text, start, end);
    }
    buf.is_modified = true;
}

/// Insert CONTENT at offset AT in BUF, recording it so it can be undone.
pub(crate) fn insert_text<B: BufferTrait>(buf: &mut Buffer<B>, at: usize, content: &str) {
    if content.is_empty() {
        return;
    }
    let at = at.min(buf.text.len());
    let point = buf.text.cursor_pos_1d();
    buf.undo.record_insert(at, content.chars().count(), point);
    buf.mark = deactivated(buf.mark);
    undo::apply_insert(&mut buf.text, at, content);
    buf.is_modified = true;
}

/// A mark that survives an edit, but no longer defines a region.
///
/// Every edit goes through the two functions above, so this is every edit:
/// making a change is what ends a selection, in this editor as in Emacs.
/// Deactivating rather than clearing keeps the position available to
/// `exchange-point-and-mark`, and it is also what keeps an active region
/// honest -- an active mark can never have had an edit under it, so it never
/// needs adjusting for one.
fn deactivated(mark: Option<Mark>) -> Option<Mark> {
    mark.map(|mark| Mark {
        active: false,
        ..mark
    })
}

/// Insert CONTENT at point.
pub(crate) fn insert_at_point<B: BufferTrait>(buf: &mut Buffer<B>, content: &str) {
    let at = buf.text.cursor_pos_1d();
    insert_text(buf, at, content);
}

/// Delete `[from, to)` and return what was deleted, ready for the kill ring.
///
/// The text comes back rather than going straight into the ring because the
/// caller is holding the buffer lock and the ring is a lock of its own. Taking
/// the second while holding the first would put an ordering between them that
/// nothing else in the editor respects -- so every kill command drops the
/// buffer first and saves afterwards.
fn cut_out<B: BufferTrait>(buf: &mut Buffer<B>, from: usize, to: usize) -> String {
    let start = from.min(to);
    let end = from.max(to).min(buf.text.len());
    if start >= end {
        return String::new();
    }
    let text: String = (start..end).filter_map(|i| buf.text.at(i)).collect();
    delete_range(buf, start, end);
    text
}

pub const SELF_INSERT_DOC: &str = "(self-insert STRING): Insert the first character of STRING at point \
         in the current buffer. Unlike Emacs's `self-insert-command`, which \
         reads the character to insert from `last-command-event`, this takes \
         the character explicitly as an argument.\n\n\
         Example:\n\
         (self-insert \"a\") ; inserts the character a at point";

primitive!(self_insert, args, _env, ctx, {
    if let Some(ELispExp::String(s)) = args.first() {
        if let Some(c) = s.chars().next() {
            ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
                let mut utf8 = [0u8; 4];
                insert_at_point(buf, c.encode_utf8(&mut utf8));
            });
        }
        Ok(ELispExp::symbol("nil".into()))
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        })
    }
});

pub const INSERT_NEWLINE_DOC: &str = "(insert-newline): Insert a newline character at point in the current \
         buffer.\n\n\
         Example:\n\
         (define-key nil \"<ret>\" 'insert-newline)";

primitive!(insert_newline, _args, _env, ctx, {
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        insert_at_point(buf, "\n");
    });
    Ok(ELispExp::nil())
});

pub const DELETE_BACKWARD_CHAR_DOC: &str = "(delete-backward-char): Delete the character before point in the \
         current buffer. Unlike Emacs's command of the same name, this takes \
         no count argument -- it always deletes exactly one character.\n\n\
         Example:\n\
         (define-key nil \"<backspace>\" 'delete-backward-char)";

primitive!(delete_backward_char, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf
        .write()
        .expect("Failed to acquire a write lock on buffer");
    let point = buf.text.cursor_pos_1d();
    delete_range(&mut buf, point.saturating_sub(1), point);
    Ok(ELispExp::nil())
});

pub const FORWARD_CHAR_DOC: &str = "(forward-char &optional N): Move point forward N characters (default \
         1) in the current buffer.\n\n\
         Example:\n\
         (forward-char)   ; move forward 1 character\n\
         (forward-char 4) ; move forward 4 characters";

primitive!(forward_char, args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf
        .write()
        .expect("Failed to acquire a write lock on buffer");
    let step = repeat_count(args)?;
    // Moves by character *offset*, so it crosses line boundaries the way C-f
    // does. It used to move within the line only, which meant point simply
    // stopped at the end of a line and would not advance past it.
    let target = buf.text.cursor_pos_1d() + step;
    goto_offset(&mut buf.text, target);
    Ok(ELispExp::nil())
});

/// Read an optional repeat count from a primitive's arguments.
fn repeat_count<B: BufferTrait>(args: &[ELispExp<B>]) -> Result<usize, EvalError<EditorState<B>>> {
    match args.first() {
        None => Ok(1),
        Some(ELispExp::Number(n)) => Ok(n.floor().max(0.0) as usize),
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: other.clone(),
        }),
    }
}

/// Move point to a 1-D character offset, clamped to the buffer.
fn goto_offset<B: BufferTrait>(text: &mut B, offset: usize) {
    let target = offset.min(text.len());
    let (line, col) = text.cursor_1d_to_2d(target);
    text.cursor_move(line, col);
}

/// Characters that make up a word. Matches what most modes mean by one
/// without needing a syntax table yet.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A line with nothing but whitespace on it -- what separates paragraphs.
fn is_blank_line<B: BufferTrait>(text: &B, line: usize) -> bool {
    text.get_lines(line, line + 1)
        .first()
        .map(|l| l.trim().is_empty())
        .unwrap_or(true)
}

/// Offset `count` words forward of `from`.
///
/// Factored out so `forward-word` and `kill-word` agree by construction: a
/// deletion that computed its own target could drift from the movement it is
/// supposed to mirror.
fn word_forward<B: BufferTrait>(text: &B, from: usize, count: usize) -> usize {
    let len = text.len();
    let mut pos = from;
    for _ in 0..count {
        while pos < len && !text.at(pos).map(is_word_char).unwrap_or(false) {
            pos += 1;
        }
        while pos < len && text.at(pos).map(is_word_char).unwrap_or(false) {
            pos += 1;
        }
    }
    pos
}

/// Offset `count` words back of `from`.
fn word_backward<B: BufferTrait>(text: &B, from: usize, count: usize) -> usize {
    let mut pos = from;
    for _ in 0..count {
        while pos > 0 && !text.at(pos - 1).map(is_word_char).unwrap_or(false) {
            pos -= 1;
        }
        while pos > 0 && text.at(pos - 1).map(is_word_char).unwrap_or(false) {
            pos -= 1;
        }
    }
    pos
}

/// Line that `forward-paragraph` would land on from `line`.
fn paragraph_forward<B: BufferTrait>(text: &B, line: usize) -> usize {
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

/// Line that `backward-paragraph` would land on from `line`.
fn paragraph_backward<B: BufferTrait>(text: &B, line: usize) -> usize {
    let mut line = line;
    while line > 0 && is_blank_line(text, line) {
        line -= 1;
    }
    while line > 0 && !is_blank_line(text, line) {
        line -= 1;
    }
    line
}

fn line_length<B: BufferTrait>(text: &B, line: usize) -> usize {
    text.get_lines(line, line + 1)
        .first()
        .map(|l| l.chars().count())
        .unwrap_or(0)
}

pub const BACKWARD_CHAR_DOC: &str = "(backward-char &optional N): Move point backward N characters \
         (default 1) in the current buffer, stopping at the beginning of the \
         line.\n\n\
         Example:\n\
         (backward-char)   ; move back 1 character\n\
         (backward-char 4) ; move back 4 characters";

primitive!(backward_char, args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf
        .write()
        .expect("Failed to acquire a write lock on buffer");
    let step = repeat_count(args)?;
    let target = buf.text.cursor_pos_1d().saturating_sub(step);
    goto_offset(&mut buf.text, target);
    Ok(ELispExp::nil())
});

pub const PREVIOUS_LINE_DOC: &str = "(previous-line): Move point up one line in the current buffer, \
         keeping the same column (clamped to that line's length), stopping at \
         the first line.\n\n\
         Example:\n\
         (define-key nil \"<up>\" 'previous-line)";

primitive!(previous_line, args, _env, ctx, {
    move_line(ctx, -(repeat_count(args)? as isize))
});

pub const NEXT_LINE_DOC: &str = "(next-line): Move point down one line in the current buffer, keeping \
         the same column.\n\n\
         Example:\n\
         (define-key nil \"<down>\" 'next-line)";

primitive!(next_line, args, _env, ctx, {
    move_line(ctx, repeat_count(args)? as isize)
});

/// Move point one line up or down, keeping the column the user is aiming for.
///
/// The subtlety is the *goal column*. Moving down through a short line clamps
/// the column, so re-reading the cursor on the next move would lose the
/// original column for good -- go down past a short line and back up, and you
/// end up somewhere you did not start. The column is therefore remembered
/// across a run of vertical moves and only re-read when the previous command
/// was something else, which is what `last-command` is for.
fn move_line<B: BufferTrait>(
    ctx: &EditorState<B>,
    delta: isize,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let buf = ctx.get_current_buffer();
    let mut buf = buf
        .write()
        .expect("Failed to acquire a write lock on buffer");

    let (line, col) = buf.text.cursor_pos();
    let continuing = ctx.last_command_is("next-line") || ctx.last_command_is("previous-line");
    let goal = match (continuing, ctx.goal_column()) {
        (true, Some(goal)) => goal,
        _ => col,
    };
    ctx.set_goal_column(Some(goal));

    let last_line = buf.text.line_count().saturating_sub(1);
    let target = if delta < 0 {
        line.saturating_sub(delta.unsigned_abs())
    } else {
        (line + delta as usize).min(last_line)
    };
    // `cursor_move` clamps the column to the line, so a short line in the
    // middle of a run does not disturb the goal we are steering by.
    buf.text.cursor_move(target, goal);
    Ok(ELispExp::nil())
}

// ---------------------------------------------------------------------------
// Line, word, paragraph and buffer movement
// ---------------------------------------------------------------------------

pub const BEGINNING_OF_LINE_DOC: &str = "(beginning-of-line): Move point to the first character of \
         the current line.\n\n\
         Example:\n\
         (define-key nil \"C-a\" 'beginning-of-line)";

primitive!(beginning_of_line, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let (line, _) = buf.text.cursor_pos();
    buf.text.cursor_move(line, 0);
    Ok(ELispExp::nil())
});

pub const END_OF_LINE_DOC: &str = "(end-of-line): Move point just past the last character of the \
         current line, before its newline.\n\n\
         Example:\n\
         (define-key nil \"C-e\" 'end-of-line)";

primitive!(end_of_line, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let (line, _) = buf.text.cursor_pos();
    let end = line_length(&buf.text, line);
    buf.text.cursor_move(line, end);
    Ok(ELispExp::nil())
});

pub const FORWARD_WORD_DOC: &str = "(forward-word &optional N): Move point forward past the end of \
         the next N words (default 1). A word is a run of letters, digits or \
         underscores; anything between words is skipped.\n\n\
         Example:\n\
         (define-key nil \"M-f\" 'forward-word)";

primitive!(forward_word, args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let target = word_forward(&buf.text, buf.text.cursor_pos_1d(), repeat_count(args)?);
    goto_offset(&mut buf.text, target);
    Ok(ELispExp::nil())
});

pub const BACKWARD_WORD_DOC: &str = "(backward-word &optional N): Move point back to the beginning \
         of the Nth previous word (default 1).\n\n\
         Example:\n\
         (define-key nil \"M-b\" 'backward-word)";

primitive!(backward_word, args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let target = word_backward(&buf.text, buf.text.cursor_pos_1d(), repeat_count(args)?);
    goto_offset(&mut buf.text, target);
    Ok(ELispExp::nil())
});

pub const FORWARD_PARAGRAPH_DOC: &str = "(forward-paragraph): Move point to the blank line that \
         ends the current paragraph, or to the end of the buffer. Paragraphs \
         are separated by blank lines.\n\n\
         Example:\n\
         (define-key nil \"M-n\" 'forward-paragraph)";

primitive!(forward_paragraph, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let (line, _) = buf.text.cursor_pos();
    let target = paragraph_forward(&buf.text, line);
    buf.text.cursor_move(target, 0);
    Ok(ELispExp::nil())
});

pub const BACKWARD_PARAGRAPH_DOC: &str = "(backward-paragraph): Move point to the blank line that \
         begins the current paragraph, or to the beginning of the buffer.\n\n\
         Example:\n\
         (define-key nil \"M-p\" 'backward-paragraph)";

primitive!(backward_paragraph, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let (line, _) = buf.text.cursor_pos();
    let target = paragraph_backward(&buf.text, line);
    buf.text.cursor_move(target, 0);
    Ok(ELispExp::nil())
});

pub const BEGINNING_OF_BUFFER_DOC: &str = "(beginning-of-buffer): Move point to the very start of \
         the buffer.\n\n\
         Example:\n\
         (define-key nil \"M-<\" 'beginning-of-buffer)";

primitive!(beginning_of_buffer, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    buf.text.cursor_move(0, 0);
    Ok(ELispExp::nil())
});

pub const END_OF_BUFFER_DOC: &str = "(end-of-buffer): Move point to the very end of the buffer.\n\n\
         Example:\n\
         (define-key nil \"M->\" 'end-of-buffer)";

primitive!(end_of_buffer, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let end = buf.text.len();
    goto_offset(&mut buf.text, end);
    Ok(ELispExp::nil())
});

pub const GOTO_LINE_DOC: &str = "(goto-line N): Move point to the beginning of line N, counting \
         from 1. Registered as a command, so M-x goto-line prompts for the \
         number.\n\n\
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
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    // 1-based for the user, 0-based inside, and clamped so a number past the
    // end lands on the last line rather than failing.
    let last = buf.text.line_count().saturating_sub(1);
    let line = (requested.max(1.0) as usize - 1).min(last);
    buf.text.cursor_move(line, 0);
    Ok(ELispExp::nil())
});

// ---------------------------------------------------------------------------
// Deletion, mirroring the movement commands above
// ---------------------------------------------------------------------------
//
// Each of these removes exactly the text the corresponding movement command
// would have travelled over, by asking the same helper where that movement
// ends. That is why they cannot drift apart: `kill-word` deletes to
// `word_forward`, which is the offset `forward-word` moves to.
//
// A note on the names. In Emacs a *kill* also copies the text to the kill ring
// so it can be yanked back, while a *delete* discards it. That distinction is
// live here: the `kill-*` commands below save what they remove, and
// `delete-char` and `delete-backward-char` do not. Keeping the Emacs names
// meant the ring could be added to these commands later without renaming them
// out from under anyone's configuration, which is exactly what happened.

pub const DELETE_CHAR_DOC: &str = "(delete-char &optional N): Delete N characters (default 1) \
         forward from point, crossing line boundaries. Deletes nothing at the \
         end of the buffer.\n\n\
         Example:\n\
         (define-key nil \"C-d\" 'delete-char)";

primitive!(delete_char, args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let from = buf.text.cursor_pos_1d();
    let to = from + repeat_count(args)?;
    delete_range(&mut buf, from, to);
    Ok(ELispExp::nil())
});

pub const KILL_LINE_DOC: &str = "(kill-line): Delete from point to the end of the line. When point \
         is already at the end of a line, delete the newline instead, joining \
         the next line onto this one.\n\n\
         The text is saved to the kill ring, so `yank' puts it back. A run \
         of kill commands accumulates into one entry.\n\n\
         Example:\n\
         (define-key nil \"C-k\" 'kill-line)";

primitive!(kill_line, _args, _env, ctx, {
    let killed = {
        let buf = ctx.get_current_buffer();
        let mut buf = buf.write().expect("write lock on buffer");
        let from = buf.text.cursor_pos_1d();
        let (line, _) = buf.text.cursor_pos();
        let end_of_line = buf.text.cursor_2d_to_1d(line, line_length(&buf.text, line));
        // At the end of a line there is nothing left to kill on it, so the
        // newline goes instead -- which is what makes repeated C-k swallow a
        // paragraph rather than stalling on every line ending.
        let to = if from == end_of_line {
            from + 1
        } else {
            end_of_line
        };
        cut_out(&mut buf, from, to)
    };
    ctx.kill(killed, Direction::Forward);
    Ok(ELispExp::nil())
});

pub const KILL_WHOLE_LINE_DOC: &str = "(kill-whole-line): Delete the entire line point is on, \
         including its newline.\n\n\
         The text is saved to the kill ring, so `yank' puts it back. A run \
         of kill commands accumulates into one entry.";

primitive!(kill_whole_line, _args, _env, ctx, {
    let killed = {
        let buf = ctx.get_current_buffer();
        let mut buf = buf.write().expect("write lock on buffer");
        let (line, _) = buf.text.cursor_pos();
        let from = buf.text.cursor_2d_to_1d(line, 0);
        let to =
            (buf.text.cursor_2d_to_1d(line, line_length(&buf.text, line)) + 1).min(buf.text.len());
        cut_out(&mut buf, from, to)
    };
    ctx.kill(killed, Direction::Forward);
    Ok(ELispExp::nil())
});

pub const KILL_WORD_DOC: &str = "(kill-word &optional N): Delete forward to the end of the Nth next \
         word (default 1) -- the text `forward-word' would move over.\n\n\
         The text is saved to the kill ring, so `yank' puts it back. A run \
         of kill commands accumulates into one entry.\n\n\
         Example:\n\
         (define-key nil \"M-d\" 'kill-word)";

primitive!(kill_word, args, _env, ctx, {
    let killed = {
        let buf = ctx.get_current_buffer();
        let mut buf = buf.write().expect("write lock on buffer");
        let from = buf.text.cursor_pos_1d();
        let to = word_forward(&buf.text, from, repeat_count(args)?);
        cut_out(&mut buf, from, to)
    };
    ctx.kill(killed, Direction::Forward);
    Ok(ELispExp::nil())
});

pub const BACKWARD_KILL_WORD_DOC: &str = "(backward-kill-word &optional N): Delete back to the \
         beginning of the Nth previous word (default 1) -- the text \
         `backward-word' would move over.\n\n\
         The text is saved to the kill ring, so `yank' puts it back. A run \
         of kill commands accumulates into one entry.\n\n\
         Example:\n\
         (define-key nil \"M-<backspace>\" 'backward-kill-word)";

primitive!(backward_kill_word, args, _env, ctx, {
    let killed = {
        let buf = ctx.get_current_buffer();
        let mut buf = buf.write().expect("write lock on buffer");
        let to = buf.text.cursor_pos_1d();
        let from = word_backward(&buf.text, to, repeat_count(args)?);
        cut_out(&mut buf, from, to)
    };
    ctx.kill(killed, Direction::Backward);
    Ok(ELispExp::nil())
});

pub const KILL_PARAGRAPH_DOC: &str = "(kill-paragraph): Delete forward to the end of the current \
         paragraph -- the text `forward-paragraph' would move over.\n\n\
         The text is saved to the kill ring, so `yank' puts it back. A run \
         of kill commands accumulates into one entry.";

primitive!(kill_paragraph, _args, _env, ctx, {
    let killed = {
        let buf = ctx.get_current_buffer();
        let mut buf = buf.write().expect("write lock on buffer");
        let from = buf.text.cursor_pos_1d();
        let (line, _) = buf.text.cursor_pos();
        let target_line = paragraph_forward(&buf.text, line);
        let to = buf.text.cursor_2d_to_1d(target_line, 0);
        cut_out(&mut buf, from, to)
    };
    ctx.kill(killed, Direction::Forward);
    Ok(ELispExp::nil())
});

pub const BACKWARD_KILL_PARAGRAPH_DOC: &str = "(backward-kill-paragraph): Delete back to the \
         beginning of the current paragraph -- the text `backward-paragraph' \
         would move over.\n\n\
         The text is saved to the kill ring, so `yank' puts it back. A run \
         of kill commands accumulates into one entry.";

primitive!(backward_kill_paragraph, _args, _env, ctx, {
    let killed = {
        let buf = ctx.get_current_buffer();
        let mut buf = buf.write().expect("write lock on buffer");
        let to = buf.text.cursor_pos_1d();
        let (line, _) = buf.text.cursor_pos();
        let target_line = paragraph_backward(&buf.text, line);
        let from = buf.text.cursor_2d_to_1d(target_line, 0);
        cut_out(&mut buf, from, to)
    };
    ctx.kill(killed, Direction::Backward);
    Ok(ELispExp::nil())
});

// ---------------------------------------------------------------------------
// Undo and redo
// ---------------------------------------------------------------------------

pub const UNDO_DOC: &str = "(undo): Undo the most recent group of changes in the current buffer, \
         and move point to where that change happened. Returns t if something \
         was undone, nil if the history is empty.\n\n\
         A group is one command's worth of editing, except that a run of \
         ordinary typing amalgamates into groups of about twenty characters \
         so that undo does not step one keystroke at a time.\n\n\
         Example:\n\
         (define-key nil \"C-/\" 'undo)";

primitive!(undo, _args, _env, ctx, {
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        // Split borrow: the history and the text it describes are two fields
        // of the same buffer, and undo needs to write both.
        let Buffer { text, undo, .. } = buf;
        match undo.undo(text) {
            Some(point) => {
                goto_offset(text, point);
                buf.is_modified = true;
                Ok(ELispExp::symbol("t".into()))
            }
            None => Ok(ELispExp::nil()),
        }
    })
});

pub const REDO_DOC: &str = "(redo): Redo the most recently undone group of changes in the current \
         buffer. Returns t if something was redone, nil if there is nothing to \
         redo.\n\n\
         Making a new edit after an undo discards what could have been redone, \
         which is the usual editor behaviour: history is a stack of undone \
         changes, not a tree of alternatives.\n\n\
         Example:\n\
         (define-key nil \"M-_\" 'redo)";

primitive!(redo, _args, _env, ctx, {
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        let Buffer { text, undo, .. } = buf;
        match undo.redo(text) {
            Some(point) => {
                goto_offset(text, point);
                buf.is_modified = true;
                Ok(ELispExp::symbol("t".into()))
            }
            None => Ok(ELispExp::nil()),
        }
    })
});

pub const UNDO_BOUNDARY_DOC: &str = "(undo-boundary): End the current undo group, so that the next \
         change begins a new one and the two undo separately. Does nothing if \
         no group is open.\n\n\
         The editor already places a boundary between commands; this is for a \
         Lisp function that makes several edits and wants them undone in \
         steps rather than all at once.\n\n\
         Example:\n\
         (self-insert \"a\")\n\
         (undo-boundary)\n\
         (self-insert \"b\") ; (undo) removes only b";

primitive!(undo_boundary, _args, _env, ctx, {
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| buf.undo.boundary());
    Ok(ELispExp::nil())
});

pub const SET_UNDO_LIMIT_DOC: &str = "(set-undo-limit N): Keep at most N bytes of deleted text in the \
         current buffer's undo history, dropping the oldest changes when it \
         grows past that. The most recent change is always kept, so a single \
         deletion larger than the limit stays undoable.\n\n\
         Only deletions carry text, so this bounds the one part of the history \
         that grows with the size of the document rather than with the number \
         of edits. The default is one mebibyte.\n\n\
         Example:\n\
         (set-undo-limit 0) ; keep only the most recent change";

primitive!(set_undo_limit, args, _env, ctx, {
    let limit = match args.first() {
        Some(ELispExp::Number(n)) => n.floor().max(0.0) as usize,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| buf.undo.set_limit(limit));
    Ok(ELispExp::nil())
});

// ---------------------------------------------------------------------------
// Point, as a number
// ---------------------------------------------------------------------------
//
// Everything above moves point by *describing* the move -- a character, a word,
// a line. That covers editing, and it is useless to anything that has computed
// a position and simply wants to go there: a search result, a saved place, a
// jump back to where a command started. These three are that missing half, and
// they are counted in characters because point, mark, the region and the undo
// history all are.

pub const POINT_DOC: &str = "(point): Return the position of point in the current buffer, as a \
         character offset from the beginning. The first position is 0.\n\n\
         Example:\n\
         (point) => 42";

primitive!(point, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let buf = buf.read().expect("read lock on buffer");
    Ok(ELispExp::number(buf.text.cursor_pos_1d() as f64))
});

pub const POINT_MIN_DOC: &str = "(point-min): Return the first position in the current buffer, \
         which is always 0. Present so that code walking a buffer can name both ends rather than \
         writing one of them as a literal.";

primitive!(point_min, _args, _env, _ctx, { Ok(ELispExp::number(0.0)) });

pub const POINT_MAX_DOC: &str = "(point-max): Return the position just past the last character of \
         the current buffer -- the position `point' reaches at the end of the text.\n\n\
         Example:\n\
         (goto-char (point-max))   ; to the end";

primitive!(point_max, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let buf = buf.read().expect("read lock on buffer");
    Ok(ELispExp::number(buf.text.len() as f64))
});

pub const GOTO_CHAR_DOC: &str = "(goto-char POSITION): Move point to POSITION, a character offset \
         from the beginning of the buffer, and return where it ended up.\n\n\
         POSITION is clamped to the buffer rather than refused, so a position \
         computed before an edit still lands somewhere sensible afterwards \
         instead of failing.\n\n\
         Example:\n\
         (goto-char 0)             ; to the beginning\n\
         (goto-char (point-max))   ; to the end";

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
    Ok(ELispExp::number(ctx.mutate_buffer(
        ctx.get_current_buffer(),
        |buf| {
            // Clamped at both ends: a negative offset is the beginning, and
            // anything past the text is the end. A position outside the buffer
            // is almost always one computed before an edit, and putting point
            // somewhere real is more useful than refusing.
            let target = requested.max(0.0) as usize;
            let target = target.min(buf.text.len());
            let (line, col) = buf.text.cursor_1d_to_2d(target);
            buf.text.cursor_move(line, col);
            target as f64
        },
    )))
});
