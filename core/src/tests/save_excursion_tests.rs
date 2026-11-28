//! `save-excursion`: point put back after its body, as a marker carries it
//! through the body's edits, in the buffer that was current.
//!
//! Design: `doc/architettura-editor.typ, <mod-commands>`.
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

    /// With `commands.lisp`, which is where `save-excursion` lives, and the
    /// current buffer holding "hello world", point at 5, after "hello".
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        eval_str(include_str!("../../lisp/base/commands.lisp"), &env, &ctx)
            .expect("loading commands.lisp");
        run(
            r#"(clear-buffer) (insert "hello world") (goto-char 5)"#,
            &env,
            &ctx,
        );
        (ctx, env)
    }

    fn number(n: f64) -> LispExp<Ctx> {
        LispExp::number(n)
    }

    fn string(s: &str) -> LispExp<Ctx> {
        LispExp::string(s.into())
    }

    #[test]
    fn point_comes_back_after_the_body_moves_it() {
        let (ctx, env) = editor();
        run(
            "(save-excursion (goto-char 0) (forward-char 2))",
            &env,
            &ctx,
        );
        assert_eq!(run("(point)", &env, &ctx), number(5.0));
    }

    #[test]
    fn point_follows_the_text_the_body_inserts_before_it() {
        let (ctx, env) = editor();
        run(
            r#"(save-excursion (goto-char 0) (insert ">> "))"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(buffer-string)", &env, &ctx), string(">> hello world"));
        assert_eq!(run("(point)", &env, &ctx), number(8.0));
    }

    #[test]
    fn point_follows_the_text_the_body_deletes_before_it() {
        let (ctx, env) = editor();
        run("(save-excursion (delete-region 0 2))", &env, &ctx);
        assert_eq!(run("(buffer-string)", &env, &ctx), string("llo world"));
        assert_eq!(run("(point)", &env, &ctx), number(3.0));
    }

    #[test]
    fn text_the_body_inserts_at_point_goes_after_it() {
        let (ctx, env) = editor();
        run(r#"(save-excursion (insert ","))"#, &env, &ctx);
        assert_eq!(run("(buffer-string)", &env, &ctx), string("hello, world"));
        assert_eq!(run("(point)", &env, &ctx), number(5.0));
    }

    #[test]
    fn point_comes_back_from_inside_a_replaced_span() {
        // `replace-regions' sends point to the start of a span it replaces;
        // the marker is moved there too, and comes back no further.
        let (ctx, env) = editor();
        run(
            r#"(save-excursion (replace-regions '((3 8 "LO WO"))))"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(buffer-string)", &env, &ctx), string("helLO WOrld"));
        assert_eq!(run("(point)", &env, &ctx), number(3.0));
    }

    #[test]
    fn save_excursion_answers_what_its_body_does() {
        let (ctx, env) = editor();
        assert_eq!(
            run("(save-excursion (goto-char 0) 42)", &env, &ctx),
            number(42.0)
        );
        assert!(run("(save-excursion)", &env, &ctx).is_nil());
    }

    #[test]
    fn point_comes_back_even_when_the_body_fails() {
        let (ctx, env) = editor();
        run(
            "(condition-case nil
                 (save-excursion (goto-char 0) (signal 'body-failed nil))
               (body-failed nil))",
            &env,
            &ctx,
        );
        assert_eq!(run("(point)", &env, &ctx), number(5.0));
    }

    #[test]
    fn the_error_the_body_raised_is_the_one_that_comes_out() {
        // Even from a buffer the body killed, where there is nothing to put
        // back and nothing on the way out to fail.
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "*doomed*") (switch-to-buffer "*doomed*")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run(
                r#"(condition-case data
                       (save-excursion
                         (kill-buffer-without-saving "*doomed*")
                         (signal 'body-failed '("the body's")))
                     (body-failed (cdr data)))"#,
                &env,
                &ctx,
            ),
            LispExp::proper_list(vec![string("the body's")])
        );
    }

    #[test]
    fn save_excursions_nest() {
        let (ctx, env) = editor();
        run(
            "(save-excursion
               (goto-char 2)
               (save-excursion (goto-char 0))
               (setq inner (point)))",
            &env,
            &ctx,
        );
        assert_eq!(run("inner", &env, &ctx), number(2.0));
        assert_eq!(run("(point)", &env, &ctx), number(5.0));
    }

    #[test]
    fn the_marker_is_let_go_on_the_way_out() {
        // The body sees the marker only by naming it, as here.
        let (ctx, env) = editor();
        run(
            "(save-excursion (setq kept save-excursion--marker))",
            &env,
            &ctx,
        );
        assert!(run("(marker-position kept)", &env, &ctx).is_nil());
        run(
            "(condition-case nil
                 (save-excursion
                   (setq kept save-excursion--marker)
                   (signal 'body-failed nil))
               (body-failed nil))",
            &env,
            &ctx,
        );
        assert!(run("(marker-position kept)", &env, &ctx).is_nil());
    }

    #[test]
    fn only_point_comes_back_and_not_the_buffer_the_body_switched_to() {
        let (ctx, env) = editor();
        run(
            r#"(setq home (current-buffer))
               (buffer-create "*elsewhere*")
               (save-excursion
                 (goto-char 0)
                 (switch-to-buffer "*elsewhere*")
                 (insert "there"))"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(current-buffer)", &env, &ctx), string("*elsewhere*"));
        assert_eq!(
            run("(with-current-buffer home (lambda () (point)))", &env, &ctx),
            number(5.0)
        );
    }

    #[test]
    fn a_body_that_kills_its_buffer_still_succeeds() {
        // Nothing to put back, and no error on the way out for it.
        let (ctx, env) = editor();
        assert_eq!(
            run(
                r#"(buffer-create "*doomed*")
                   (switch-to-buffer "*doomed*")
                   (insert "text")
                   (save-excursion (kill-buffer-without-saving "*doomed*") 7)"#,
                &env,
                &ctx,
            ),
            number(7.0)
        );
        assert!(run(r#"(member "*doomed*" (all-buffer-names))"#, &env, &ctx).is_nil());
    }

    #[test]
    fn a_buffer_made_again_under_the_same_name_is_left_alone() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "*again*")
               (switch-to-buffer "*again*")
               (insert "first")
               (save-excursion
                 (kill-buffer-without-saving "*again*")
                 (buffer-create "*again*")
                 (with-current-buffer "*again*"
                   (lambda () (insert "second") (goto-char 1))))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run(
                r#"(with-current-buffer "*again*" (lambda () (point)))"#,
                &env,
                &ctx
            ),
            number(1.0)
        );
    }
}