use super::*;

pub const CURRENT_BUFFER_DOC: &str = "(current-buffer): Return the name of the current buffer, as a \
         string. Unlike real Emacs Lisp's `current-buffer`, which returns a \
         buffer object, this returns the buffer's name.\n\n\
         Example:\n\
         (current-buffer) => \"*scratch*\"";

primitive!(current_buffer, _args, _env, ctx, {
    Ok(ELispExp::string(ctx.get_current_buffer_name()))
});

primitive!(close_buffer, args, _env, ctx, {
    if args.is_empty() {
        let current_buffer_name = ctx.get_current_buffer_name();
        if let Some(buffer) = ctx.get_buffer(&current_buffer_name) {
            if current_buffer_name == "*Minibuffer*" {
                todo!("Restore the LayoutNode and the focus to the previous window");
            }
            todo!(
                "Assign an other buffer to the current window. Find a way to store the last viewn buffer"
            );
            Ok(ELispExp::t())
        } else {
            Ok(ELispExp::nil())
        }
    } else {
        todo!("Extract the name of the buffer to close from the args. Close it if exists.");
        Ok(ELispExp::nil())
    }
});

pub const BUFFER_STRING_DOC: &str = "(buffer-string): Return the entire contents of the current buffer as \
         a string.\n\n\
         Example:\n\
         (buffer-string) => \"line one\\nline two\\n\"";

primitive!(buffer_string, _args, _env, ctx, {
    let buf = ctx.get_current_buffer();
    let content = buf
        .read()
        .expect("Failed to acquire read lock on current buffer")
        .text
        .to_string();
    Ok(ELispExp::string(content))
});

pub const CLEAR_BUFFER_DOC: &str = "(clear-buffer): Delete the entire contents of the current buffer. \
         Not a standard Elisp primitive -- comparable to Emacs's \
         `erase-buffer`.\n\n\
         Example:\n\
         (clear-buffer)\n\
         (buffer-string) => \"\"";

primitive!(clear_buffer, _args, _env, ctx, {
    // TODO(improve) this is highly inefficent. A clear function of BufferTrait must be added
    // to clear the buffer.
    ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        while buf.text.cursor_pos() != (0, 0) {
            buf.text.delete();
        }
    });

    Ok(ELispExp::nil())
});

pub const SWITCH_TO_BUFFER_DOC: &str = "(switch-to-buffer BUFFER-NAME): Make the buffer named BUFFER-NAME \
         (a string or symbol) the one shown in the focused window, and \
         return BUFFER-NAME. Returns nil (logging a diagnostic) if no buffer \
         with that name exists -- unlike real Emacs Lisp's \
         `switch-to-buffer`, this does not create one.\n\n\
         Example:\n\
         (switch-to-buffer \"*scratch*\") => \"*scratch*\"";

primitive!(switch_to_buffer, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        match &args[0] {
            ELispExp::String(buffer_name) => {
                if let Some(_) = ctx.get_buffer(buffer_name) {
                    if let Some(window) = ctx
                        .layout_root
                        .write()
                        .expect("Failed to acquire write lock on layout_root")
                        .get_window_by_id(ctx.get_focused_window_id())
                    {
                        window.buffer_name = buffer_name.to_string();
                    }
                    Ok(args[0].clone())
                } else {
                    ctx.log_diagnostic(&format!("[LOG] buffer {} does not exist.", buffer_name));
                    Ok(ELispExp::nil())
                }
            }
            ELispExp::Symbol(buffer_name) => {
                if let Some(_) = ctx.get_buffer(buffer_name) {
                    if let Some(window) = ctx
                        .layout_root
                        .write()
                        .expect("Failed to acquire write lock on layout_root")
                        .get_window_by_id(ctx.get_focused_window_id())
                    {
                        window.buffer_name = buffer_name.to_string();
                    }
                    Ok(args[0].clone())
                } else {
                    ctx.log_diagnostic(&format!("[LOG] buffer {} does not exist.", buffer_name));
                    Ok(ELispExp::nil())
                }
            }
            _ => Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: format!("{:?}", args.get(0)),
            }),
        }
    }
});
