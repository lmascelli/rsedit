use super::*;

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
                buf.text.insert(c);
                buf.is_modified = true;
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
        buf.text.insert('\n');
        buf.is_modified = true;
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
    buf.text.delete();
    buf.is_modified = true;
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

/// Remove the text between two offsets, leaving point where the text began.
///
/// `delete()` removes the character *before* point, so this positions at the
/// far end and deletes backwards. Each of those is O(1) once the gap is there,
/// so the whole range costs one gap move rather than one per character.
fn delete_between<B: BufferTrait>(text: &mut B, from: usize, to: usize) {
    let start = from.min(to);
    let end = to.max(from).min(text.len());
    if start >= end {
        return;
    }
    goto_offset(text, end);
    for _ in 0..(end - start) {
        text.delete();
    }
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

primitive!(previous_line, _args, _env, ctx, { move_line(ctx, -1) });

pub const NEXT_LINE_DOC: &str = "(next-line): Move point down one line in the current buffer, keeping \
         the same column.\n\n\
         Example:\n\
         (define-key nil \"<down>\" 'next-line)";

primitive!(next_line, _args, _env, ctx, { move_line(ctx, 1) });

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
    let continuing = matches!(
        ctx.last_command().as_deref(),
        Some("next-line") | Some("previous-line")
    );
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
// so it can be yanked back, while a *delete* discards it. The ring is roadmap
// #20 and does not exist yet, so today these all discard. The Emacs names are
// used regardless, so that the bindings and muscle memory people already have
// keep working, and so that #20 adds saving to these commands rather than
// renaming them out from under anyone's configuration.

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
    delete_between(&mut buf.text, from, to);
    buf.is_modified = true;
    Ok(ELispExp::nil())
});

pub const KILL_LINE_DOC: &str = "(kill-line): Delete from point to the end of the line. When point \
         is already at the end of a line, delete the newline instead, joining \
         the next line onto this one.\n\n\
         Does not yet save to a kill ring -- see roadmap #20.\n\n\
         Example:\n\
         (define-key nil \"C-k\" 'kill-line)";

primitive!(kill_line, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let from = buf.text.cursor_pos_1d();
    let (line, _) = buf.text.cursor_pos();
    let end_of_line = buf.text.cursor_2d_to_1d(line, line_length(&buf.text, line));
    // At the end of a line there is nothing left to kill on it, so the newline
    // goes instead -- which is what makes repeated C-k swallow a paragraph
    // rather than stalling on every line ending.
    let to = if from == end_of_line {
        from + 1
    } else {
        end_of_line
    };
    delete_between(&mut buf.text, from, to);
    buf.is_modified = true;
    Ok(ELispExp::nil())
});

pub const KILL_WHOLE_LINE_DOC: &str = "(kill-whole-line): Delete the entire line point is on, \
         including its newline.\n\n\
         Does not yet save to a kill ring -- see roadmap #20.";

primitive!(kill_whole_line, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let (line, _) = buf.text.cursor_pos();
    let from = buf.text.cursor_2d_to_1d(line, 0);
    let to = (buf.text.cursor_2d_to_1d(line, line_length(&buf.text, line)) + 1).min(buf.text.len());
    delete_between(&mut buf.text, from, to);
    buf.is_modified = true;
    Ok(ELispExp::nil())
});

pub const KILL_WORD_DOC: &str = "(kill-word &optional N): Delete forward to the end of the Nth next \
         word (default 1) -- the text `forward-word' would move over.\n\n\
         Does not yet save to a kill ring -- see roadmap #20.\n\n\
         Example:\n\
         (define-key nil \"M-d\" 'kill-word)";

primitive!(kill_word, args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let from = buf.text.cursor_pos_1d();
    let to = word_forward(&buf.text, from, repeat_count(args)?);
    delete_between(&mut buf.text, from, to);
    buf.is_modified = true;
    Ok(ELispExp::nil())
});

pub const BACKWARD_KILL_WORD_DOC: &str = "(backward-kill-word &optional N): Delete back to the \
         beginning of the Nth previous word (default 1) -- the text \
         `backward-word' would move over.\n\n\
         Does not yet save to a kill ring -- see roadmap #20.\n\n\
         Example:\n\
         (define-key nil \"M-<backspace>\" 'backward-kill-word)";

primitive!(backward_kill_word, args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let to = buf.text.cursor_pos_1d();
    let from = word_backward(&buf.text, to, repeat_count(args)?);
    delete_between(&mut buf.text, from, to);
    buf.is_modified = true;
    Ok(ELispExp::nil())
});

pub const KILL_PARAGRAPH_DOC: &str = "(kill-paragraph): Delete forward to the end of the current \
         paragraph -- the text `forward-paragraph' would move over.\n\n\
         Does not yet save to a kill ring -- see roadmap #20.";

primitive!(kill_paragraph, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let from = buf.text.cursor_pos_1d();
    let (line, _) = buf.text.cursor_pos();
    let target_line = paragraph_forward(&buf.text, line);
    let to = buf.text.cursor_2d_to_1d(target_line, 0);
    delete_between(&mut buf.text, from, to);
    buf.is_modified = true;
    Ok(ELispExp::nil())
});

pub const BACKWARD_KILL_PARAGRAPH_DOC: &str = "(backward-kill-paragraph): Delete back to the \
         beginning of the current paragraph -- the text `backward-paragraph' \
         would move over.\n\n\
         Does not yet save to a kill ring -- see roadmap #20.";

primitive!(backward_kill_paragraph, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    let to = buf.text.cursor_pos_1d();
    let (line, _) = buf.text.cursor_pos();
    let target_line = paragraph_backward(&buf.text, line);
    let from = buf.text.cursor_2d_to_1d(target_line, 0);
    delete_between(&mut buf.text, from, to);
    buf.is_modified = true;
    Ok(ELispExp::nil())
});
