use super::*;

pub const MAKE_FLOATING_WINDOW_DOC: &str = "(make-floating-window BUFFER-NAME X Y WIDTH HEIGHT \
         &optional TITLE MODE): Create a new buffer named BUFFER-NAME in \
         major mode MODE (a symbol; defaults to fundamental-mode if \
         omitted), open it in a new bordered floating window positioned \
         at (X, Y) with the given WIDTH and HEIGHT (and optional TITLE \
         string), give that window focus, and return t. Closing the \
         floating window (e.g. via close-buffer or \
         close-floating-window) restores focus to whatever window was \
         focused before this call. Not a standard Elisp primitive.\n\n\
         Example:\n\
         (make-floating-window \"*Minibuffer*\" 0 20 80 1 \"Find file\" 'minibuffer-mode)";

primitive!(make_floating_window, args, _env, ctx, {
    if args.len() < 5 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 5,
            got: args.len(),
        })
    } else {
        if let (
            ELispExp::String(buf_name),
            ELispExp::Number(x),
            ELispExp::Number(y),
            ELispExp::Number(w),
            ELispExp::Number(h),
        ) = (&args[0], &args[1], &args[2], &args[3], &args[4])
        {
            let title = args.get(5).and_then(|exp| {
                if let ELispExp::String(t) = exp {
                    Some(t.to_string())
                } else {
                    None
                }
            });
            let mode = args.get(6).and_then(|exp| match exp {
                ELispExp::Symbol(m) | ELispExp::String(m) => Some(m.to_string()),
                _ => None,
            });

            let previous_focused_window_id = ctx.get_focused_window_id();
            ctx.new_buffer(buf_name, None, mode);

            let new_id = ctx.get_next_window_id();
            let window = Window {
                id: new_id,
                buffer_name: buf_name.to_string(),
                scroll_x: 0,
                scroll_y: 0,
            };

            let rect = Rect {
                x: *x as isize,
                y: *y as isize,
                width: *w as usize,
                height: *h as usize,
            };

            let floating_win = FloatingWindow {
                window,
                rect,
                has_border: true,
                title,
                previous_focused_window_id,
            };

            ctx.floating_windows
                .write()
                .expect("Failed to acquire write lock on floating_windows")
                .push(floating_win);

            ctx.set_focused_window_id(new_id);
            ctx.set_current_buffer_name(buf_name);

            Ok(ELispExp::t())
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String, Number, Number, Number, Number".into(),
                got: format!("{:?}", args),
            })
        }
    }
});

primitive!(close_floating_window, _args, _env, ctx, {
    // TODO(improve) this still always closes the most-recently-opened
    // floating window rather than a specific one by name/id. Focus
    // restoration is now correct, though: it comes from the popped
    // window's own previous_focused_window_id rather than a hardcoded 0.
    let restore_id = {
        let mut floats = ctx
            .floating_windows
            .write()
            .expect("Failed to acquire write lock for floating_windows");
        floats.pop().map(|f| f.previous_focused_window_id)
    };
    ctx.set_focused_window_id(restore_id.unwrap_or(0));
    Ok(ELispExp::nil())
});
