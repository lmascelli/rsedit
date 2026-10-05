//! What a click and a wheel notch do.
//!
//! # Why these are commands and not code in the event handler
//!
//! `EditorState::handle_mouse_event` works out *where* the pointer was, which
//! needs the layout and every window's scroll and so can only happen there.
//! What that position should *mean* is a separate question, and routing it
//! through the command machinery buys the same things a keystroke gets: one
//! undo step, one `post-command-hook`, a name in the logs and in `M-x`, and a
//! binding that can be replaced without touching the handler.
//!
//! Their arguments come from the dispatcher rather than from a prompt, which
//! is why they register with no argument specs -- the same arrangement
//! `insert-pasted-text` has.
use super::*;
use crate::buffer::Mark;
use crate::ui::WindowId;
use crate::{
    editor::{MOUSE_MODE, mouse_mode},
    managers::MouseDrag,
};

/// How many lines one notch of the wheel moves.
///
/// Three, which is what a terminal's own scrollback does and therefore what
/// the hand already expects. Not a screenful: the wheel is for nudging, and
/// `C-v` is for pages.
const MOUSE_WHEEL_LINES: isize = 3;

fn number_arg<B: BufferTrait>(
    args: &[ELispExp<B>],
    index: usize,
) -> Result<f64, EvalError<EditorState<B>>> {
    match args.get(index) {
        Some(ELispExp::Number(n)) => Ok(*n),
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: other.clone(),
        }),
        None => Err(EvalError::WrongNumberOfArguments {
            expected: index + 1,
            got: args.len(),
        }),
    }
}

pub const MOUSE_SET_POINT_DOC: &str = "(mouse-set-point WINDOW LINE COLUMN): Focus WINDOW, put \
         point at LINE and COLUMN in it, and end any selection. Returns t, or nil if there is no \
         such window.\n\n\
         What a single left click runs. LINE and COLUMN are buffer coordinates -- the window's \
         scroll has already been added by the time this is called -- and both are clamped to the \
         buffer, so clicking past the end of a short line lands at its end rather than \
         nowhere.\n\n\
         The window is focused first and point set afterwards, in that order: focusing a window \
         restores the point it remembered, and the click has to win over it.\n\n\
         The selection ends because a click says where the cursor goes, not where a region \
         stops. See `mouse-start-selection' for the one that does the other thing.\n\n\
         Example:\n\
         (mouse-set-point 0 12 4)";

primitive!(mouse_set_point, args, _env, ctx, {
    Ok(ELispExp::boolean(click_at(args, ctx, Selecting::No)?))
});

pub const MOUSE_START_SELECTION_DOC: &str = "(mouse-start-selection WINDOW LINE COLUMN): Focus WINDOW, put point at LINE and COLUMN, and \
         begin a selection there. Returns t, or nil if there is no such window.\n\n\
         What a double left click runs. The region starts empty and grows as point moves, \
         whether it is moved by dragging or from the keyboard -- so a double click is the \
         mouse's way of saying `set-mark\', in the place where a single click says `goto\'.\n\n\
         Rebind the double click if you would rather it did something else -- selecting the word \
         under the pointer is the obvious other choice, and `bounds-of-thing-at-point\' is what \
         a Lisp command would use to do it.\n\n\
         Example:\n\
         (mouse-start-selection 0 12 4)";

primitive!(mouse_start_selection, args, _env, ctx, {
    Ok(ELispExp::boolean(click_at(args, ctx, Selecting::Yes)?))
});

/// Whether the click that lands here begins a region or ends one.
#[derive(Clone, Copy, PartialEq)]
enum Selecting {
    Yes,
    No,
}

/// Focus a window, move point, and either start a selection or end one.
///
/// # Why both clicks go through one function
///
/// They differ in a single line, and everything before it -- the window, the
/// clamping, the order the two happen in -- is the part that is easy to get
/// subtly wrong. Two copies of it would agree until one of them was fixed.
fn click_at<B: BufferTrait>(
    args: &[ELispExp<B>],
    ctx: &EditorState<B>,
    selecting: Selecting,
) -> Result<bool, EvalError<EditorState<B>>> {
    let window = number_arg(args, 0)?.max(0.0) as usize;
    let line = number_arg(args, 1)?.max(0.0) as usize;
    let column = number_arg(args, 2)?.max(0.0) as usize;
    if !ctx.select_window(WindowId(window)) {
        return Ok(false);
    }
    ctx.with_current_buffer_mut(|buf| {
        // Clamped against the buffer rather than against the window: the
        // screen has no opinion about how long a line is, and a click in the
        // blank space to the right of a short one means its end.
        let line = line.min(buf.text.line_count().saturating_sub(1));
        let column = column.min(edits::line_length(&buf.text, line));
        buf.text.cursor_move(line, column);
        match selecting {
            // Set *after* point has moved, so the region is anchored where the
            // click landed rather than where the cursor used to be.
            Selecting::Yes => buf.mark = Some(Mark::new(buf.text.cursor_pos_1d())),
            // # Why a click ends the selection
            //
            // A region is the span between the mark and point, so moving
            // point moves one end of it. Without this, every click while a
            // selection was up dragged that end to wherever you clicked --
            // including a click far away, which left a selection stretching
            // across half the file that nobody asked for. Clicking somewhere
            // is how you say "not there, here", and the selection is the
            // clearest casualty of it.
            //
            // Deactivated rather than forgotten: `exchange-point-and-mark'
            // can still go back to it, which is exactly what an inactive mark
            // is for.
            Selecting::No => {
                if let Some(mark) = buf.mark.as_mut() {
                    mark.active = false;
                }
            }
        }
    });
    Ok(true)
}

pub const MOUSE_SCROLL_DOC: &str = "(mouse-scroll WINDOW NOTCHES): Scroll WINDOW by NOTCHES of \
         the wheel, positive being towards the end of the buffer. Returns t if the view moved.\n\n\
         Neither focus nor point moves. Rolling the wheel over a window you are not working in \
         is a way of *looking* at it, and having it steal the cursor would make the next \
         keystroke land somewhere unexpected.\n\n\
         Example:\n\
         (mouse-scroll 0 1)";

primitive!(mouse_scroll, args, _env, ctx, {
    let window = number_arg(args, 0)?.max(0.0) as usize;
    let notches = number_arg(args, 1)? as isize;
    let moved = ctx.scroll_window_by(WindowId(window), notches * MOUSE_WHEEL_LINES);
    Ok(if moved {
        ELispExp::symbol("t".into())
    } else {
        ELispExp::nil()
    })
});

pub const MOUSE_MODE_TOGGLE_DOC: &str = "(mouse-mode-toggle): Turn the mouse on or off, and say \
         which. Returns t when it is now on.\n\n\
         A built-in command rather than three lines in the shipped configuration, because a \
         setting you cannot change without editing a file is a setting you will fight. It is in \
         Rust rather than Lisp for a duller reason: the configuration that turns the mouse on is \
         evaluated before the module defining `defcommand' has necessarily loaded, so a command \
         defined there is a command that cannot be defined.\n\n\
         Example:\n\
         (define-key nil \\\"C-c m\\\" 'mouse-mode-toggle)";

primitive!(mouse_mode_toggle, _args, env, ctx, {
    let now_on = !mouse_mode(&env);
    env.set_variable(
        MOUSE_MODE.into(),
        if now_on {
            ELispExp::symbol("t".into())
        } else {
            ELispExp::nil()
        },
    );
    ctx.set_echo_message(if now_on { "Mouse on" } else { "Mouse off" });
    Ok(if now_on {
        ELispExp::symbol("t".into())
    } else {
        ELispExp::nil()
    })
});

pub const MOUSE_DRAG_TO_DOC: &str = "(mouse-drag-to WINDOW LINE COLUMN): Extend the selection in \
         WINDOW to LINE and COLUMN. Returns t, or nil if there is no such window.\n\n\
         What dragging with the left button held runs. The first one of a drag sets the mark \
         where the button went down -- which is where point still is, because nothing has moved \
         it since -- and every one after it moves point, so the region grows from the click.\n\n\
         Whether the mark is already active is what tells the first from the rest, so no count \
         of events has to be kept anywhere.\n\n\
         Example:\n\
         (mouse-drag-to 0 12 8)";

primitive!(mouse_drag_to, args, _env, ctx, {
    let window = number_arg(args, 0)?.max(0.0) as usize;
    let line = number_arg(args, 1)?.max(0.0) as usize;
    let column = number_arg(args, 2)?.max(0.0) as usize;
    // Checked rather than selected. `select_window` restores the point that
    // window remembered, which during a drag is precisely the wrong thing: the
    // click already focused it and already put point where the button went
    // down, and that position is what the region is about to be anchored to.
    if WindowId(window) != ctx.get_focused_window_id() {
        return Ok(ELispExp::nil());
    }
    ctx.with_current_buffer_mut(|buf| {
        // The first event of the drag: point is still where the button went
        // down, so that is where the region starts.
        if !buf.mark.as_ref().is_some_and(|mark| mark.active) {
            buf.mark = Some(Mark::new(buf.text.cursor_pos_1d()));
        }
        let line = line.min(buf.text.line_count().saturating_sub(1));
        let column = column.min(edits::line_length(&buf.text, line));
        buf.text.cursor_move(line, column);
    });
    Ok(ELispExp::symbol("t".into()))
});

pub const MOUSE_RESIZE_DOC: &str = "(mouse-resize DELTA): Move the boundary being dragged by \
         DELTA cells, towards the second window when positive. Returns t if it moved.\n\n\
         Which boundary is not an argument: it was decided when the button went down, and is \
         held with the rest of the drag. A split has no name to pass -- it is a node that exists \
         to hold two others -- so the alternative would be inventing one.\n\n\
         The kind of division is kept. A strip opened at a fixed height and then dragged is \
         still a strip: it asked for a size rather than a share, and it still wants one.\n\n\
         Example:\n\
         (mouse-resize 1)";

primitive!(mouse_resize, args, _env, ctx, {
    let delta = number_arg(args, 0)? as isize;
    let Some(MouseDrag::Divider { path, .. }) = ctx.mouse_drag() else {
        return Ok(ELispExp::nil());
    };
    Ok(if ctx.resize_dragged_split(&path, delta) {
        ELispExp::symbol("t".into())
    } else {
        ELispExp::nil()
    })
});

/// Register this module's primitives: clicks, drags and the wheel.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // A command rather than a plain function, because it is one: the renderer
    // runs it when the terminal reports a bracketed paste, and it wants the
    // undo grouping and the `post-command-hook' that being a command brings.
    into.command("mouse-set-point", mouse_set_point, &[], MOUSE_SET_POINT_DOC);
    into.command(
        "mouse-start-selection",
        mouse_start_selection,
        &[],
        MOUSE_START_SELECTION_DOC,
    );
    into.command("mouse-drag-to", mouse_drag_to, &[], MOUSE_DRAG_TO_DOC);
    into.command("mouse-resize", mouse_resize, &[], MOUSE_RESIZE_DOC);
    into.command(
        "mouse-mode-toggle",
        mouse_mode_toggle,
        &[],
        MOUSE_MODE_TOGGLE_DOC,
    );
    into.command("mouse-scroll", mouse_scroll, &[], MOUSE_SCROLL_DOC);
}
