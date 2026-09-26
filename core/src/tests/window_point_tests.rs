//! Two windows on one buffer: each keeps its own view, and its own point.
//!
//! # The two bugs these come from
//!
//! **The view.** `scroll_x`/`scroll_y` were already per-window, but every
//! window reconciled its scroll against the buffer's one point on every frame.
//! So typing in one window dragged the other's view along with it, and the
//! second window stopped being a second view of the file and became a copy of
//! the first.
//!
//! The obvious fix -- only the focused window follows -- breaks incremental
//! search, which moves point from a *floating* window while no tiled window has
//! focus. Then nothing follows, the match scrolls off the top, and `C-s' looks
//! like it has stopped finding anything. So the rule is "focused, or no tiled
//! window is focused", and the test for the second half of that is the one
//! worth having: it is the regression the obvious fix would have caused.
//!
//! **The point.** Fixing the view left the cursor shared, so switching windows
//! still yanked you back to wherever you had last been in the *other* one. Each
//! window now remembers where point was while its view was tracking it, and
//! focus puts it back. Nothing saves explicitly: composing a frame records it,
//! and a frame is composed after every command.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
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

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// An editor showing LINES numbered lines, split in two, with a frame
    /// composed so both windows know how tall they are.
    fn split_editor(lines: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        let text: String = (0..lines).map(|n| format!("line {n}\n")).collect();
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text.as_str());
            b.text.cursor_move(0, 0);
        });
        run("(split-window-below)", &env, &ctx);
        compose(&ctx, &env);
        (ctx, env)
    }

    /// Lay out a frame, which is what records each window's scroll and point.
    fn compose(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        let _ = ctx.snapshot(env, W, H);
    }

    /// The first line of text window INDEX is showing.
    fn top_line_of(ctx: &Ctx, env: &Arc<Env<Ctx>>, index: usize) -> String {
        ctx.snapshot(env, W, H).views[index]
            .lines
            .first()
            .cloned()
            .unwrap_or_default()
    }

    /// The line point is on, in the scratch buffer.
    fn point_line(ctx: &Ctx) -> usize {
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos().0)
            .expect("*scratch*")
    }

    /// Put point on the first character of LINE.
    fn goto_line(ctx: &Ctx, env: &Arc<Env<Ctx>>, line: usize) {
        let offset = ctx
            .with_buffer("*scratch*", |b| b.text.cursor_2d_to_1d(line, 0))
            .expect("*scratch*");
        run(&format!("(goto-char {offset})"), env, ctx);
    }

    /// Move point without going through the focused window, the way incremental
    /// search does when the minibuffer has the keyboard.
    fn move_point_from_elsewhere(ctx: &Ctx, line: usize) {
        ctx.with_buffer_mut("*scratch*", |b| b.text.cursor_move(line, 0));
    }

    // ----------------------------------------------------------------
    // The view
    // ----------------------------------------------------------------

    #[test]
    fn moving_point_in_one_window_leaves_the_others_view_alone() {
        // The reported bug, at its plainest.
        let (ctx, env) = split_editor(400);
        let others_top = top_line_of(&ctx, &env, 1);

        goto_line(&ctx, &env, 300);
        compose(&ctx, &env);

        assert_eq!(
            top_line_of(&ctx, &env, 1),
            others_top,
            "the window nobody is standing in should not have moved"
        );
    }

    #[test]
    fn the_focused_window_still_follows_point() {
        // The other half: the fix must not be "nothing follows".
        let (ctx, env) = split_editor(400);
        let mine_top = top_line_of(&ctx, &env, 0);

        goto_line(&ctx, &env, 300);
        compose(&ctx, &env);

        assert_ne!(
            top_line_of(&ctx, &env, 0),
            mine_top,
            "the window point is in has to bring it into view"
        );
        assert!(top_line_of(&ctx, &env, 0).contains("line 2"));
    }

    #[test]
    fn every_window_follows_point_while_a_float_has_focus() {
        // The regression that "only the focused window follows" would have
        // caused. Incremental search moves point while the minibuffer holds the
        // keyboard, so no tiled window is focused -- and if none followed, the
        // match would scroll off the top and `C-s' would look broken.
        let (ctx, env) = split_editor(400);
        run(r#"(minibuffer-read "P:" nil nil nil)"#, &env, &ctx);

        move_point_from_elsewhere(&ctx, 300);
        compose(&ctx, &env);

        for window in [0, 1] {
            assert!(
                top_line_of(&ctx, &env, window).contains("line 2"),
                "window {window} should have followed the match"
            );
        }
    }

    // ----------------------------------------------------------------
    // The point
    // ----------------------------------------------------------------

    #[test]
    fn each_window_keeps_its_own_point() {
        // The whole feature: be in two places in one file at once.
        let (ctx, env) = split_editor(400);
        goto_line(&ctx, &env, 5);
        compose(&ctx, &env);

        run("(other-window)", &env, &ctx);
        compose(&ctx, &env);
        goto_line(&ctx, &env, 200);
        compose(&ctx, &env);

        run("(other-window)", &env, &ctx);
        assert_eq!(point_line(&ctx), 5, "back where this window left off");

        run("(other-window)", &env, &ctx);
        assert_eq!(
            point_line(&ctx),
            200,
            "and the other one kept its place too"
        );
    }

    #[test]
    fn a_window_keeps_its_point_across_an_edit_in_the_other_one() {
        // Typing in one window must not drag the other's cursor, which is the
        // form the bug took once the *view* had been fixed.
        let (ctx, env) = split_editor(400);
        goto_line(&ctx, &env, 5);
        compose(&ctx, &env);
        run("(other-window)", &env, &ctx);
        compose(&ctx, &env);
        goto_line(&ctx, &env, 200);
        compose(&ctx, &env);
        run(r#"(self-insert "x")"#, &env, &ctx);
        compose(&ctx, &env);

        run("(other-window)", &env, &ctx);
        assert_eq!(point_line(&ctx), 5);
    }

    #[test]
    fn a_fresh_split_adopts_point_rather_than_resetting_it() {
        // A window that has never been composed answers `None`, and the right
        // thing for a new window is to take the point that is already there --
        // not to drag it to the top of the file.
        let (ctx, env) = split_editor(400);
        goto_line(&ctx, &env, 100);
        compose(&ctx, &env);
        run("(split-window-below)", &env, &ctx);

        run("(other-window)", &env, &ctx);
        assert_eq!(
            point_line(&ctx),
            100,
            "the new window starts where you were, not at offset zero"
        );
    }

    #[test]
    fn a_remembered_point_is_clamped_when_the_buffer_shrinks() {
        // Window points are stored offsets and nothing adjusts them for edits,
        // so one can outlive the text it described. Clamped rather than
        // refused: landing at the end of a buffer that got shorter is the only
        // answer that is always available.
        let (ctx, env) = split_editor(400);
        run("(other-window)", &env, &ctx);
        compose(&ctx, &env);
        goto_line(&ctx, &env, 350);
        compose(&ctx, &env);
        run("(other-window)", &env, &ctx);

        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from("one\ntwo\n");
            b.text.cursor_move(0, 0);
        });

        run("(other-window)", &env, &ctx);
        let (point, length) = {
            ctx.with_buffer("*scratch*", |buf| {
                (buf.text.cursor_pos_1d(), buf.text.len())
            })
            .expect("*scratch*")
        };
        assert_eq!(
            point, length,
            "a point past the end of a shrunken buffer lands at the end of it"
        );
    }

    #[test]
    fn a_floating_window_taking_focus_does_not_move_the_buffers_point() {
        // Only tiled windows remember a point. A prompt is not in the layout,
        // so focusing it restores nothing -- which is what lets incremental
        // search move point from inside one.
        let (ctx, env) = split_editor(400);
        goto_line(&ctx, &env, 50);
        compose(&ctx, &env);

        run(r#"(minibuffer-read "P:" nil nil nil)"#, &env, &ctx);

        assert_eq!(
            point_line(&ctx),
            50,
            "opening a prompt left the file's point where it was"
        );
    }
}
