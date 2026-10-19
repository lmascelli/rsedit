use super::*;

pub const MAKE_FLOATING_WINDOW_DOC: &str = "(make-floating-window BUFFER-NAME X Y WIDTH HEIGHT \
         &optional TITLE MODE): Make a buffer called BUFFER-NAME in MODE, fundamental-mode by \
         default, open it in a bordered window floating at column X and row Y, WIDTH by HEIGHT, \
         with TITLE on its border, give it the focus, and return its id.\n\n\
         Example:\n\
         (make-floating-window \"*note*\" 2 1 40 6 \"Note\")";

primitive!(make_floating_window, args, env, ctx, {
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

            let id = ctx.open_floating_window(
                buf_name,
                *x as isize,
                *y as isize,
                *w as usize,
                *h as usize,
                title,
                mode,
                &env,
            );

            // An id rather than `t`, and still truthy either way: nothing in
            // this Lisp is false but nil, so a caller written as
            // `(if (make-floating-window ...) ...)` reads the same.
            Ok(id.into())
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String, Number, Number, Number, Number".into(),
                got: ELispExp::string(format!("{:?}", args)),
            })
        }
    }
});

pub const CLOSE_FLOATING_WINDOW_DOC: &str = "(close-floating-window): Close the newest floating \
         window and give the focus back to the window that had it before; do nothing when there \
         is none.";

primitive!(close_floating_window, _args, _env, ctx, {
    // TODO(improve) this still always closes the most-recently-opened
    // floating window rather than a specific one by name/id. Focus
    // restoration is now correct, though: it comes from the popped
    // window's own previous_focused_window_id rather than a hardcoded 0.
    let restore_id = ctx.windows_mut(|windows| {
        windows
            .floating_mut()
            .pop()
            .map(|f| f.previous_focused_window_id)
    });
    ctx.set_focused_window_id(restore_id.unwrap_or(0.into()));
    Ok(ELispExp::nil())
});

pub const RECENTER_DOC: &str = "(recenter &optional WHERE): Scroll the window so that point's \
         line is WHERE of the way down it: 0 the top, 0.5 the middle, which is the default, 1 the \
         bottom. Point does not move.";

primitive!(recenter, args, _env, ctx, {
    let where_to = match args.first() {
        Some(ELispExp::Number(n)) if n.is_finite() => *n,
        _ => 0.5,
    };
    ctx.recenter_focused_window(where_to);
    Ok(ELispExp::nil())
});

/// Register this module's primitives: what the view is asked to show.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("recenter", recenter, &[], RECENTER_DOC);
    into.function(
        "make-floating-window",
        make_floating_window,
        MAKE_FLOATING_WINDOW_DOC,
    );
    into.function(
        "close-floating-window",
        close_floating_window,
        CLOSE_FLOATING_WINDOW_DOC,
    );
}
