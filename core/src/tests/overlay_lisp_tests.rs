//! Overlays as Lisp reaches them, and as the renderer sees them.
//!
//! `overlay_tests` covers the adjustment rules against the table directly.
//! This is the other two ends: the primitives, and the fact that an overlay
//! actually becomes a `Highlight` in the right place, over the mode's
//! colouring and under the region.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::ui::{Face, Highlight};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    /// The highlights the focused window would be drawn with.
    fn highlights(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<Highlight> {
        ctx.snapshot(env, 80, 24)
            .views
            .iter()
            .find(|view| view.is_focused)
            .expect("a focused window")
            .highlights
            .clone()
    }

    // ----------------------------------------------------------------
    // The primitives
    // ----------------------------------------------------------------

    #[test]
    fn an_overlay_can_be_made_and_found_again() {
        let (ctx, env) = editor();
        run(r#"(insert "hello world")"#, &env, &ctx);
        let id = run(r#"(make-overlay 6 11 'error)"#, &env, &ctx);
        assert!(!id.is_nil(), "a handle comes back");
        let found = run("(overlays-at 7)", &env, &ctx);
        assert_eq!(found.iter().count(), 1);
    }

    #[test]
    fn an_overlay_covers_its_start_and_not_its_end() {
        let (ctx, env) = editor();
        run(r#"(insert "hello world")"#, &env, &ctx);
        run(r#"(make-overlay 6 11 'error)"#, &env, &ctx);
        assert_eq!(run("(overlays-at 6)", &env, &ctx).iter().count(), 1);
        assert_eq!(run("(overlays-at 10)", &env, &ctx).iter().count(), 1);
        assert_eq!(run("(overlays-at 11)", &env, &ctx).iter().count(), 0);
        assert_eq!(run("(overlays-at 5)", &env, &ctx).iter().count(), 0);
    }

    #[test]
    fn an_empty_overlay_is_refused() {
        let (ctx, env) = editor();
        run(r#"(insert "hello")"#, &env, &ctx);
        assert!(run("(make-overlay 2 2 'error)", &env, &ctx).is_nil());
    }

    #[test]
    fn the_face_is_remembered() {
        let (ctx, env) = editor();
        run(r#"(insert "hello world")"#, &env, &ctx);
        let id = run(r#"(make-overlay 0 5 'keyword)"#, &env, &ctx);
        let LispExp::Number(id) = id else {
            panic!("expected a handle")
        };
        assert_eq!(
            run(&format!("(overlay-face {id})"), &env, &ctx),
            LispExp::symbol("keyword".to_string())
        );
    }

    #[test]
    fn an_overlay_can_be_deleted_by_handle() {
        let (ctx, env) = editor();
        run(r#"(insert "hello world")"#, &env, &ctx);
        let LispExp::Number(id) = run(r#"(make-overlay 0 5 'error)"#, &env, &ctx) else {
            panic!("expected a handle")
        };
        assert!(!run(&format!("(delete-overlay {id})"), &env, &ctx).is_nil());
        assert_eq!(run("(overlays-at 1)", &env, &ctx).iter().count(), 0);
        // And deleting it again is not a success.
        assert!(run(&format!("(delete-overlay {id})"), &env, &ctx).is_nil());
    }

    #[test]
    fn a_whole_category_can_be_replaced() {
        // What a producer wants: re-running a search forgets its old matches
        // without having kept every handle.
        let (ctx, env) = editor();
        run(r#"(insert "one two three four")"#, &env, &ctx);
        run(
            r#"(make-overlay 0 3 'error 0 'isearch)
               (make-overlay 4 7 'error 0 'isearch)
               (make-overlay 8 13 'error 0 'diagnostics)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(remove-overlays 'isearch)", &env, &ctx),
            LispExp::number(2.0)
        );
        assert_eq!(run("(overlays-at 1)", &env, &ctx).iter().count(), 0);
        assert_eq!(run("(overlays-at 9)", &env, &ctx).iter().count(), 1);
    }

    #[test]
    fn removing_with_no_category_removes_everything() {
        let (ctx, env) = editor();
        run(r#"(insert "one two three")"#, &env, &ctx);
        run(
            r#"(make-overlay 0 3 'error 0 'a) (make-overlay 4 7 'error 0 'b)"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(remove-overlays)", &env, &ctx), LispExp::number(2.0));
    }

    #[test]
    fn a_span_past_the_end_is_clamped_rather_than_refused() {
        // A caller working from stale offsets marks something wrong rather
        // than marking past the end, where there is no text to draw over.
        let (ctx, env) = editor();
        run(r#"(insert "short")"#, &env, &ctx);
        assert!(!run("(make-overlay 2 900 'error)", &env, &ctx).is_nil());
        assert_eq!(run("(overlays-at 4)", &env, &ctx).iter().count(), 1);
    }

    #[test]
    fn overlays_at_answers_lowest_priority_first() {
        let (ctx, env) = editor();
        run(r#"(insert "hello world")"#, &env, &ctx);
        let LispExp::Number(high) = run("(make-overlay 0 5 'error 9 'high)", &env, &ctx) else {
            panic!()
        };
        let LispExp::Number(low) = run("(make-overlay 0 5 'keyword 1 'low)", &env, &ctx) else {
            panic!()
        };
        let found: Vec<f64> = run("(overlays-at 2)", &env, &ctx)
            .iter()
            .map(|item| match item {
                LispExp::Number(n) => n,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(found, vec![low, high], "drawn low first, high on top");
    }

    // ----------------------------------------------------------------
    // Surviving edits, through Lisp
    // ----------------------------------------------------------------

    #[test]
    fn typing_in_front_carries_the_overlay_along() {
        let (ctx, env) = editor();
        run(r#"(insert "hello world") (make-overlay 6 11 'error)"#, &env, &ctx);
        run(r#"(goto-char 0) (insert ">> ")"#, &env, &ctx);
        // Still on "world", which now begins at 9.
        assert_eq!(run("(overlays-at 9)", &env, &ctx).iter().count(), 1);
        assert_eq!(run("(overlays-at 6)", &env, &ctx).iter().count(), 0);
    }

    #[test]
    fn deleting_the_marked_text_removes_the_overlay() {
        let (ctx, env) = editor();
        run(r#"(insert "hello world") (make-overlay 6 11 'error)"#, &env, &ctx);
        run(r#"(goto-char 6) (set-mark) (goto-char 11) (kill-region)"#, &env, &ctx);
        assert_eq!(run("(overlays-at 6)", &env, &ctx).iter().count(), 0);
    }

    // ----------------------------------------------------------------
    // Reaching the screen
    // ----------------------------------------------------------------

    #[test]
    fn an_overlay_becomes_a_highlight_where_the_text_is() {
        let (ctx, env) = editor();
        run(r#"(insert "hello world") (make-overlay 6 11 'error)"#, &env, &ctx);
        let drawn = highlights(&ctx, &env);
        let found = drawn
            .iter()
            .find(|h| h.face == Face::intern("error"))
            .expect("the overlay should be drawn");
        assert_eq!((found.row, found.start_col, found.end_col), (0, 6, 11));
    }

    #[test]
    fn an_overlay_off_screen_is_not_drawn() {
        let (ctx, env) = editor();
        let lines = "line\n".repeat(200);
        run(&format!(r#"(insert "{lines}")"#), &env, &ctx);
        // Well below the 24-row window, which is showing the top.
        run(r#"(goto-char 0) (make-overlay 600 610 'error)"#, &env, &ctx);
        let drawn = highlights(&ctx, &env);
        assert!(
            !drawn.iter().any(|h| h.face == Face::intern("error")),
            "nothing off screen should be drawn"
        );
    }

    #[test]
    fn a_multi_line_overlay_is_drawn_on_each_of_its_rows() {
        let (ctx, env) = editor();
        run(r#"(insert "one\ntwo\nthree") (make-overlay 0 13 'error)"#, &env, &ctx);
        let rows: Vec<usize> = highlights(&ctx, &env)
            .iter()
            .filter(|h| h.face == Face::intern("error"))
            .map(|h| h.row)
            .collect();
        assert_eq!(rows, vec![0, 1, 2]);
    }

    #[test]
    fn the_region_is_drawn_over_an_overlay() {
        // The order rule: syntax, then overlays, then the region. Later
        // entries are drawn over earlier ones.
        let (ctx, env) = editor();
        run(
            r#"(insert "hello world")
               (make-overlay 0 11 'error)
               (goto-char 0) (set-mark) (goto-char 5)"#,
            &env,
            &ctx,
        );
        let drawn = highlights(&ctx, &env);
        let overlay_at = drawn
            .iter()
            .position(|h| h.face == Face::intern("error"))
            .expect("the overlay");
        let region_at = drawn
            .iter()
            .position(|h| h.face == Face::REGION)
            .expect("the region");
        assert!(
            region_at > overlay_at,
            "the selection must be drawn after, and so over, the overlay"
        );
    }

    #[test]
    fn a_buffer_with_no_overlays_produces_no_overlay_highlights() {
        let (ctx, env) = editor();
        run(r#"(insert "hello world")"#, &env, &ctx);
        assert!(highlights(&ctx, &env).is_empty());
    }
}
