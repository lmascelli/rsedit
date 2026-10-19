//! Splitting the frame into windows, and moving between them.
use super::*;
use crate::ui::Division;
use crate::ui::Orientation;

/// Read a window count from a `p` argument.
fn count(args: &[ELispExp<impl BufferTrait>]) -> isize {
    match args.first() {
        Some(ELispExp::Number(n)) => *n as isize,
        _ => 1,
    }
}

/// The division a split gets, given an optional size argument.
///
/// With no size the two halves share the space evenly, as `C-x 2` and `C-x 3`
/// always have. With one, *this* window -- the one being split -- keeps exactly
/// that many rows or columns and the new one takes the rest, which is what
/// Emacs' `split-window-right SIZE` means too.
///
/// Fixing the window being split rather than the new one is the useful way
/// round: a directory listing that wants to stay 40 columns wide should keep
/// those 40 columns when the frame is resized, and it is the file beside it
/// that should grow.
fn split_division<B: BufferTrait>(
    size: Option<&ELispExp<B>>,
) -> Result<Division, EvalError<EditorState<B>>> {
    match size {
        None => Ok(Division::Ratio(0.5)),
        Some(exp) if exp.is_nil() => Ok(Division::Ratio(0.5)),
        Some(ELispExp::Number(n)) if *n >= 0.0 => Ok(Division::FirstFixed(*n as usize)),
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: "a non-negative Number".into(),
            got: other.clone(),
        }),
    }
}

pub const SPLIT_WINDOW_BELOW_DOC: &str = "(split-window-below &optional ROWS): Split the focused \
         window in two, one above the other, the new one showing the same buffer; the focus \
         stays. With ROWS, this window keeps that many rows and the new one takes the rest.";

primitive!(split_window_below, args, _env, ctx, {
    let division = split_division(args.first())?;
    Ok(
        match ctx.split_focused_window(Orientation::Horizontal, division) {
            Some(_) => ELispExp::t(),
            None => ELispExp::nil(),
        },
    )
});

pub const SPLIT_WINDOW_RIGHT_DOC: &str = "(split-window-right &optional COLUMNS): Split the \
         focused window in two, side by side, the new one showing the same buffer; the focus \
         stays. With COLUMNS, this window keeps that many columns and the new one takes the rest.\n\n\
         Example:\n\
         (split-window-right 40)";

primitive!(split_window_right, args, _env, ctx, {
    let division = split_division(args.first())?;
    Ok(
        match ctx.split_focused_window(Orientation::Vertical, division) {
            Some(_) => ELispExp::t(),
            None => ELispExp::nil(),
        },
    )
});

pub const DELETE_WINDOW_DOC: &str = "(delete-window &optional ID): Close the window with ID, the \
         focused one by default, giving its space back, and return t. Closing the only window is \
         an error.";

primitive!(delete_window, args, _env, ctx, {
    let closed = match args.first() {
        None => ctx.delete_focused_window(),
        Some(exp) if exp.is_nil() => ctx.delete_focused_window(),
        Some(ELispExp::Number(id)) => ctx.delete_window_by_id((*id as usize).into()),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number naming a window".into(),
                got: other.clone(),
            });
        }
    };
    if closed {
        Ok(ELispExp::t())
    } else {
        Err(EvalError::RuntimeMessage(
            "Attempt to delete minibuffer or sole ordinary window".into(),
        ))
    }
});

pub const DISPLAY_BUFFER_AT_BOTTOM_DOC: &str = "(display-buffer-at-bottom BUFFER HEIGHT): Show \
         BUFFER in a window HEIGHT rows high across the bottom of the frame, and return its id. \
         The height holds when the terminal is resized, and the focus does not move.\n\n\
         Example:\n\
         (setq strip (display-buffer-at-bottom \"*scratch*\" 6))\n\
         (delete-window strip)";

primitive!(display_buffer_at_bottom, args, _env, ctx, {
    exact_arity(args, 2)?;
    let name = match &args[0] {
        ELispExp::String(name) | ELispExp::Symbol(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String or Symbol naming a buffer".into(),
                got: other.clone(),
            });
        }
    };
    let ELispExp::Number(height) = &args[1] else {
        return Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: args[1].clone(),
        });
    };
    if !ctx.has_buffer(&name) {
        ctx.log_diagnostic(&format!("[LOG] buffer {name} does not exist."));
        return Ok(ELispExp::nil());
    }
    // At least one row: a strip of no rows is invisible, and a caller that
    // asked for one and got nothing would have no way to tell.
    let height = (height.max(1.0)) as usize;
    Ok(ctx.open_bottom_window(&name, height).into())
});

pub const DELETE_OTHER_WINDOWS_DOC: &str = "(delete-other-windows): Close every window but the \
         focused one, which then fills the frame.";

primitive!(delete_other_windows, _args, _env, ctx, {
    Ok(match ctx.delete_other_windows() {
        true => ELispExp::t(),
        false => ELispExp::nil(),
    })
});

pub const OTHER_WINDOW_DOC: &str = "(other-window &optional N): Give the focus to the window N on \
         from this one, 1 by default, the tiled windows left to right and top to bottom, then the \
         floating ones. A negative N goes the other way. Returns nil when there is one window.";

primitive!(other_window, args, _env, ctx, {
    Ok(match ctx.focus_other_window(count(args)) {
        true => ELispExp::t(),
        false => ELispExp::nil(),
    })
});

pub const COUNT_WINDOWS_DOC: &str = "(count-windows): Return how many tiled windows the frame \
         holds; floating windows are not counted.";

primitive!(count_windows, _args, _env, ctx, {
    Ok(ELispExp::number(ctx.window_count() as f64))
});

pub const WINDOW_BUFFER_DOC: &str = "(window-buffer): Return the name of the buffer the focused \
         window shows, which is not the current buffer while a prompt is open.";

primitive!(window_buffer, _args, _env, ctx, {
    Ok(match ctx.focused_window_buffer() {
        Some(name) => ELispExp::string(name),
        None => ELispExp::nil(),
    })
});

pub const SCROLL_UP_COMMAND_DOC: &str = "(scroll-up-command &optional N): Move the focused \
         window's view N screens forward, 1 by default, keeping two lines of the last screen in \
         sight. Point moves only when its line would leave the window. At the end of the buffer, \
         it says so.";

primitive!(scroll_up_command, args, _env, ctx, {
    Ok(scrolled(ctx, count(args), "End of buffer"))
});

pub const SCROLL_DOWN_COMMAND_DOC: &str = "(scroll-down-command &optional N): Move the focused \
         window's view N screens back, 1 by default, keeping two lines of the last screen in \
         sight. Point moves only when its line would leave the window. At the start of the \
         buffer, it says so.";

primitive!(scroll_down_command, args, _env, ctx, {
    Ok(scrolled(ctx, -count(args), "Beginning of buffer"))
});

/// Scroll by AMOUNT screenfuls, saying AT_THE_END when there was nowhere to go.
///
/// The report is the point of this being shared. A scroll key that does nothing
/// and says nothing is indistinguishable from one that is not bound, and the
/// user's next move is to press it harder.
fn scrolled<B: BufferTrait>(ctx: &EditorState<B>, amount: isize, at_the_end: &str) -> ELispExp<B> {
    if ctx.scroll_focused_window(amount) {
        ELispExp::t()
    } else {
        ctx.set_echo_message(at_the_end);
        ELispExp::nil()
    }
}

pub const SELECTED_WINDOW_DOC: &str = "(selected-window): Return the id of the window that has \
         the focus.\n\n\
         Example:\n\
         (setq my-window (selected-window))";

primitive!(selected_window, _args, _env, ctx, {
    Ok(ctx.get_focused_window_id().into())
});

pub const SELECT_WINDOW_DOC: &str = "(select-window ID): Give the focus to the window with ID, \
         and return t, or nil when no window has that id any longer.\n\n\
         Example:\n\
         (select-window my-window)";

primitive!(select_window, args, _env, ctx, {
    let id = match args.first() {
        Some(ELispExp::Number(n)) if *n >= 0.0 => *n as usize,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "a non-negative Number".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    Ok(if ctx.select_window(id.into()) {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

/// Register this module's primitives: splitting, focusing, resizing and closing windows.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // ---------------------------------------------------------------
    // Windows
    // ---------------------------------------------------------------
    into.command(
        "split-window-below",
        split_window_below,
        &[],
        SPLIT_WINDOW_BELOW_DOC,
    );
    into.command(
        "split-window-right",
        split_window_right,
        &[],
        SPLIT_WINDOW_RIGHT_DOC,
    );
    into.command("delete-window", delete_window, &[], DELETE_WINDOW_DOC);
    into.command(
        "delete-other-windows",
        delete_other_windows,
        &[],
        DELETE_OTHER_WINDOWS_DOC,
    );
    into.command("other-window", other_window, &["p"], OTHER_WINDOW_DOC);
    into.function("selected-window", selected_window, SELECTED_WINDOW_DOC);
    into.function("select-window", select_window, SELECT_WINDOW_DOC);
    into.function("count-windows", count_windows, COUNT_WINDOWS_DOC);
    // Moving the view rather than point. Commands, because `C-v` and `M-v` are
    // keys and because a prefix argument means screenfuls.
    into.command(
        "scroll-up-command",
        scroll_up_command,
        &["p"],
        SCROLL_UP_COMMAND_DOC,
    );
    into.command(
        "scroll-down-command",
        scroll_down_command,
        &["p"],
        SCROLL_DOWN_COMMAND_DOC,
    );
    into.function(
        "display-buffer-at-bottom",
        display_buffer_at_bottom,
        DISPLAY_BUFFER_AT_BOTTOM_DOC,
    );
    into.function("window-buffer", window_buffer, WINDOW_BUFFER_DOC);
}
