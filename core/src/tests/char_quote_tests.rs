//! Character literals, as the syntax table's character quote declares them:
//! that `'"'` is one atom, so its `"` opens no string, that a lifetime is not
//! a literal, and that what reads the table, `electric-pair`, `forward-sexp`
//! and `syntax-ppss`, reads them so.
//!
//! Design: `doc/architettura-editor.typ, <tabella-sintattica>`.
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

    /// Returns an editor with rust-mode and electric-pair, as the real one has
    /// them.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/indent.lisp"),
            include_str!("../../lisp/rust-mode.lisp"),
            include_str!("../../lisp/electric-pair.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// Makes NAME a fresh rust-mode buffer holding TEXT, with point at AT.
    fn rust_buffer(name: &str, text: &str, at: usize, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        run(
            &format!(
                r#"(buffer-create "{name}" 'rust-mode) (switch-to-buffer "{name}")
                   (clear-buffer) (insert "{escaped}") (goto-char {at})"#
            ),
            env,
            ctx,
        );
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|b| b.text.to_string())
    }

    /// Returns the list depth at the end of the buffer: 0 when every list is
    /// closed.
    fn depth_at_end(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> f64 {
        match run("(nth 0 (syntax-ppss (point-max)))", env, ctx) {
            LispExp::Number(n) => n,
            other => panic!("expected a depth, got {other:?}"),
        }
    }

    // ----------------------------------------------------------------
    // What the scanner sees
    // ----------------------------------------------------------------

    #[test]
    fn a_quote_inside_a_character_literal_does_not_open_a_string() {
        // At its root: the `"` inside the literal opens nothing.
        let (ctx, env) = editor();
        rust_buffer("a", "fn a() {\n  let c = '\"';\n}", 0, &env, &ctx);
        assert_eq!(
            depth_at_end(&env, &ctx),
            0.0,
            "the braces balance, so nothing after the char literal was eaten"
        );
    }

    #[test]
    fn a_brace_after_a_character_literal_still_counts() {
        let (ctx, env) = editor();
        rust_buffer("b", "let c = '\"';\nfn a() {", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 1.0, "the unclosed brace is seen");
    }

    #[test]
    fn a_lifetime_is_not_a_character_literal() {
        // `'static` never closes, so it is not a literal -- and must not be
        // treated as one, or everything after it would be swallowed.
        let (ctx, env) = editor();
        rust_buffer("c", "fn a<'x>(r: &'x str) {\n}", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn an_apostrophe_in_a_comment_changes_nothing() {
        let (ctx, env) = editor();
        rust_buffer("d", "fn a() {\n  // don't do this\n}", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn an_apostrophe_inside_a_string_changes_nothing() {
        // Why `'` is not an escape character: escapes count inside strings
        // too, so it would eat this closing quote.
        let (ctx, env) = editor();
        rust_buffer("e", "fn a() {\n  let s = \"it's\";\n}", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn an_escaped_character_literal_is_read_whole() {
        let (ctx, env) = editor();
        rust_buffer("f", "fn a() {\n  let n = '\\n';\n}", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn a_brace_inside_a_character_literal_is_not_a_brace() {
        // `'{'` is one atom, so it opens nothing.
        let (ctx, env) = editor();
        rust_buffer("g", "let c = '{';\n", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn a_mode_without_a_char_quote_is_unaffected() {
        // Lisp has no character literals, and `'` there is a prefix. Declaring
        // one per mode is why that keeps working.
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'plain-mode) (buffer-create "h" 'plain-mode)
               (switch-to-buffer "h") (insert "a 'b' c {")"#,
            &env,
            &ctx,
        );
        assert_eq!(depth_at_end(&env, &ctx), 1.0);
    }

    // ----------------------------------------------------------------
    // As a user meets it
    // ----------------------------------------------------------------

    #[test]
    fn retyping_a_brace_in_a_file_with_a_character_literal_adds_no_second_one() {
        // The `{` of a function whose body holds `'"'`, deleted and typed
        // back.
        let (ctx, env) = editor();
        rust_buffer("i", "fn a() \n  let c = '\"';\n}", 7, &env, &ctx);
        run(
            r#"(self-insert "{") (electric-pair-post-self-insert)"#,
            &env,
            &ctx,
        );
        assert_eq!(contents(&ctx), "fn a() {\n  let c = '\"';\n}");
    }

    #[test]
    fn pairing_still_works_in_the_same_file_where_it_should() {
        // The check is not "never pair".
        let (ctx, env) = editor();
        rust_buffer("j", "fn a() \n  let c = '\"';\n", 7, &env, &ctx);
        run(
            r#"(self-insert "{") (electric-pair-post-self-insert)"#,
            &env,
            &ctx,
        );
        assert_eq!(contents(&ctx), "fn a() {}\n  let c = '\"';\n");
    }

    #[test]
    fn forward_sexp_steps_over_a_character_literal_as_one_thing() {
        // The table is read by more than electric-pair, and every reader sees
        // the literal as one atom.
        let (ctx, env) = editor();
        rust_buffer("k", "'a' next", 0, &env, &ctx);
        run("(forward-sexp)", &env, &ctx);
        assert_eq!(run("(point)", &env, &ctx), LispExp::number(3.0));
    }

    #[test]
    fn a_quote_that_never_closes_is_left_as_punctuation() {
        // A lifetime at the very end of a buffer: not a literal, so the quote
        // is ordinary punctuation -- and what follows it is still code. Were
        // the scan to treat it as a literal, or to run off the end looking for
        // a closer, the brace after it would stop counting.
        let (ctx, env) = editor();
        rust_buffer("l", "fn a<'x>() {\n", 0, &env, &ctx);
        assert_eq!(
            depth_at_end(&env, &ctx),
            1.0,
            "the unclosed brace is still seen"
        );
    }

    #[test]
    fn a_quote_at_the_very_end_of_the_buffer_does_not_run_off_it() {
        // Nothing follows for the lookahead to read.
        let (ctx, env) = editor();
        rust_buffer("m", "fn a() {}\nlet r: &'", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }
}
