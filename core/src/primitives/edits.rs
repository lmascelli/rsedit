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
    let step = if args.is_empty() {
        1
    } else {
        if let ELispExp::Number(n) = args[0] {
            n.floor() as usize
        } else {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: args[0].clone(),
            });
        }
    };
    let (line, col) = buf.text.cursor_pos();
    buf.text.cursor_move(line, col + step);

    Ok(ELispExp::nil())
});

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
    let step = if args.is_empty() {
        1
    } else {
        if let ELispExp::Number(n) = args[0] {
            n.floor() as usize
        } else {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: args[0].clone(),
            });
        }
    };
    let (line, col) = buf.text.cursor_pos();
    if col >= step {
        buf.text.cursor_move(line, col - step);
    } else {
        buf.text.cursor_move(line, 0);
    }
    Ok(ELispExp::nil())
});

pub const PREVIOUS_LINE_DOC: &str = "(previous-line): Move point up one line in the current buffer, \
         keeping the same column (clamped to that line's length), stopping at \
         the first line.\n\n\
         Example:\n\
         (define-key nil \"<up>\" 'previous-line)";

primitive!(previous_line, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf
        .write()
        .expect("Failed to acquire a write lock on buffer");
    let (line, col) = buf.text.cursor_pos();
    if line > 0 {
        buf.text.cursor_move(line - 1, col);
    } else {
        buf.text.cursor_move(0, 0);
    }
    Ok(ELispExp::nil())
});

pub const NEXT_LINE_DOC: &str = "(next-line): Move point down one line in the current buffer, keeping \
         the same column.\n\n\
         Example:\n\
         (define-key nil \"<down>\" 'next-line)";

primitive!(next_line, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("Failed to acquire write lock on buffer");
    let (line, col) = buf.text.cursor_pos();
    buf.text.cursor_move(line + 1, col);
    Ok(ELispExp::nil())
});
