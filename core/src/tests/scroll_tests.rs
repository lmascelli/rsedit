//! Moving the view rather than point: `C-v` and `M-v`.
//!
//! # Why a frame is rendered before each one
//!
//! A screenful is only a number once the frame has been laid out -- it depends
//! on the frame size, on how the windows are split, and on whether there is a
//! status line. `compute_tiled_views` works it out and leaves it on the window,
//! and scrolling reads it back. So a test that never renders is testing a
//! window that has never been on screen, which is a real case (it falls back to
//! one line) but not the one anybody cares about.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;
    /// The rows of text a lone window gets in an H-row frame: one goes to the
    /// echo area and one to the status line.
    const TEXT_ROWS: usize = H - 2;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// An editor showing LINES numbered lines, with a frame already composed so
    /// the window knows how tall it is.
    fn editor_showing(lines: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        let text: String = (0..lines).map(|n| format!("line {n}\n")).collect();
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| {
            b.text = GapBuffer::from(text.as_str());
            b.text.cursor_move(0, 0);
        });
        let _ = ctx.snapshot(&env, W, H);
        (ctx, env)
    }

    /// The first line of text the focused window is showing.
    fn top_line(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
        ctx.snapshot(env, W, H).views[0]
            .lines
            .first()
            .cloned()
            .unwrap_or_default()
    }

    fn point_line(ctx: &Ctx) -> usize {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .cursor_pos()
            .0
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode, modifiers: KeyModifiers) {
        ctx.handle_key_event(KeyEvent { code, modifiers }, env);
    }

    fn ctrl() -> KeyModifiers {
        KeyModifiers {
            ctrl: true,
            ..Default::default()
        }
    }

    fn alt() -> KeyModifiers {
        KeyModifiers {
            alt: true,
            ..Default::default()
        }
    }

    // ---------------- a screenful ----------------

    /// A screenful is the window less two lines, so consecutive pages overlap
    /// and the line you were reading is still there to find.
    #[test]
    fn scrolling_forward_moves_by_a_window_less_two_lines() {
        let (ctx, env) = editor_showing(200);
        assert_eq!(top_line(&ctx, &env), "line 0");

        run("(scroll-up-command)", &env, &ctx);

        assert_eq!(top_line(&ctx, &env), format!("line {}", TEXT_ROWS - 2));
    }

    #[test]
    fn scrolling_back_returns_to_where_it_started() {
        let (ctx, env) = editor_showing(200);
        run("(scroll-up-command)(scroll-up-command)", &env, &ctx);
        assert_ne!(top_line(&ctx, &env), "line 0");

        run("(scroll-down-command)(scroll-down-command)", &env, &ctx);

        assert_eq!(top_line(&ctx, &env), "line 0");
    }

    /// The overlap is the point of the two lines: whatever was on the last row
    /// is still on screen after the page turns.
    #[test]
    fn two_lines_are_shared_between_consecutive_pages() {
        let (ctx, env) = editor_showing(200);
        let before: Vec<String> = ctx.snapshot(&env, W, H).views[0].lines.clone();

        run("(scroll-up-command)", &env, &ctx);
        let after: Vec<String> = ctx.snapshot(&env, W, H).views[0].lines.clone();

        let shared: Vec<&String> = before.iter().filter(|line| after.contains(line)).collect();
        assert_eq!(
            shared.len(),
            2,
            "expected two lines in common, got {shared:?}"
        );
    }

    #[test]
    fn a_prefix_argument_means_that_many_screenfuls() {
        let (ctx, env) = editor_showing(400);
        run("(scroll-up-command 3)", &env, &ctx);
        let with_argument = top_line(&ctx, &env);

        let (ctx, env) = editor_showing(400);
        run(
            "(scroll-up-command)(scroll-up-command)(scroll-up-command)",
            &env,
            &ctx,
        );

        assert_eq!(top_line(&ctx, &env), with_argument);
    }

    // ---------------- what happens to point ----------------

    /// Scrolling moves the *view*. Point comes along only when the line it is
    /// on has scrolled out of sight -- which is what lets two pages of reading
    /// leave the cursor where the eye left it.
    #[test]
    fn point_stays_on_its_line_when_that_line_is_still_visible() {
        let (ctx, env) = editor_showing(200);
        // Down one page, then back: point was dragged to the top on the way
        // down, and on the way back its line is still on screen.
        run("(scroll-up-command)", &env, &ctx);
        let after_first = point_line(&ctx);

        run("(scroll-down-command)", &env, &ctx);

        assert_eq!(
            point_line(&ctx),
            after_first,
            "the line is still shown, so point had no reason to move"
        );
    }

    #[test]
    fn point_is_dragged_into_view_when_its_line_scrolls_away() {
        let (ctx, env) = editor_showing(200);
        assert_eq!(point_line(&ctx), 0);

        run("(scroll-up-command)", &env, &ctx);

        assert!(
            point_line(&ctx) >= TEXT_ROWS - 2,
            "point at line {} is above the new top",
            point_line(&ctx)
        );
    }

    /// Point keeps its column, so scrolling through a file and typing does not
    /// begin at the margin.
    #[test]
    fn point_keeps_its_column() {
        let (ctx, env) = editor_showing(200);
        run("(forward-char 4)", &env, &ctx);
        let column = ctx
            .get_buffer("*scratch*")
            .unwrap()
            .read()
            .unwrap()
            .text
            .cursor_pos()
            .1;
        assert_eq!(column, 4);

        run("(scroll-up-command)", &env, &ctx);

        let after = ctx
            .get_buffer("*scratch*")
            .unwrap()
            .read()
            .unwrap()
            .text
            .cursor_pos()
            .1;
        assert_eq!(after, 4);
    }

    // ---------------- the ends ----------------

    /// Scrolling stops with the last line on the bottom row. Further than that
    /// is a screen of nothing at all, which the user then has to scroll back
    /// out of.
    #[test]
    fn scrolling_forward_stops_with_the_last_line_in_view() {
        let (ctx, env) = editor_showing(60);

        for _ in 0..20 {
            run("(scroll-up-command)", &env, &ctx);
        }

        let shown = ctx.snapshot(&env, W, H).views[0].lines.clone();
        assert!(
            shown.iter().any(|line| line == "line 59"),
            "the last line should still be on screen: {shown:?}"
        );
        assert!(
            shown.iter().any(|line| !line.is_empty()),
            "the window should not be blank"
        );
    }

    /// A key that does nothing and says nothing is indistinguishable from one
    /// that is not bound, and the user's next move is to press it harder.
    #[test]
    fn reaching_the_end_says_so() {
        let (ctx, env) = editor_showing(30);
        for _ in 0..10 {
            run("(scroll-up-command)", &env, &ctx);
        }

        assert_eq!(ctx.get_echo_message(), "End of buffer");
        assert_eq!(
            run("(scroll-up-command)", &env, &ctx),
            LispExp::nil(),
            "and reports that it did nothing"
        );
    }

    #[test]
    fn reaching_the_beginning_says_so() {
        let (ctx, env) = editor_showing(30);
        run("(scroll-up-command)", &env, &ctx);

        for _ in 0..10 {
            run("(scroll-down-command)", &env, &ctx);
        }

        assert_eq!(top_line(&ctx, &env), "line 0");
        assert_eq!(ctx.get_echo_message(), "Beginning of buffer");
        assert_eq!(run("(scroll-down-command)", &env, &ctx), LispExp::nil());
    }

    /// A buffer that fits on screen has nowhere to scroll to, and saying so is
    /// better than moving the text under the reader for no reason.
    #[test]
    fn a_buffer_that_fits_does_not_scroll_at_all() {
        let (ctx, env) = editor_showing(3);

        assert_eq!(run("(scroll-up-command)", &env, &ctx), LispExp::nil());
        assert_eq!(top_line(&ctx, &env), "line 0");
        assert_eq!(ctx.get_echo_message(), "End of buffer");
    }

    /// A window that has never been on screen has no height to scroll by --
    /// the layout has not run, so nothing has worked one out. One line is a
    /// poor screenful and a great deal better than none: the key does
    /// something, says nothing about ends it has not reached, and the next
    /// frame gives the real answer.
    #[test]
    fn a_window_that_has_never_been_drawn_still_scrolls() {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| {
            b.text = GapBuffer::from(text.as_str());
            b.text.cursor_move(0, 0);
        });
        // Deliberately no snapshot: this is the state at startup, before the
        // first frame has been composed.

        assert_eq!(
            run("(scroll-up-command)", &env, &ctx),
            LispExp::t(),
            "it should move rather than report the end of a buffer it has \
             barely looked at"
        );
        assert_eq!(top_line(&ctx, &env), "line 1");
    }

    /// A window of one or two rows is all overlap: two pages would share
    /// every line, so a screenful floors at one line and the key still does
    /// something.
    #[test]
    fn a_window_too_short_to_hold_the_overlap_still_scrolls() {
        let (ctx, env) = editor_showing(200);
        // Three rows of frame: one for the echo area, one for the status line,
        // one of text.
        let _ = ctx.snapshot(&env, W, 3);

        assert_eq!(run("(scroll-up-command)", &env, &ctx), LispExp::t());
        assert_eq!(
            ctx.snapshot(&env, W, 3).views[0].lines.first().cloned(),
            Some("line 1".to_string())
        );
    }

    // ---------------- more than one window ----------------

    /// The screenful is *this* window's, not the frame's. Two windows split
    /// horizontally are half the height, so a page is half as far.
    #[test]
    fn a_split_window_scrolls_by_its_own_height() {
        let (ctx, env) = editor_showing(400);
        run("(scroll-up-command)", &env, &ctx);
        let whole_frame = top_line(&ctx, &env);

        let (ctx, env) = editor_showing(400);
        run("(split-window-below)", &env, &ctx);
        let _ = ctx.snapshot(&env, W, H);
        run("(scroll-up-command)", &env, &ctx);

        let split = top_line(&ctx, &env);
        assert_ne!(
            split, whole_frame,
            "a half-height window should not move a whole frame's worth"
        );
        let line_of = |text: &str| {
            text.trim()
                .strip_prefix("line ")
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or_else(|| panic!("{text:?}"))
        };
        assert!(line_of(&split) < line_of(&whole_frame));
    }

    /// Only the focused window moves: scrolling is about where *you* are
    /// looking.
    #[test]
    fn scrolling_leaves_the_other_window_where_it_was() {
        let (ctx, env) = editor_showing(400);
        run("(split-window-below)", &env, &ctx);
        let _ = ctx.snapshot(&env, W, H);
        let others_top = ctx.snapshot(&env, W, H).views[1].lines[0].clone();

        run("(scroll-up-command)", &env, &ctx);

        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.views[1].lines[0], others_top);
        assert_ne!(frame.views[0].lines[0], others_top, "but this one moved");
    }

    // ---------------- the keys ----------------

    #[test]
    fn c_v_and_m_v_are_bound() {
        let (ctx, env) = editor_showing(200);
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");

        press(&ctx, &env, KeyCode::Char('v'), ctrl());
        let after_forward = top_line(&ctx, &env);
        assert_ne!(after_forward, "line 0", "C-v should have moved the view");

        press(&ctx, &env, KeyCode::Char('v'), alt());
        assert_eq!(top_line(&ctx, &env), "line 0", "M-v should have moved back");
    }
}
