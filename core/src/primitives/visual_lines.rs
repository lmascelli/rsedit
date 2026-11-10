//! Commands that move through the same visual rows the core composes.
use super::*;
use crate::primitives::args::movement_count;
use crate::ui::visual_lines::VisualLines;

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
    let continuing =
        ctx.last_command_is("next-visual-line") || ctx.last_command_is("previous-visual-line");
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

pub fn install<B: BufferTrait>(into: &Registry<B>) {
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
