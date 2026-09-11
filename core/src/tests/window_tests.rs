//! Splitting the frame, closing windows, and moving between them.
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

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        (ctx, env)
    }

    fn windows(ctx: &Ctx) -> usize {
        ctx.window_count()
    }

    /// The tiled windows of a rendered frame, in layout order.
    fn tiled(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<crate::ui::RenderableWindowView> {
        ctx.snapshot(env, W, H)
            .views
            .into_iter()
            .filter(|v| !v.has_border)
            .collect()
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

    fn plain() -> KeyModifiers {
        KeyModifiers::default()
    }

    // ---------------- splitting ----------------

    #[test]
    fn a_split_makes_two_windows_showing_the_same_buffer() {
        let (ctx, env) = editor();
        assert_eq!(windows(&ctx), 1);

        eval_str("(split-window-below)", &env, &ctx).expect("split");
        assert_eq!(windows(&ctx), 2);

        let views = tiled(&ctx, &env);
        assert_eq!(views.len(), 2);
        assert_eq!(
            views[0].buffer_name, views[1].buffer_name,
            "a split shows one buffer twice, not a jump somewhere else"
        );
    }

    /// Focus stays where it was, as it does in Emacs: `C-x 2` then typing
    /// continues in the window you were already in.
    ///
    /// *Which* of the two that is matters, and is only observable through
    /// focus: both windows show the same buffer, so they are otherwise
    /// identical. The window you were in keeps the first half -- the top after
    /// `split-window-below`, the left after `split-window-right` -- and the
    /// new one goes after it, which is what makes the command names true.
    #[test]
    fn a_split_leaves_focus_in_the_first_half() {
        let (ctx, env) = editor();
        let before = ctx.get_focused_window_id();
        eval_str("(split-window-below)", &env, &ctx).expect("split");
        assert_eq!(ctx.get_focused_window_id(), before);

        let views = tiled(&ctx, &env);
        assert_eq!(
            views.iter().filter(|v| v.is_focused).count(),
            1,
            "exactly one window has focus"
        );
        assert!(
            views[0].is_focused,
            "the window that was split keeps the top half, and comes first in              layout order"
        );
        assert!(!views[1].is_focused, "the new window is the one below");

        let (ctx, env) = editor();
        eval_str("(split-window-right)", &env, &ctx).expect("split");
        let views = tiled(&ctx, &env);
        assert!(
            views[0].is_focused,
            "and the left half when splitting side by side"
        );
    }

    #[test]
    fn splitting_below_stacks_and_splitting_right_sits_side_by_side() {
        let (ctx, env) = editor();
        eval_str("(split-window-below)", &env, &ctx).expect("split");
        let views = tiled(&ctx, &env);
        assert_eq!(views[0].rect.x, views[1].rect.x, "same column");
        assert!(views[0].rect.y < views[1].rect.y, "one above the other");

        let (ctx, env) = editor();
        eval_str("(split-window-right)", &env, &ctx).expect("split");
        let views = tiled(&ctx, &env);
        assert_eq!(views[0].rect.y, views[1].rect.y, "same row");
        assert!(views[0].rect.x < views[1].rect.x, "side by side");
    }

    #[test]
    fn splits_nest() {
        let (ctx, env) = editor();
        eval_str(
            "(split-window-below) (split-window-right) (other-window) (split-window-below)",
            &env,
            &ctx,
        )
        .expect("three splits");
        assert_eq!(windows(&ctx), 4);
        assert_eq!(tiled(&ctx, &env).len(), 4);
    }

    /// The tree knows nothing about sizes, so there is no "too small to
    /// split" -- but a window with no rows must not bring the renderer down.
    #[test]
    fn splitting_past_the_point_of_visibility_does_not_panic() {
        let (ctx, env) = editor();
        for _ in 0..8 {
            eval_str("(split-window-below)", &env, &ctx).expect("split");
        }
        assert_eq!(windows(&ctx), 9);
        // Composing the frame is where a zero-height window would show up.
        let views = tiled(&ctx, &env);
        assert_eq!(views.len(), 9);
        assert!(views.iter().all(|v| v.lines.len() <= v.rect.height));
    }

    // ---------------- closing ----------------

    #[test]
    fn deleting_a_window_gives_its_space_to_the_other() {
        let (ctx, env) = editor();
        let full_height = tiled(&ctx, &env)[0].rect.height;

        eval_str("(split-window-below)", &env, &ctx).expect("split");
        assert!(tiled(&ctx, &env)[0].rect.height < full_height);

        eval_str("(delete-window)", &env, &ctx).expect("delete");
        assert_eq!(windows(&ctx), 1);
        assert_eq!(
            tiled(&ctx, &env)[0].rect.height,
            full_height,
            "the survivor grows back into the space"
        );
    }

    /// Focus was pointing at the window that just went, so it has to move
    /// before anything tries to draw a cursor in it.
    #[test]
    fn deleting_the_focused_window_moves_focus_to_a_survivor() {
        let (ctx, env) = editor();
        eval_str("(split-window-below)", &env, &ctx).expect("split");
        let gone = ctx.get_focused_window_id();

        eval_str("(delete-window)", &env, &ctx).expect("delete");
        assert_ne!(ctx.get_focused_window_id(), gone);
        assert_eq!(
            tiled(&ctx, &env).iter().filter(|v| v.is_focused).count(),
            1,
            "the surviving window has focus, so the cursor has somewhere to go"
        );
    }

    #[test]
    fn the_last_window_cannot_be_deleted() {
        let (ctx, env) = editor();
        assert!(
            eval_str("(delete-window)", &env, &ctx).is_err(),
            "a frame with no windows has nowhere to put the cursor"
        );
        assert_eq!(windows(&ctx), 1);
    }

    #[test]
    fn delete_other_windows_leaves_the_focused_one() {
        let (ctx, env) = editor();
        eval_str("(split-window-below) (split-window-right)", &env, &ctx).expect("splits");
        assert_eq!(windows(&ctx), 3);
        let kept = ctx.get_focused_window_id();

        eval_str("(delete-other-windows)", &env, &ctx).expect("delete others");
        assert_eq!(windows(&ctx), 1);
        assert_eq!(ctx.get_focused_window_id(), kept);
    }

    // ---------------- cycling ----------------

    #[test]
    fn other_window_walks_round_and_wraps() {
        let (ctx, env) = editor();
        eval_str("(split-window-below)", &env, &ctx).expect("split");
        let first = ctx.get_focused_window_id();

        eval_str("(other-window)", &env, &ctx).expect("next");
        let second = ctx.get_focused_window_id();
        assert_ne!(second, first);

        eval_str("(other-window)", &env, &ctx).expect("next");
        assert_eq!(
            ctx.get_focused_window_id(),
            first,
            "cycling wraps rather than stopping at the end"
        );
    }

    /// Cycling has to visit windows in the order they appear on screen, or
    /// `C-x o` jumps somewhere other than the next window along. Two windows
    /// cannot show this -- alternating looks the same in either direction --
    /// so this uses three.
    #[test]
    fn cycling_follows_the_order_windows_are_laid_out_in() {
        let (ctx, env) = editor();
        // Splitting twice from the same window gives three: the focused one,
        // the one just below it, and the one from the first split at the
        // bottom.
        eval_str("(split-window-below) (split-window-below)", &env, &ctx).expect("splits");
        assert_eq!(windows(&ctx), 3);

        let focused_index = |ctx: &Ctx, env: &Arc<Env<Ctx>>| {
            tiled(ctx, env)
                .iter()
                .position(|v| v.is_focused)
                .expect("some window has focus")
        };
        assert_eq!(focused_index(&ctx, &env), 0, "starting at the top");

        eval_str("(other-window)", &env, &ctx).expect("next");
        assert_eq!(
            focused_index(&ctx, &env),
            1,
            "the next window is the next one down the screen"
        );

        eval_str("(other-window)", &env, &ctx).expect("next");
        assert_eq!(focused_index(&ctx, &env), 2);

        eval_str("(other-window)", &env, &ctx).expect("next");
        assert_eq!(focused_index(&ctx, &env), 0, "and round again");
    }

    #[test]
    fn a_negative_count_walks_the_other_way() {
        let (ctx, env) = editor();
        eval_str("(split-window-below) (split-window-below)", &env, &ctx).expect("splits");
        assert_eq!(windows(&ctx), 3);

        let start = ctx.get_focused_window_id();
        eval_str("(other-window -1)", &env, &ctx).expect("back one");
        let back = ctx.get_focused_window_id();
        eval_str("(other-window 1)", &env, &ctx).expect("forward one");
        assert_eq!(
            ctx.get_focused_window_id(),
            start,
            "one back then one forward should return: {start} -> {back} -> ?"
        );
    }

    #[test]
    fn cycling_does_nothing_with_one_window() {
        let (ctx, env) = editor();
        assert_eq!(
            eval_str("(other-window)", &env, &ctx).expect("other-window"),
            LispExp::nil()
        );
    }

    /// `other-window` takes a `p` argument, so the prefix argument reaches it
    /// the way it reaches any counted command.
    #[test]
    fn a_prefix_argument_counts_windows_to_move() {
        let (ctx, env) = editor();
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");
        eval_str("(split-window-below) (split-window-below)", &env, &ctx).expect("splits");
        let start = ctx.get_focused_window_id();

        // C-u 2 C-x o
        press(&ctx, &env, KeyCode::Char('u'), ctrl());
        press(&ctx, &env, KeyCode::Char('2'), plain());
        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        press(&ctx, &env, KeyCode::Char('o'), plain());
        let two_on = ctx.get_focused_window_id();
        assert_ne!(two_on, start);

        // One more gets back to where we started, three windows round.
        eval_str("(other-window)", &env, &ctx).expect("one more");
        assert_eq!(ctx.get_focused_window_id(), start);
    }

    // ---------------- bindings ----------------

    #[test]
    fn the_window_keys_are_bound() {
        let (ctx, env) = editor();
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        press(&ctx, &env, KeyCode::Char('2'), plain());
        assert_eq!(windows(&ctx), 2, "C-x 2 should split");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        press(&ctx, &env, KeyCode::Char('3'), plain());
        assert_eq!(windows(&ctx), 3, "C-x 3 should split");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        press(&ctx, &env, KeyCode::Char('0'), plain());
        assert_eq!(windows(&ctx), 2, "C-x 0 should close this one");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        press(&ctx, &env, KeyCode::Char('1'), plain());
        assert_eq!(windows(&ctx), 1, "C-x 1 should close the others");
    }

    /// A digit after `C-x` belongs to the key sequence, not to a prefix
    /// argument -- which is exactly the case the two-pieces-of-state design
    /// exists to get right, and now has real bindings to get wrong.
    #[test]
    fn c_x_2_is_a_binding_not_a_prefix_argument() {
        let (ctx, env) = editor();
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        press(&ctx, &env, KeyCode::Char('2'), plain());
        assert_eq!(windows(&ctx), 2);
        assert_eq!(ctx.prefix_argument(), None, "the 2 was not a count");
    }

    // ---------------- the mode line, now that there is more than one ----------------

    /// With two windows only one has focus, so the two mode-line faces are
    /// finally both reachable.
    #[test]
    fn only_the_focused_window_is_marked_as_such() {
        let (ctx, env) = editor();
        eval_str("(split-window-below)", &env, &ctx).expect("split");

        let views = tiled(&ctx, &env);
        assert_eq!(views.iter().filter(|v| v.is_focused).count(), 1);
        assert!(
            views.iter().all(|v| v.mode_line.is_some()),
            "both windows carry a status line"
        );
    }
}
