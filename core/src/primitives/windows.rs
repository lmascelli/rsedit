//! Splitting the frame into windows, and moving between them.
use super::*;
use crate::ui::Orientation;

/// Read a window count from a `p` argument.
fn count(args: &[ELispExp<impl BufferTrait>]) -> isize {
    match args.first() {
        Some(ELispExp::Number(n)) => *n as isize,
        _ => 1,
    }
}

pub const SPLIT_WINDOW_BELOW_DOC: &str = "(split-window-below): Split the focused window in two, one \
         above the other. The new window shows the same buffer at the same \
         place; focus stays where it was.\n\n\
         Example:\n\
         (define-key nil \"C-x 2\" 'split-window-below)";

primitive!(split_window_below, _args, _env, ctx, {
    Ok(match ctx.split_focused_window(Orientation::Horizontal) {
        Some(_) => ELispExp::t(),
        None => ELispExp::nil(),
    })
});

pub const SPLIT_WINDOW_RIGHT_DOC: &str = "(split-window-right): Split the focused window in two, side \
         by side. The new window shows the same buffer at the same place; \
         focus stays where it was.\n\n\
         Example:\n\
         (define-key nil \"C-x 3\" 'split-window-right)";

primitive!(split_window_right, _args, _env, ctx, {
    Ok(match ctx.split_focused_window(Orientation::Vertical) {
        Some(_) => ELispExp::t(),
        None => ELispExp::nil(),
    })
});

pub const DELETE_WINDOW_DOC: &str = "(delete-window): Close the focused window; the window it was \
         split from grows into the space. Signals an error when it is the \
         only window, since a frame with none has nowhere to put the \
         cursor.\n\n\
         Example:\n\
         (define-key nil \"C-x 0\" 'delete-window)";

primitive!(delete_window, _args, _env, ctx, {
    if ctx.delete_focused_window() {
        Ok(ELispExp::t())
    } else {
        Err(EvalError::RuntimeMessage(
            "Attempt to delete minibuffer or sole ordinary window".into(),
        ))
    }
});

pub const DELETE_OTHER_WINDOWS_DOC: &str = "(delete-other-windows): Close every window but the \
         focused one, which grows to fill the frame.\n\n\
         Example:\n\
         (define-key nil \"C-x 1\" 'delete-other-windows)";

primitive!(delete_other_windows, _args, _env, ctx, {
    Ok(match ctx.delete_other_windows() {
        true => ELispExp::t(),
        false => ELispExp::nil(),
    })
});

pub const OTHER_WINDOW_DOC: &str = "(other-window &optional N): Move focus N windows on (default 1), \
         wrapping round at the end. A negative N moves the other way. \
         Returns nil when there is only one window.\n\n\
         Windows are ordered as they are laid out -- left to right, top to \
         bottom -- so cycling walks the frame the way it looks.\n\n\
         Example:\n\
         (define-key nil \"C-x o\" 'other-window)";

primitive!(other_window, args, _env, ctx, {
    Ok(match ctx.focus_other_window(count(args)) {
        true => ELispExp::t(),
        false => ELispExp::nil(),
    })
});

pub const COUNT_WINDOWS_DOC: &str = "(count-windows): Return how many tiled windows the frame holds. \
         Floating windows, such as an open minibuffer, are not counted.";

primitive!(count_windows, _args, _env, ctx, {
    Ok(ELispExp::number(ctx.window_count() as f64))
});

pub const WINDOW_BUFFER_DOC: &str = "(window-buffer): Return the name of the buffer shown by the \
         focused window.\n\n\
         Not always the current buffer: a minibuffer prompt makes itself \
         current without taking a tiled window's place.";

primitive!(window_buffer, _args, _env, ctx, {
    Ok(match ctx.focused_window_buffer() {
        Some(name) => ELispExp::string(name),
        None => ELispExp::nil(),
    })
});
