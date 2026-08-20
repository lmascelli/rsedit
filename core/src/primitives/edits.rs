use super::*;

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
            got: format!("{:?}", args.first()),
        })
    }
});

primitive!(insert_newline, _args, _env, ctx, {
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        buf.text.insert('\n');
        buf.is_modified = true;
    });
    Ok(ELispExp::nil())
});

primitive!(delete_backward_char, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf
    .write()
    .expect("Failed to acquire a write lock on buffer");
    buf.text.delete();
    buf.is_modified = true;
    Ok(ELispExp::nil())
});

primitive!(forward_char, args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf
    .write()
    .expect("Failed to acquire a write lock on buffer");
    let step = if is_nil(args) {
        1
    } else {
        if let ELispExp::Number(n) = args[0] {
            n.floor() as usize
        } else {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: format!("{:?}", args[0]),
            });
        }
    };
    let (line, col) = buf.text.cursor_pos();
    buf.text.cursor_move(line, col + step);

    Ok(ELispExp::nil())
});

primitive!(backward_char, args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf
    .write()
    .expect("Failed to acquire a write lock on buffer");
    let step = if is_nil(args) {
        1
    } else {
        if let ELispExp::Number(n) = args[0] {
            n.floor() as usize
        } else {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: format!("{:?}", args[0]),
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

primitive!(next_line, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("Failed to acquire write lock on buffer");
    let (line, col) = buf.text.cursor_pos();
    buf.text.cursor_move(line + 1, col);
    Ok(ELispExp::nil())
});
