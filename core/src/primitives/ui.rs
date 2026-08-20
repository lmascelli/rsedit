use super::*;

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

            ctx.new_buffer(buf_name, None, None);

            let new_id = ctx.get_next_window_id();
            let window = Window {
                id: new_id,
                buffer_name: buf_name.to_string(),
                scroll_x: 0,
                scroll_y: 0,
            };

            let rect = Rect {
                x: *x as usize,
                y: *y as usize,
                width: *w as usize,
                height: *h as usize,
            };

            let floating_win = FloatingWindow {
                window,
                rect,
                has_border: true,
                title,
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
    // TODO(improve) this is a mess at the moment. It close the last floating opened and close
    // it not just toggle it so it has to be reopened. It must be improved to get the name of
    // the buffer or the id of the window to close or toggle. Moreover the last not floating
    // window id must be stored so when a window is closed the focus can be passed where it was.
    let mut floats = ctx
    .floating_windows
    .write()
    .expect("Failed to acquire write lock for floating_windows");
    floats.pop();
    ctx.set_focused_window_id(0);
    Ok(ELispExp::nil())
    });

