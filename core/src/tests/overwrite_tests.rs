//! `overwrite-mode`: a character typed takes the place of the one after
//! point, up to the end of the line, in the buffers where the mode is on.
//!
//! Design: `doc/architettura-editor.typ, <mod-overwrite>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent};
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

    /// An editor showing TEXT in *scratch*, with point at POINT and
    /// `overwrite-mode` on there, from its module.
    fn editor_with(text: &str, point: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/overwrite.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(point);
            b.text.cursor_move(line, col);
        });
        run("(overwrite-mode 1)", &env, &ctx);
        (ctx, env)
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|b| b.text.to_string())
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_current_buffer(|b| b.text.cursor_pos_1d())
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode) {
        ctx.handle_key_event(KeyEvent::new(code), env);
    }

    #[test]
    fn a_character_typed_takes_the_place_of_the_one_after_point() {
        let (ctx, env) = editor_with("abc", 1);
        run(r#"(self-insert "x")"#, &env, &ctx);
        assert_eq!(text(&ctx), "axc");
        assert_eq!(point(&ctx), 2);
    }

    #[test]
    fn at_the_end_of_a_line_it_is_inserted_and_the_newline_stays() {
        let (ctx, env) = editor_with("ab\ncd", 2);
        run(r#"(self-insert "x")"#, &env, &ctx);
        assert_eq!(text(&ctx), "abx\ncd");
        let (ctx, env) = editor_with("ab", 2);
        run(r#"(self-insert "x")"#, &env, &ctx);
        assert_eq!(text(&ctx), "abx", "and at the end of the buffer");
    }

    #[test]
    fn a_count_replaces_as_many_and_inserts_what_does_not_fit_on_the_line() {
        let (ctx, env) = editor_with("abcd\nef", 1);
        run(r#"(self-insert "x" 2)"#, &env, &ctx);
        assert_eq!(text(&ctx), "axxd\nef");
        run(r#"(self-insert "y" 3)"#, &env, &ctx);
        assert_eq!(text(&ctx), "axxyyy\nef");
        assert_eq!(point(&ctx), 6);
    }

    #[test]
    fn with_the_mode_off_a_character_is_inserted() {
        let (ctx, env) = editor_with("abc", 1);
        run("(overwrite-mode -1)", &env, &ctx);
        run(r#"(self-insert "x")"#, &env, &ctx);
        assert_eq!(text(&ctx), "axbc");
    }

    #[test]
    fn the_mode_is_one_buffers() {
        let (ctx, env) = editor_with("abc", 0);
        run(
            r#"(buffer-create "other") (switch-to-buffer "other")
               (insert "abc") (goto-char 0) (self-insert "x")"#,
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "xabc");
    }

    #[test]
    fn what_insert_and_a_paste_put_in_is_inserted() {
        let (ctx, env) = editor_with("abc", 1);
        run(r#"(insert "x") (insert-pasted-text "y")"#, &env, &ctx);
        assert_eq!(text(&ctx), "axybc");
    }

    #[test]
    fn a_character_typed_over_another_undoes_in_one_step() {
        let (ctx, env) = editor_with("abc", 1);
        press(&ctx, &env, KeyCode::Char('x'));
        assert_eq!(text(&ctx), "axc");
        run("(undo)", &env, &ctx);
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn a_read_only_buffer_refuses_it_and_loses_nothing() {
        let (ctx, env) = editor_with("abc", 1);
        run("(set-buffer-read-only t)", &env, &ctx);
        run(r#"(self-insert "x")"#, &env, &ctx);
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn the_insert_key_turns_the_mode_off_and_on() {
        let (ctx, env) = editor_with("abc", 0);
        press(&ctx, &env, KeyCode::Insert);
        assert!(run("(minor-mode-p 'overwrite-mode)", &env, &ctx).is_nil());
        press(&ctx, &env, KeyCode::Insert);
        assert!(run("(minor-mode-p 'overwrite-mode)", &env, &ctx).is_truthy());
    }

    #[test]
    fn an_editor_without_the_module_inserts() {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        run(
            r#"(insert "abc") (goto-char 0) (self-insert "x")"#,
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "xabc");
    }
}
