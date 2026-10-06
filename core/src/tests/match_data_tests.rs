//! `string-match` and the match data, as Emacs has them.
//!
//! The contract is Emacs': `string-match` answers where the match starts, and
//! the groups are read afterwards with `match-beginning`, `match-end` and
//! `match-string`. Most of what is worth pinning is where this could quietly
//! differ from Emacs and still look like it works -- byte offsets instead of
//! characters, `^` matching at START, a failed match wiping the data, a bad
//! pattern answering nil as though it had simply not matched.
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

    /// With `commands.lisp`, which is where `save-match-data` lives.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        eval_str(include_str!("../../lisp/commands.lisp"), &env, &ctx)
            .expect("loading commands.lisp");
        (ctx, env)
    }

    fn number(n: f64) -> LispExp<Ctx> {
        LispExp::number(n)
    }

    fn string(s: &str) -> LispExp<Ctx> {
        LispExp::string(s.into())
    }

    /// What `(match-data)` answers, as Rust can compare it.
    fn data(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Vec<Option<f64>> {
        run("(match-data)", env, ctx)
            .iter()
            .map(|item| match item {
                LispExp::Number(n) => Some(n),
                other if other.is_nil() => None,
                other => panic!("match data holds {other:?}"),
            })
            .collect()
    }

    // ----------------------------------------------------------------
    // What `string-match` answers, and what it records
    // ----------------------------------------------------------------

    #[test]
    fn string_match_answers_where_the_match_starts() {
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(string-match "[0-9]+" "see main.rs:42")"#, &env, &ctx),
            number(12.0)
        );
    }

    #[test]
    fn the_groups_are_read_from_the_match_data() {
        let (ctx, env) = editor();
        run(
            r#"(setq line "see src/main.rs:42 there")
               (string-match "([a-z.]+):([0-9]+)" line)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(match-string 0 line)", &env, &ctx),
            string("main.rs:42")
        );
        assert_eq!(run("(match-string 1 line)", &env, &ctx), string("main.rs"));
        assert_eq!(run("(match-string 2 line)", &env, &ctx), string("42"));
        assert_eq!(run("(match-beginning 2)", &env, &ctx), number(16.0));
        assert_eq!(run("(match-end 2)", &env, &ctx), number(18.0));
    }

    #[test]
    fn a_group_that_did_not_take_part_is_nil_and_keeps_its_place() {
        // So that "group 3 is the column" is right whether or not group 2 was
        // there -- which is what `compilation-patterns' depends on.
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(string-match "(a)|(b)" "b")"#, &env, &ctx),
            number(0.0)
        );
        assert!(run("(match-beginning 1)", &env, &ctx).is_nil());
        assert!(run(r#"(match-string 1 "b")"#, &env, &ctx).is_nil());
        assert_eq!(run(r#"(match-string 2 "b")"#, &env, &ctx), string("b"));
        assert_eq!(
            data(&env, &ctx),
            vec![Some(0.0), Some(1.0), None, None, Some(0.0), Some(1.0)]
        );
    }

    #[test]
    fn a_group_past_the_last_one_is_nil() {
        let (ctx, env) = editor();
        run(r#"(string-match "a" "a")"#, &env, &ctx);
        assert!(run("(match-beginning 7)", &env, &ctx).is_nil());
    }

    #[test]
    fn no_match_is_nil_and_leaves_the_last_match_alone() {
        let (ctx, env) = editor();
        run(r#"(string-match "b" "abc")"#, &env, &ctx);
        assert!(run(r#"(string-match "zzz" "abc")"#, &env, &ctx).is_nil());
        assert_eq!(data(&env, &ctx), vec![Some(1.0), Some(2.0)]);
    }

    #[test]
    fn positions_are_characters_not_bytes() {
        // `è' is two bytes. Emacs, `substring' and `length' all count it as
        // one, so a byte offset here would be off by one after it -- and
        // `match-string' would cut through the middle of a character.
        let (ctx, env) = editor();
        assert_eq!(run(r#"(string-match ":" "è:42")"#, &env, &ctx), number(1.0));
        run(r#"(string-match "([0-9]+)" "città 42")"#, &env, &ctx);
        assert_eq!(
            run(r#"(match-string 1 "città 42")"#, &env, &ctx),
            string("42")
        );
    }

    // ----------------------------------------------------------------
    // START
    // ----------------------------------------------------------------

    #[test]
    fn start_skips_ahead_and_the_answer_still_counts_from_the_front() {
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(string-match "b" "abab" 2)"#, &env, &ctx),
            number(3.0)
        );
    }

    #[test]
    fn a_negative_start_counts_from_the_end() {
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(string-match "b" "abab" -1)"#, &env, &ctx),
            number(3.0)
        );
    }

    #[test]
    fn caret_still_means_the_start_of_the_string_not_of_the_search() {
        // As in Emacs. Otherwise a loop walking a string with START would find
        // a "line start" at every place it resumed.
        let (ctx, env) = editor();
        assert!(run(r#"(string-match "^b" "ab" 1)"#, &env, &ctx).is_nil());
    }

    #[test]
    fn a_start_outside_the_string_is_out_of_range() {
        let (ctx, env) = editor();
        assert_eq!(
            run(
                r#"(condition-case nil (string-match "a" "abc" 9) (args-out-of-range 'out))"#,
                &env,
                &ctx
            ),
            LispExp::symbol("out".into())
        );
    }

    // ----------------------------------------------------------------
    // Errors
    // ----------------------------------------------------------------

    #[test]
    fn a_pattern_that_does_not_compile_signals_invalid_regexp() {
        // Not nil: a pattern that cannot match anything is a bug in the
        // pattern, and nil would make it look like a string that did not
        // match.
        let (ctx, env) = editor();
        assert_eq!(
            run(
                r#"(condition-case nil (string-match "(unclosed" "abc") (invalid-regexp 'caught))"#,
                &env,
                &ctx
            ),
            LispExp::symbol("caught".into())
        );
        assert_eq!(
            run(
                r#"(condition-case nil (string-match "(unclosed" "abc") (error 'caught))"#,
                &env,
                &ctx
            ),
            LispExp::symbol("caught".into()),
            "and it is an error, so a handler for any error catches it"
        );
    }

    #[test]
    fn match_string_needs_the_string_that_was_matched() {
        // Emacs reads the buffer when STRING is left out. There is no buffer
        // regexp search here to have matched there, so leaving it out is an
        // arity error rather than a quiet read of the wrong text.
        let (ctx, env) = editor();
        run(r#"(string-match "b" "abc")"#, &env, &ctx);
        assert!(eval_str("(match-string 0)", &env, &ctx).is_err());
    }

    #[test]
    fn match_string_on_a_shorter_string_is_out_of_range_not_a_panic() {
        let (ctx, env) = editor();
        run(r#"(string-match "c" "abc")"#, &env, &ctx);
        assert!(eval_str(r#"(match-string 0 "a")"#, &env, &ctx).is_err());
    }

    // ----------------------------------------------------------------
    // Keeping it
    // ----------------------------------------------------------------

    #[test]
    fn string_match_p_leaves_the_match_data_alone() {
        let (ctx, env) = editor();
        run(r#"(string-match "b" "abc")"#, &env, &ctx);
        assert_eq!(
            run(r#"(string-match-p "c" "abc")"#, &env, &ctx),
            number(2.0)
        );
        assert_eq!(data(&env, &ctx), vec![Some(1.0), Some(2.0)]);
    }

    #[test]
    fn set_match_data_puts_back_what_match_data_took() {
        let (ctx, env) = editor();
        run(
            r#"(string-match "(a)|(b)" "b")
               (setq saved (match-data))
               (string-match "x" "xyz")
               (set-match-data saved)"#,
            &env,
            &ctx,
        );
        assert_eq!(run(r#"(match-string 2 "b")"#, &env, &ctx), string("b"));
        assert!(run("(match-beginning 1)", &env, &ctx).is_nil());
    }

    #[test]
    fn save_match_data_restores_the_match_after_its_body() {
        let (ctx, env) = editor();
        run(
            r#"(string-match "b" "abc")
               (save-match-data (string-match "z" "xyz"))"#,
            &env,
            &ctx,
        );
        assert_eq!(data(&env, &ctx), vec![Some(1.0), Some(2.0)]);
    }

    #[test]
    fn save_match_data_restores_it_even_when_the_body_fails() {
        let (ctx, env) = editor();
        run(
            r#"(string-match "b" "abc")
               (condition-case nil
                   (save-match-data
                     (string-match "z" "xyz")
                     (error "on the way out"))
                 (error nil))"#,
            &env,
            &ctx,
        );
        assert_eq!(data(&env, &ctx), vec![Some(1.0), Some(2.0)]);
    }

    #[test]
    fn save_match_data_answers_what_its_body_does() {
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(save-match-data (string-match "z" "xyz"))"#, &env, &ctx),
            number(2.0)
        );
    }

    #[test]
    fn another_thread_has_match_data_of_its_own() {
        // A worker matching its own strings must not change what the command
        // on the main thread is about to read.
        let (ctx, env) = editor();
        run(r#"(string-match "b" "abc")"#, &env, &ctx);

        std::thread::spawn(|| {
            let (ctx, env) = editor();
            assert!(
                run("(match-data)", &env, &ctx).is_nil(),
                "a fresh thread starts with none"
            );
            run(r#"(string-match "yz" "xyz")"#, &env, &ctx);
        })
        .join()
        .expect("the other thread");

        assert_eq!(data(&env, &ctx), vec![Some(1.0), Some(2.0)]);
    }
}
