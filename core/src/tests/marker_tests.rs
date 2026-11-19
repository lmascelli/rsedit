//! Markers from Lisp: a position that the text carries along, with the side
//! of text inserted exactly at it that it ends on.
//!
//! Design: `doc/architettura-editor.typ, <mark-overlay>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
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

    /// An editor whose *scratch* holds "hello world", point at its end.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        run(r#"(clear-buffer) (insert "hello world")"#, &env, &ctx);
        (ctx, env)
    }

    fn number(n: f64) -> LispExp<Ctx> {
        LispExp::number(n)
    }

    #[test]
    fn a_marker_follows_the_text_inserted_before_it() {
        let (ctx, env) = editor();
        run(
            "(setq m (make-marker 6)) (goto-char 0) (insert \">> \")",
            &env,
            &ctx,
        );
        assert_eq!(run("(marker-position m)", &env, &ctx), number(9.0));
    }

    #[test]
    fn text_inserted_at_a_marker_goes_after_it_unless_it_advances() {
        let (ctx, env) = editor();
        run(
            "(setq stays (make-marker 5)) (setq goes (make-marker 5 t))
             (goto-char 5) (insert \",\")",
            &env,
            &ctx,
        );
        assert_eq!(run("(marker-position stays)", &env, &ctx), number(5.0));
        assert_eq!(run("(marker-position goes)", &env, &ctx), number(6.0));
    }

    #[test]
    fn a_marker_inside_deleted_text_lands_at_its_start() {
        let (ctx, env) = editor();
        run("(setq m (make-marker 8)) (delete-region 5 11)", &env, &ctx);
        assert_eq!(run("(marker-position m)", &env, &ctx), number(5.0));
    }

    #[test]
    fn a_marker_is_kept_within_the_buffer_and_moved_by_set_marker() {
        let (ctx, env) = editor();
        run("(setq m (make-marker 500))", &env, &ctx);
        assert_eq!(run("(marker-position m)", &env, &ctx), number(11.0));
        assert_eq!(run("(set-marker m 2)", &env, &ctx), number(2.0));
        assert_eq!(run("(marker-position m)", &env, &ctx), number(2.0));
    }

    #[test]
    fn a_deleted_marker_is_gone_and_another_buffer_has_none() {
        let (ctx, env) = editor();
        run("(setq m (make-marker 3))", &env, &ctx);
        run(r#"(buffer-create "other")"#, &env, &ctx);
        assert!(
            run(
                r#"(with-current-buffer "other" (lambda () (marker-position m)))"#,
                &env,
                &ctx
            )
            .is_nil()
        );
        assert!(run("(delete-marker m)", &env, &ctx).is_truthy());
        assert!(run("(delete-marker m)", &env, &ctx).is_nil());
        assert!(run("(marker-position m)", &env, &ctx).is_nil());
        assert!(run("(set-marker m 1)", &env, &ctx).is_nil());
    }

    #[test]
    fn undoing_an_insertion_brings_a_marker_back() {
        let (ctx, env) = editor();
        run(
            "(setq m (make-marker 6)) (undo-boundary) (goto-char 0) (insert \"abc\") (undo)",
            &env,
            &ctx,
        );
        assert_eq!(run("(marker-position m)", &env, &ctx), number(6.0));
    }

    #[test]
    fn a_handle_that_is_not_a_number_is_refused() {
        let (ctx, env) = editor();
        assert!(eval_str("(marker-position \"m\")", &env, &ctx).is_err());
        assert!(eval_str("(make-marker -1)", &env, &ctx).is_err());
    }
}
