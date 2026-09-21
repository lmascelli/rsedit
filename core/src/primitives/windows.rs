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
         window in two, one above the other. The new window shows the same buffer at the same \
         place; focus stays where it was.\n\n\
         With ROWS, *this* window keeps exactly that many rows and the new one takes the rest. \
         Without it the two share the space evenly. Fixing the window being split rather than \
         the new one is the useful way round: a window that should stay a given height should \
         keep it when the frame is resized, and it is the other one that gives or takes.\n\n\
         Example:\n\
         (define-key nil \"C-x 2\" 'split-window-below)\n\
         (split-window-below 10)   ; this window keeps 10 rows";

primitive!(split_window_below, args, _env, ctx, {
    let division = split_division(args.first())?;
    Ok(
        match ctx.split_focused_window(Orientation::Horizontal, division) {
            Some(_) => ELispExp::t(),
            None => ELispExp::nil(),
        },
    )
});

pub const SPLIT_WINDOW_RIGHT_DOC: &str = "(split-window-right &optional COLUMNS): Split the focused \
         window in two, side by side. The new window shows the same buffer at the same place; \
         focus stays where it was.\n\n\
         With COLUMNS, *this* window keeps exactly that many columns and the new one takes the \
         rest -- the same meaning Emacs' `split-window-right' gives its SIZE. Without it the two \
         share the space evenly.\n\n\
         Example:\n\
         (define-key nil \"C-x 3\" 'split-window-right)\n\
         (split-window-right 40)   ; this window keeps 40 columns";

primitive!(split_window_right, args, _env, ctx, {
    let division = split_division(args.first())?;
    Ok(
        match ctx.split_focused_window(Orientation::Vertical, division) {
            Some(_) => ELispExp::t(),
            None => ELispExp::nil(),
        },
    )
});

pub const DELETE_WINDOW_DOC: &str = "(delete-window): Close the focused window; the window it was \
         split from grows into the space. Signals an error when it is the \
         only window, since a frame with none has nowhere to put the \
         cursor.\n\n\
         Example:\n\
         (define-key nil \"C-x 0\" 'delete-window)";

primitive!(delete_window, args, _env, ctx, {
    let closed = match args.first() {
        None => ctx.delete_focused_window(),
        Some(exp) if exp.is_nil() => ctx.delete_focused_window(),
        Some(ELispExp::Number(id)) => ctx.delete_window_by_id(*id as usize),
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
         BUFFER in a full-width window of exactly HEIGHT rows along the bottom of the frame, \
         and return the new window's id -- which is what `delete-window' needs to close it \
         again.\n\n\
         The frame is divided, not one window: the strip appears below everything, and every \
         window above it gives up a share of the space. Its height stays HEIGHT whatever the \
         terminal is resized to, which an ordinary split cannot promise -- a split holds a \
         fraction, and a fraction of a taller frame is more rows.\n\n\
         **Focus does not move.** A strip is shown while something else is being typed into, \
         and focus moving would make the strip's buffer current -- so the next keystroke would \
         go into the strip instead of wherever the user is looking. Nothing about a strip is \
         usable if this is not true.\n\n\
         Example:\n\
         (setq w (display-buffer-at-bottom \"*Completions*\" 6))\n\
         (delete-window w)";

primitive!(display_buffer_at_bottom, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
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
    if ctx.get_buffer(&name).is_none() {
        ctx.log_diagnostic(&format!("[LOG] buffer {name} does not exist."));
        return Ok(ELispExp::nil());
    }
    // At least one row: a strip of no rows is invisible, and a caller that
    // asked for one and got nothing would have no way to tell.
    let height = (height.max(1.0)) as usize;
    Ok(ELispExp::number(
        ctx.open_bottom_window(&name, height) as f64
    ))
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

pub const SCROLL_UP_COMMAND_DOC: &str = "(scroll-up-command &optional N): Move the focused window's \
         view forward through the buffer by N screenfuls (default 1), taking point along only if \
         it would otherwise be left behind.\n\n\
         A screenful is the window's height less `next-screen-context-lines' (2), so two pages \
         share a couple of lines and a reader can find their place. Point keeps the line it was \
         on whenever that line is still on screen, so reading a long file leaves the cursor where \
         the eye left it.\n\n\
         Reports \"End of buffer\" and does nothing when the last line is already at the top.\n\n\
         Example:\n\
         (define-key nil \"C-v\" 'scroll-up-command)";

primitive!(scroll_up_command, args, _env, ctx, {
    Ok(scrolled(ctx, count(args), "End of buffer"))
});

pub const SCROLL_DOWN_COMMAND_DOC: &str = "(scroll-down-command &optional N): Move the focused \
         window's view back through the buffer by N screenfuls (default 1). The mirror of \
         `scroll-up-command'; see it for what a screenful is and when point moves.\n\n\
         Reports \"Beginning of buffer\" and does nothing when the first line is already at the \
         top.\n\n\
         Example:\n\
         (define-key nil \"M-v\" 'scroll-down-command)";

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

pub const SELECTED_WINDOW_DOC: &str = "(selected-window): The id of the window that has focus.\n\n\
         An id is what `select-window' and `delete-window' take, so this is how a command \
         remembers a window to come back to. `dired' uses it for exactly that: `o' opens a file \
         beside the listing and remembers where it put it, so pressing `o' again replaces that \
         file rather than splitting the frame a second time.\n\n\
         Example:\n\
         (setq my-window (selected-window))";

primitive!(selected_window, _args, _env, ctx, {
    Ok(ELispExp::number(ctx.get_focused_window_id() as f64))
});

pub const SELECT_WINDOW_DOC: &str = "(select-window ID): Give focus to the window with ID. Returns \
         t, or nil if no window has that id.\n\n\
         Returning nil rather than signalling is the point: a window remembered earlier may have \
         been closed since, and a caller that checks the answer can fall back to opening a new \
         one. That is the whole of what \"reuse the window if it is still there\" needs.\n\n\
         A floating window's id works here too, though focus returns to whatever had it when the \
         float closes.\n\n\
         Example:\n\
         (if (select-window my-window) (find-file path) (progn (split-window-right) ...))";

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
    Ok(if ctx.select_window(id) {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});
