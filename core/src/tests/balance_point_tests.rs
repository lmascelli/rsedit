//! Asking whether an opener already has a closer, and what it costs.
//!
//! # The question these are about
//!
//! `electric-pair` has to decide, having just seen a `{` typed, whether to put
//! a `}` after it. Three ways of asking have been tried and two are wrong:
//!
//! * *Is the innermost list closed?* A scanner matches a closer to the
//!   innermost opener, so a new brace inside an already-closed block appears
//!   to have taken the outer one's closer, and pairing is refused exactly
//!   where it is wanted.
//! * *Does the whole buffer balance?* Anything below that is still being typed
//!   keeps the depth from ever reaching zero at the end, so a brace whose
//!   partner is on the very next line is reported as unclosed.
//! * *Does the depth reach zero anywhere ahead?* Neither fault, and it stops
//!   at the first point of balance rather than the end of the file.
//!
//! The second was what shipped, and the test for it is the first one below.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
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

    /// Type OPENER at point and let the pairing hook decide.
    fn type_opener(ctx: &Ctx, env: &Arc<Env<Ctx>>, opener: &str) {
        run(
            &format!(r#"(self-insert "{opener}") (electric-pair-post-self-insert)"#),
            env,
            ctx,
        );
    }

    // ----------------------------------------------------------------
    // What electric-pair does with it
    // ----------------------------------------------------------------

    #[test]
    fn an_unfinished_function_below_does_not_earn_a_second_closer() {
        // The bug the whole-buffer check had. `a`'s closer is on the very next
        // line; `b` below is half-typed, as any function is while you write
        // it. Asking whether the *buffer* balances hears "no" -- because of
        // `b` -- and adds a brace `a` already had.
        let (ctx, env) = editor();
        rust_buffer("a", "fn a() \n}\nfn b() {\n", 7, &env, &ctx);
        type_opener(&ctx, &env, "{");
        assert_eq!(contents(&ctx), "fn a() {\n}\nfn b() {\n");
    }

    #[test]
    fn a_brace_whose_partner_is_missing_still_gets_one() {
        // The other direction: nothing ahead closes it, so it wants a closer.
        let (ctx, env) = editor();
        rust_buffer("b", "fn a() \nfn b() {\n}\n", 7, &env, &ctx);
        type_opener(&ctx, &env, "{");
        assert_eq!(contents(&ctx), "fn a() {}\nfn b() {\n}\n");
    }

    #[test]
    fn a_brace_nested_inside_a_closed_block_gets_its_own() {
        // The fault of asking about the innermost list. The `}` below closes
        // the *function*; the new brace has nothing, and wants a partner.
        let (ctx, env) = editor();
        rust_buffer("c", "fn a() {\n  if b \n}\n", 16, &env, &ctx);
        type_opener(&ctx, &env, "{");
        assert_eq!(contents(&ctx), "fn a() {\n  if b {}\n}\n");
    }

    #[test]
    fn repairing_a_brace_adds_nothing() {
        // The case that started all of this: delete the `{` of a function
        // whose body is intact, and type it back.
        let (ctx, env) = editor();
        rust_buffer("d", "fn a() \n  let c = 1;\n}\n", 7, &env, &ctx);
        type_opener(&ctx, &env, "{");
        assert_eq!(contents(&ctx), "fn a() {\n  let c = 1;\n}\n");
    }

    // ----------------------------------------------------------------
    // The primitive itself
    // ----------------------------------------------------------------

    fn balance_point(ctx: &Ctx, env: &Arc<Env<Ctx>>, at: usize) -> Option<usize> {
        match run(&format!("(balance-point {at})"), env, ctx) {
            LispExp::Number(n) => Some(n as usize),
            other if other.is_nil() => None,
            other => panic!("expected a position or nil, got {other:?}"),
        }
    }

    #[test]
    fn at_the_top_level_nothing_has_to_close() {
        let (ctx, env) = editor();
        rust_buffer("e", "fn a() {\n}\n", 0, &env, &ctx);
        assert_eq!(balance_point(&ctx, &env, 0), Some(0));
    }

    #[test]
    fn inside_a_list_it_is_just_past_the_closer() {
        let (ctx, env) = editor();
        let text = "fn a() {\n}\n";
        rust_buffer("f", text, 0, &env, &ctx);
        // Just after the `{`: the depth returns to zero one past the `}`.
        let closer = text.find('}').expect("a closer");
        assert_eq!(balance_point(&ctx, &env, 8), Some(closer + 1));
    }

    #[test]
    fn an_opener_with_nothing_to_close_it_answers_nothing() {
        let (ctx, env) = editor();
        rust_buffer("g", "fn a() {\n  let x = 1;\n", 0, &env, &ctx);
        assert_eq!(balance_point(&ctx, &env, 8), None);
    }

    #[test]
    fn it_reaches_zero_before_the_end_rather_than_at_it() {
        // The whole difference from "does the buffer balance", in one
        // assertion: the depth is zero here, and not zero at the end.
        let (ctx, env) = editor();
        let text = "fn a() {\n}\nfn b() {\n";
        rust_buffer("h", text, 0, &env, &ctx);
        assert_eq!(
            balance_point(&ctx, &env, 8),
            Some(text.find('}').expect("a closer") + 1)
        );
        assert_ne!(
            run("(nth 0 (syntax-ppss (point-max)))", &env, &ctx),
            LispExp::number(0.0),
            "while the buffer as a whole does not balance at all"
        );
    }

    #[test]
    fn it_stops_at_the_enclosing_list_rather_than_the_end_of_the_file() {
        // The cost claim, as a property rather than a timing: the answer is
        // near point, not near the end, however much file is below.
        let (ctx, env) = editor();
        let mut text = String::from("fn a() {\n}\n");
        for n in 0..400 {
            text.push_str(&format!("fn f{n}() {{\n}}\n"));
        }
        rust_buffer("i", &text, 0, &env, &ctx);
        let at = balance_point(&ctx, &env, 8).expect("the brace is closed");
        assert_eq!(at, 10, "one past the closing brace on the second line");
        assert!(
            at < text.len() / 100,
            "and nowhere near the end of a {}-character buffer",
            text.len()
        );
    }
}
