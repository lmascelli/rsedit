//! `risp-repl.lisp`: C-j in a REPL buffer evaluates the block of expressions
//! before point and writes the value under it.
//!
//! Design: `doc/architettura-editor.typ, <mod-risp-repl>`.
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

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/base/indent.lisp"),
            include_str!("../../lisp/languages/risp-mode.lisp"),
            include_str!("../../lisp/features/risp-repl.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// The *risp* buffer holding TEXT after its greeting, with point at the
    /// end.
    fn repl(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run("(risp-repl) (clear-buffer)", env, ctx);
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        run(&format!(r#"(insert "{escaped}")"#), env, ctx);
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|b| b.text.to_string())
    }

    /// Presses C-j, as the key would: through the binding of the buffer.
    fn press_c_j(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            r#"(call-interactively (car (nth 1 (key-binding "C-j"))))"#,
            env,
            ctx,
        );
    }

    #[test]
    fn the_repl_buffer_is_in_risp_mode_with_c_j_bound() {
        let (ctx, env) = editor();
        run("(risp-repl)", &env, &ctx);
        assert_eq!(
            run("(current-buffer)", &env, &ctx),
            LispExp::string("*risp*".into())
        );
        assert_eq!(
            run("(major-mode)", &env, &ctx),
            LispExp::symbol("risp-mode".into())
        );
        assert_eq!(
            run(r#"(car (nth 1 (key-binding "C-j")))"#, &env, &ctx),
            LispExp::symbol("risp-repl-eval-print-block".into())
        );
    }

    #[test]
    fn opening_the_repl_again_keeps_what_was_written() {
        let (ctx, env) = editor();
        repl("(+ 1 2)", &env, &ctx);
        run(r#"(buffer-create "other") (switch-to-buffer "other") (risp-repl)"#, &env, &ctx);
        assert_eq!(contents(&ctx), "(+ 1 2)");
    }

    #[test]
    fn c_j_writes_the_value_of_the_expression_before_point_under_it() {
        let (ctx, env) = editor();
        repl("(+ 1 2)", &env, &ctx);
        press_c_j(&env, &ctx);
        assert_eq!(contents(&ctx), "(+ 1 2)\n;; => 3\n");
    }

    #[test]
    fn a_block_is_evaluated_in_order_and_the_last_value_is_written() {
        let (ctx, env) = editor();
        repl(
            "(defun repl-test-twice (n)\n  (* 2 n))\n;; a comment\n(repl-test-twice 21)",
            &env,
            &ctx,
        );
        press_c_j(&env, &ctx);
        assert!(
            contents(&ctx).ends_with("(repl-test-twice 21)\n;; => 42\n"),
            "{}",
            contents(&ctx)
        );
    }

    #[test]
    fn a_blank_line_ends_the_block() {
        let (ctx, env) = editor();
        repl("(setq repl-test-x 1)\n\n(setq repl-test-y 2)", &env, &ctx);
        press_c_j(&env, &ctx);
        assert!(run("(boundp 'repl-test-y)", &env, &ctx).is_truthy());
        assert!(run("(boundp 'repl-test-x)", &env, &ctx).is_nil());
    }

    #[test]
    fn a_value_written_ends_the_block_before_it() {
        let (ctx, env) = editor();
        repl("(setq repl-test-n 0)", &env, &ctx);
        press_c_j(&env, &ctx);
        run(r#"(insert "(setq repl-test-n (+ repl-test-n 1))")"#, &env, &ctx);
        press_c_j(&env, &ctx);
        // Evaluated once more, the first setq would have reset it to 0 + 1.
        run(r#"(insert "(setq repl-test-n (+ repl-test-n 1))")"#, &env, &ctx);
        press_c_j(&env, &ctx);
        assert_eq!(
            run("repl-test-n", &env, &ctx),
            LispExp::number(2.0)
        );
        assert!(contents(&ctx).ends_with(";; => 2\n"), "{}", contents(&ctx));
    }

    #[test]
    fn a_blank_line_inside_an_expression_does_not_split_it() {
        let (ctx, env) = editor();
        repl("(list 1\n\n      2)", &env, &ctx);
        press_c_j(&env, &ctx);
        assert!(contents(&ctx).ends_with(";; => (1 2)\n"), "{}", contents(&ctx));
    }

    #[test]
    fn an_error_is_written_and_stops_the_block() {
        let (ctx, env) = editor();
        repl("(repl-test-no-such-function)\n(setq repl-test-after t)", &env, &ctx);
        press_c_j(&env, &ctx);
        assert!(run("(boundp 'repl-test-after)", &env, &ctx).is_nil());
        assert!(contents(&ctx).contains("\n;; !! "), "{}", contents(&ctx));
    }

    #[test]
    fn a_string_value_is_written_in_quotes_with_its_escapes() {
        let (ctx, env) = editor();
        repl(r#"(concat "say \"hi\"" "\\")"#, &env, &ctx);
        press_c_j(&env, &ctx);
        assert!(
            contents(&ctx).ends_with(";; => \"say \\\"hi\\\"\\\\\"\n"),
            "{}",
            contents(&ctx)
        );
    }

    #[test]
    fn every_line_of_a_value_is_a_comment() {
        let (ctx, env) = editor();
        repl(r#"(format "a\nb")"#, &env, &ctx);
        press_c_j(&env, &ctx);
        assert!(
            contents(&ctx).ends_with(";; => \"a\n;;    b\"\n"),
            "{}",
            contents(&ctx)
        );
    }

    #[test]
    fn inside_a_list_only_the_expression_before_point_is_evaluated() {
        let (ctx, env) = editor();
        repl("(list (+ 1 2) (+ 3 4)", &env, &ctx);
        press_c_j(&env, &ctx);
        assert!(contents(&ctx).ends_with("(+ 3 4)\n;; => 7\n"), "{}", contents(&ctx));
    }

    #[test]
    fn nothing_before_point_is_said_and_nothing_written() {
        let (ctx, env) = editor();
        repl("", &env, &ctx);
        press_c_j(&env, &ctx);
        assert_eq!(contents(&ctx), "");
    }

    #[test]
    fn the_keys_come_and_go_in_any_buffer() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "code.lisp" 'risp-mode) (switch-to-buffer "code.lisp")"#,
            &env,
            &ctx,
        );
        run("(risp-repl-mode)", &env, &ctx);
        assert_eq!(
            run(r#"(car (nth 1 (key-binding "C-j")))"#, &env, &ctx),
            LispExp::symbol("risp-repl-eval-print-block".into())
        );
        run("(risp-repl-mode)", &env, &ctx);
        assert!(run("(buffer-get 'local-keymap)", &env, &ctx).is_nil());
    }
}
