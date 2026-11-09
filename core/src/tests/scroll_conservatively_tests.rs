//! Following point when it leaves a window: a step past an edge scrolls just
//! enough, a jump puts point's line in the middle, as `scroll-conservatively`
//! says. The rule on its own, and then in an editor, where the frame is
//! composed between the moves as a terminal would draw it.
//!
//! Design: `doc/architettura-editor.typ, <primitive-finestre>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::ui::follow_point;
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;
    /// The rows of text a lone window gets in an H-row frame: one goes to the
    /// echo area and one to the status line.
    const TEXT_ROWS: usize = H - 2;

    // ---------------- the rule ----------------

    #[test]
    fn a_point_in_view_moves_nothing() {
        assert_eq!(follow_point(10, 10, 20, 1), 10);
        assert_eq!(follow_point(10, 29, 20, 1), 10);
        assert_eq!(follow_point(10, 15, 20, 0), 10);
    }

    #[test]
    fn a_step_past_an_edge_scrolls_just_enough() {
        // One below the last row: it becomes the last row.
        assert_eq!(follow_point(10, 30, 20, 1), 11);
        // One above the first: it becomes the first.
        assert_eq!(follow_point(10, 9, 20, 1), 9);
        // Three lines, with room for three.
        assert_eq!(follow_point(10, 32, 20, 3), 13);
    }

    #[test]
    fn a_jump_puts_point_in_the_middle() {
        // Rows 0..20, so 10 above point when the rows not point's are halved
        // and rounded, as `recenter` does.
        assert_eq!(follow_point(0, 60, 20, 1), 50);
        assert_eq!(follow_point(80, 60, 20, 1), 50);
        // Near the top there are not enough lines above: the first line.
        assert_eq!(follow_point(50, 3, 20, 1), 0);
    }

    #[test]
    fn zero_centres_even_a_step_as_emacs_does_by_default() {
        assert_eq!(follow_point(10, 30, 20, 0), 20);
    }

    #[test]
    fn above_a_hundred_never_centres() {
        assert_eq!(follow_point(0, 500, 20, 101), 481);
        assert_eq!(follow_point(500, 3, 20, 101), 3);
    }

    #[test]
    fn a_window_of_one_row_still_shows_point() {
        assert_eq!(follow_point(0, 7, 1, 1), 7);
        assert_eq!(follow_point(0, 7, 0, 1), 7);
    }

    // ---------------- in an editor ----------------

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// An editor showing LINES numbered lines, with a frame already composed
    /// so the window knows how tall it is.
    fn editor_showing(lines: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        let text: String = (0..lines).map(|n| format!("line {n}\n")).collect();
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text.as_str());
            b.text.cursor_move(0, 0);
        });
        let _ = ctx.snapshot(&env, W, H);
        (ctx, env)
    }

    /// The row point's line is drawn on, from the window's top, once a frame
    /// has been composed.
    fn point_row(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> usize {
        let frame = ctx.snapshot(env, W, H);
        frame.views[0]
            .cursor_rel_pos
            .expect("the focused window draws the cursor")
            .1
    }

    /// The row a centred line is drawn on in the window, as `recenter` puts it.
    const MIDDLE: usize = TEXT_ROWS / 2;

    #[test]
    fn goto_line_far_below_centres_the_line() {
        let (ctx, env) = editor_showing(200);
        run("(goto-line 100)", &env, &ctx);
        let row = point_row(&ctx, &env);
        assert!(
            row.abs_diff(MIDDLE) <= 1,
            "line 100 on row {row}, not in the middle of {TEXT_ROWS} rows"
        );
    }

    #[test]
    fn goto_line_far_above_centres_the_line() {
        let (ctx, env) = editor_showing(200);
        run("(goto-line 150)", &env, &ctx);
        let _ = ctx.snapshot(&env, W, H);
        run("(goto-line 60)", &env, &ctx);
        let row = point_row(&ctx, &env);
        assert!(row.abs_diff(MIDDLE) <= 1, "line 60 on row {row}");
    }

    #[test]
    fn next_line_on_the_last_row_scrolls_one_line() {
        let (ctx, env) = editor_showing(200);
        // Down to the last row, one line at a time, a frame after each.
        for _ in 0..TEXT_ROWS - 1 {
            run("(next-line)", &env, &ctx);
            let _ = ctx.snapshot(&env, W, H);
        }
        assert_eq!(point_row(&ctx, &env), TEXT_ROWS - 1, "on the last row");
        run("(next-line)", &env, &ctx);
        assert_eq!(
            point_row(&ctx, &env),
            TEXT_ROWS - 1,
            "one more line scrolls by one, and point stays on the last row"
        );
    }

    #[test]
    fn with_zero_a_step_past_the_bottom_centres() {
        let (ctx, env) = editor_showing(200);
        run("(setq scroll-conservatively 0)", &env, &ctx);
        for _ in 0..TEXT_ROWS {
            run("(next-line)", &env, &ctx);
            let _ = ctx.snapshot(&env, W, H);
        }
        let row = point_row(&ctx, &env);
        assert!(row.abs_diff(MIDDLE) <= 1, "row {row}");
    }

    #[test]
    fn above_a_hundred_a_jump_lands_on_the_last_row() {
        let (ctx, env) = editor_showing(200);
        run("(setq scroll-conservatively 101)", &env, &ctx);
        run("(goto-line 100)", &env, &ctx);
        assert_eq!(point_row(&ctx, &env), TEXT_ROWS - 1);
    }

    #[test]
    fn a_nonsense_value_falls_back_to_the_default() {
        for setting in ["nil", "\"far\"", "-3"] {
            let (ctx, env) = editor_showing(200);
            run(
                &format!("(setq scroll-conservatively {setting})"),
                &env,
                &ctx,
            );
            run("(goto-line 100)", &env, &ctx);
            let row = point_row(&ctx, &env);
            assert!(row.abs_diff(MIDDLE) <= 1, "row {row} after {setting}");
        }
    }

    #[test]
    fn a_window_made_smaller_scrolls_just_enough() {
        // Point on the last row, and the frame loses three rows: the text
        // moves only as far as it has to, since the cursor did not move.
        let (ctx, env) = editor_showing(200);
        for _ in 0..TEXT_ROWS - 1 {
            run("(next-line)", &env, &ctx);
            let _ = ctx.snapshot(&env, W, H);
        }
        let frame = ctx.snapshot(&env, W, H - 3);
        let row = frame.views[0].cursor_rel_pos.expect("cursor").1;
        assert_eq!(row, TEXT_ROWS - 4, "on the last row of the smaller window");
    }

    #[test]
    fn the_setting_starts_at_its_default() {
        let (ctx, env) = editor_showing(1);
        assert_eq!(
            run("scroll-conservatively", &env, &ctx),
            LispExp::number(crate::editor::DEFAULT_SCROLL_CONSERVATIVELY)
        );
    }
}
