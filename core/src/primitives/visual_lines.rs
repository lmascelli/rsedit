//! Commands that move through the same visual rows the core composes, and
//! the ones the ordinary keys run, which move by visual rows where the buffer
//! wraps and by lines elsewhere.
use super::*;
use crate::primitives::args::movement_count;
use crate::ui::visual_lines::{MODE, VisualLines};

/// The commands the ordinary vertical keys run: Up, Down, `C-p`, `C-n`.
const ORDINARY_VERTICAL: [&str; 2] = ["move-next-line", "move-previous-line"];

/// Returns whether the current buffer wraps, `visual-line-mode` on: what
/// the ordinary keys' commands ask to choose how to move.
pub(super) fn wrapping<B: BufferTrait>(ctx: &EditorState<B>) -> bool {
    ctx.with_current_buffer(|buf| buf.minor_mode_enabled(MODE))
}

/// Returns whether the command before was one of the ordinary vertical
/// keys': a run of them keeps its goal column, whichever way they moved.
pub(super) fn after_ordinary_vertical_move<B: BufferTrait>(ctx: &EditorState<B>) -> bool {
    ORDINARY_VERTICAL
        .iter()
        .any(|name| ctx.last_command_is(name))
}

fn dimensions<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
) -> (usize, usize) {
    let width = ctx.windows(|windows| {
        windows
            .root()
            .window(windows.focused())
            .map(|win| win.rect.width.saturating_sub(win.gutter_width))
            .unwrap_or(0)
    });
    let width = if width == 0 {
        match env.get_variable("frame-width") {
            Some(ELispExp::Number(n)) => (n as usize).max(1),
            _ => 80,
        }
    } else {
        width
    };
    let mode = ctx.with_current_buffer(|buf| buf.current_mode.clone());
    let tabs = crate::editor::tab_widths(env, [mode.clone()]).of(&mode);
    (width, tabs)
}

fn move_visual<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    delta: isize,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let (width, tab_width) = dimensions(ctx, env);
    // The ordinary keys' commands count where the buffer wraps: there they
    // moved by visual rows, with a goal column of the screen.
    let continuing = ctx.last_command_is("next-visual-line")
        || ctx.last_command_is("previous-visual-line")
        || (after_ordinary_vertical_move(ctx) && wrapping(ctx));
    let remembered = ctx.goal_column();
    let goal = ctx.with_current_buffer_mut(|buf| {
        let visual = VisualLines::new(&buf.text, &buf.virtual_text, width, tab_width);
        let (line, column) = buf.text.cursor_pos();
        let (here, screen) = visual.position(line, column);
        let goal = if continuing {
            remembered.unwrap_or(screen)
        } else {
            screen
        };
        let mut target = visual.advance(here, delta);
        let destination = loop {
            if target == here {
                break None;
            }
            let rows = visual.line(target.line);
            let Some(row) = rows.get(target.row) else {
                break None;
            };
            let column = row.to_buffer(goal);
            let (landed, _) = visual.position(target.line, column);
            if (delta > 0 && landed > here) || (delta < 0 && landed < here) {
                break Some((target.line, column));
            }
            // A continuation containing only a hint or part of a Tab has
            // no buffer position of its own. Cross it rather than getting
            // stuck on the same character when moving back up through it.
            let next = visual.advance(target, delta.signum());
            if next == target {
                break None;
            }
            target = next;
        };
        if let Some((line, column)) = destination {
            buf.text.cursor_move(line, column);
        }
        goal
    });
    ctx.set_goal_column(Some(goal));
    Ok(ELispExp::nil())
}

pub const NEXT_VISUAL_LINE_DOC: &str = "(next-visual-line &optional N): Move down N visual rows, \
    one by default, preserving the screen column across consecutive vertical moves. \
    A negative N moves up. Rows wrap at the focused window's text width.\n\n\
    Example:\n\
    (next-visual-line 3)";
primitive!(next_visual_line, args, env, ctx, {
    move_visual(ctx, &env, movement_count(args)?)
});

pub const PREVIOUS_VISUAL_LINE_DOC: &str = "(previous-visual-line &optional N): Move up N visual \
    rows, one by default, preserving the screen column. A negative N moves down.\n\n\
    Example:\n\
    (previous-visual-line 2)";
primitive!(previous_visual_line, args, env, ctx, {
    move_visual(ctx, &env, movement_count(args)?.saturating_neg())
});

fn edge<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    end: bool,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let (width, tab_width) = dimensions(ctx, env);
    ctx.with_current_buffer_mut(|buf| {
        let visual = VisualLines::new(&buf.text, &buf.virtual_text, width, tab_width);
        let (line, column) = buf.text.cursor_pos();
        let (here, _) = visual.position(line, column);
        if let Some(row) = visual.line(line).get(here.row) {
            let target = if end {
                row.last_column()
            } else {
                row.first_column()
            };
            buf.text.cursor_move(line, target);
        }
        Ok(ELispExp::nil())
    })
}

pub const BEGINNING_OF_VISUAL_LINE_DOC: &str = "(beginning-of-visual-line): Move point to the \
    first buffer position on the current visual row.\n\n\
    Example:\n\
    (beginning-of-visual-line)";
primitive!(beginning_of_visual_line, args, env, ctx, {
    exact_arity(args, 0)?;
    edge(ctx, &env, false)
});

pub const END_OF_VISUAL_LINE_DOC: &str = "(end-of-visual-line): Move point to the last buffer \
    position displayed on the current visual row, or to the logical line's end on its final \
    row. No text is changed.\n\n\
    Example:\n\
    (end-of-visual-line)";
primitive!(end_of_visual_line, args, env, ctx, {
    exact_arity(args, 0)?;
    edge(ctx, &env, true)
});

pub const MOVE_NEXT_LINE_DOC: &str = "(move-next-line &optional N): Move down N lines, one by \
    default, as Down and C-n do: by visual rows, as `next-visual-line', where \
    `visual-line-mode' is on in the current buffer, and by lines, as `next-line', elsewhere. \
    A negative N moves up. `next-line' itself always moves by lines.\n\n\
    Example:\n\
    (move-next-line 2)";
primitive!(move_next_line, args, env, ctx, {
    let count = movement_count(args)?;
    if wrapping(ctx) {
        move_visual(ctx, &env, count)
    } else {
        super::movement::move_line(ctx, count)
    }
});

pub const MOVE_PREVIOUS_LINE_DOC: &str = "(move-previous-line &optional N): Move up N lines, one \
    by default, as Up and C-p do: by visual rows, as `previous-visual-line', where \
    `visual-line-mode' is on in the current buffer, and by lines, as `previous-line', \
    elsewhere. A negative N moves down.\n\n\
    Example:\n\
    (move-previous-line)";
primitive!(move_previous_line, args, env, ctx, {
    let count = movement_count(args)?.saturating_neg();
    if wrapping(ctx) {
        move_visual(ctx, &env, count)
    } else {
        super::movement::move_line(ctx, count)
    }
});

pub const MOVE_BEGINNING_OF_LINE_DOC: &str = "(move-beginning-of-line): Move point to the start \
    of the line, as Home and C-a do: of the visual row, as `beginning-of-visual-line', where \
    `visual-line-mode' is on in the current buffer, and of the line, as `beginning-of-line', \
    elsewhere.\n\n\
    Example:\n\
    (move-beginning-of-line)";
primitive!(move_beginning_of_line, args, env, ctx, {
    exact_arity(args, 0)?;
    if wrapping(ctx) {
        edge(ctx, &env, false)
    } else {
        super::movement::line_edge(ctx, false)
    }
});

pub const MOVE_END_OF_LINE_DOC: &str = "(move-end-of-line): Move point to the end of the line, as \
    End and C-e do: of the visual row, as `end-of-visual-line', where `visual-line-mode' is \
    on in the current buffer, and of the line, as `end-of-line', elsewhere.\n\n\
    Example:\n\
    (move-end-of-line)";
primitive!(move_end_of_line, args, env, ctx, {
    exact_arity(args, 0)?;
    if wrapping(ctx) {
        edge(ctx, &env, true)
    } else {
        super::movement::line_edge(ctx, true)
    }
});

pub fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("move-next-line", move_next_line, &["p"], MOVE_NEXT_LINE_DOC);
    into.command(
        "move-previous-line",
        move_previous_line,
        &["p"],
        MOVE_PREVIOUS_LINE_DOC,
    );
    into.command(
        "move-beginning-of-line",
        move_beginning_of_line,
        &[],
        MOVE_BEGINNING_OF_LINE_DOC,
    );
    into.command(
        "move-end-of-line",
        move_end_of_line,
        &[],
        MOVE_END_OF_LINE_DOC,
    );
    into.command(
        "next-visual-line",
        next_visual_line,
        &["p"],
        NEXT_VISUAL_LINE_DOC,
    );
    into.command(
        "previous-visual-line",
        previous_visual_line,
        &["p"],
        PREVIOUS_VISUAL_LINE_DOC,
    );
    into.command(
        "beginning-of-visual-line",
        beginning_of_visual_line,
        &[],
        BEGINNING_OF_VISUAL_LINE_DOC,
    );
    into.command(
        "end-of-visual-line",
        end_of_visual_line,
        &[],
        END_OF_VISUAL_LINE_DOC,
    );
}
