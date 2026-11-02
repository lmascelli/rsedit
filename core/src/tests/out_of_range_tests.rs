//! Numbers too large to be what they ask for: a width past `WIDTH_LIMIT` is
//! refused with `args-out-of-range` before anything is allocated, and a count
//! past the buffer goes as far as the buffer does. None of them may panic or
//! end the process, which is what they did before.
//!
//! Design: `doc/architettura-editor.typ, <lettori-argomenti>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        ctx.with_buffer_mut("*scratch*", |b| b.text = GapBuffer::from(text));
        (ctx, env)
    }

    fn try_run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        try_run(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn text_of(ctx: &Ctx) -> String {
        ctx.with_buffer("*scratch*", |b| b.text.to_string())
            .expect("*scratch*")
    }

    /// Asserts that SRC is refused with `args-out-of-range`.
    fn out_of_range(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        let refused = try_run(src, env, ctx).expect_err("refused");
        assert!(
            format!("{refused:?}").contains("args-out-of-range"),
            "{src}: {refused:?}"
        );
    }

    #[test]
    fn indent_line_to_a_column_past_the_limit_is_refused_and_changes_nothing() {
        let (ctx, env) = editor_with("  text");
        // The card's case, which ended the process: three hundred gigabytes.
        out_of_range("(indent-line-to 300000000000)", &env, &ctx);
        out_of_range("(indent-line-to 1000001)", &env, &ctx);
        assert_eq!(text_of(&ctx), "  text");
    }

    #[test]
    fn indent_line_to_the_limit_itself_is_done() {
        let (ctx, env) = editor_with("text");
        assert_eq!(run("(indent-line-to 1000000)", &env, &ctx), LispExp::t());
        assert_eq!(text_of(&ctx).chars().count(), 1_000_000 + 4);
    }

    #[test]
    fn a_counter_wider_than_format_takes_is_padded_all_the_same() {
        let (ctx, env) = editor_with("");
        // Past 65 535, where `format!` panicked.
        let text = run("(kmacro-insert-counter 70000)", &env, &ctx);
        let LispExp::String(text) = text else {
            panic!("expected the text inserted, got {text:?}");
        };
        assert_eq!(text.len(), 70_000);
        assert!(text.chars().all(|c| c == '0'));
    }

    #[test]
    fn a_counter_width_past_the_limit_is_refused_and_the_counter_stays() {
        let (ctx, env) = editor_with("");
        let before = run("(kmacro-counter)", &env, &ctx);
        out_of_range("(kmacro-insert-counter 1000000000000)", &env, &ctx);
        assert_eq!(run("(kmacro-counter)", &env, &ctx), before);
        assert_eq!(text_of(&ctx), "");
    }

    #[test]
    fn a_negative_counter_is_padded_after_its_sign() {
        let (ctx, env) = editor_with("");
        run("(kmacro-set-counter -5)", &env, &ctx);
        assert_eq!(
            run("(kmacro-insert-counter 3)", &env, &ctx),
            LispExp::string("-005".into())
        );
        assert_eq!(
            run("(kmacro-insert-counter)", &env, &ctx),
            LispExp::string("-4".into())
        );
    }

    /// A count far past the end deletes to the end, forwards. Before, the
    /// position plus the count overflowed: a panic in a debug build, and in a
    /// release build a deletion of the character *before* point.
    #[test]
    fn delete_char_with_a_huge_count_deletes_to_the_end() {
        let (ctx, env) = editor_with("one two");
        run(
            "(goto-char 5) (delete-char 1000000000000000000000000000000)",
            &env,
            &ctx,
        );
        assert_eq!(text_of(&ctx), "one t");
    }

    /// The commands that move or kill by a count stop at the edge of the
    /// buffer however large the count: no limit is needed, and none refuses.
    #[test]
    fn a_huge_count_moves_and_kills_as_far_as_the_buffer_goes() {
        for (src, expected) in [
            ("(goto-char 0) (forward-word 1000000000000000)", "one two"),
            ("(goto-char 0) (kill-word 1000000000000000)", ""),
            ("(goto-char 7) (backward-kill-word 1000000000000000)", ""),
            ("(goto-char 0) (forward-char 1000000000000000)", "one two"),
            ("(next-line 1000000000000000)", "one two"),
        ] {
            let (ctx, env) = editor_with("one two");
            run(src, &env, &ctx);
            assert_eq!(text_of(&ctx), expected, "{src}");
        }
    }
}
