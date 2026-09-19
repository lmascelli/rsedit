//! Reading manual pages, and the primitives underneath.
//!
//! The tests that need `man` say so and skip when it is missing, so the suite
//! passes on a machine with no manual pages installed. Everything that does
//! not need it -- the lookup order, the overstrike stripping, the environment
//! primitives -- is tested unconditionally.
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
            include_str!("../../lisp/completion.lisp"),
            include_str!("../../lisp/manpage.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    fn text_of(answer: &LispExp<Ctx>) -> String {
        match answer {
            LispExp::String(s) => s.to_string(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    /// Whether `man` is installed, so the tests that need it can stand aside.
    fn has_man() -> bool {
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("command -v man")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    // ----------------------------------------------------------------
    // The environment
    // ----------------------------------------------------------------

    #[test]
    fn an_environment_variable_can_be_set_and_read_back() {
        let (ctx, env) = editor();
        run(r#"(setenv "RSEDIT_TEST_ONE" "value")"#, &env, &ctx);
        assert_eq!(
            run(r#"(getenv "RSEDIT_TEST_ONE")"#, &env, &ctx),
            LispExp::string("value".to_string())
        );
    }

    #[test]
    fn an_unset_variable_reads_as_nil() {
        let (ctx, env) = editor();
        assert!(run(r#"(getenv "RSEDIT_NO_SUCH_VARIABLE_AT_ALL")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn setenv_with_no_value_removes_it() {
        let (ctx, env) = editor();
        run(r#"(setenv "RSEDIT_TEST_TWO" "value")"#, &env, &ctx);
        run(r#"(setenv "RSEDIT_TEST_TWO")"#, &env, &ctx);
        assert!(run(r#"(getenv "RSEDIT_TEST_TWO")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn a_nonsense_variable_name_is_refused_rather_than_fatal() {
        // `set_var` panics on these, and a typo in a configuration file must
        // not take the editor down.
        let (ctx, env) = editor();
        assert!(eval_str(r#"(setenv "" "x")"#, &env, &ctx).is_err());
        assert!(eval_str(r#"(setenv "has=equals" "x")"#, &env, &ctx).is_err());
    }

    #[test]
    fn the_path_separator_is_asked_for_rather_than_assumed() {
        let (ctx, env) = editor();
        let separator = text_of(&run("(path-separator)", &env, &ctx));
        assert_eq!(separator, if cfg!(windows) { ";" } else { ":" });
    }

    #[test]
    fn a_child_process_sees_what_setenv_set() {
        // The whole point: MANPATH has to reach `man`, which is a process.
        let (ctx, env) = editor();
        run(r#"(setenv "RSEDIT_TEST_THREE" "visible")"#, &env, &ctx);
        let seen = text_of(&run(
            r#"(shell-command-to-string "printf %s \"$RSEDIT_TEST_THREE\"")"#,
            &env,
            &ctx,
        ));
        assert_eq!(seen, "visible");
    }

    // ----------------------------------------------------------------
    // Overstrike
    // ----------------------------------------------------------------

    /// The plain half of `(parse-overstrike TEXT)`.
    fn plain_of(answer: &LispExp<Ctx>) -> String {
        text_of(&answer.iter().next().expect("the plain text"))
    }

    /// The spans half, as (start, end, kind).
    fn spans_of(answer: &LispExp<Ctx>) -> Vec<(usize, usize, String)> {
        answer
            .iter()
            .nth(1)
            .expect("the spans")
            .iter()
            .map(|span| {
                let parts: Vec<LispExp<Ctx>> = span.iter().collect();
                let number = |exp: &LispExp<Ctx>| match exp {
                    LispExp::Number(n) => *n as usize,
                    other => panic!("expected a number, got {other:?}"),
                };
                let kind = match &parts[2] {
                    LispExp::Symbol(name) => name.to_string(),
                    other => panic!("expected a symbol, got {other:?}"),
                };
                (number(&parts[0]), number(&parts[1]), kind)
            })
            .collect()
    }

    #[test]
    fn overstruck_bold_gives_the_plain_word_and_a_bold_span() {
        let (ctx, env) = editor();
        // N\bNA\bAM\bME\bE -- how `man` writes a bold "NAME".
        let answer = run(
            "(parse-overstrike \"N\u{8}NA\u{8}AM\u{8}ME\u{8}E\")",
            &env,
            &ctx,
        );
        assert_eq!(plain_of(&answer), "NAME");
        // One span, not four: a word emphasised letter by letter is one
        // emphasised word, and an overlay per character would be thousands
        // for a page.
        assert_eq!(spans_of(&answer), vec![(0, 4, "bold".to_string())]);
    }

    #[test]
    fn overstruck_underline_gives_the_plain_word_and_an_underline_span() {
        let (ctx, env) = editor();
        // _\bf_\bi_\bl_\be -- how `man` writes an underlined "file".
        let answer = run(
            "(parse-overstrike \"_\u{8}f_\u{8}i_\u{8}l_\u{8}e\")",
            &env,
            &ctx,
        );
        assert_eq!(plain_of(&answer), "file");
        assert_eq!(spans_of(&answer), vec![(0, 4, "underline".to_string())]);
    }

    #[test]
    fn plain_text_has_no_spans() {
        let (ctx, env) = editor();
        let answer = run(r#"(parse-overstrike "plain text")"#, &env, &ctx);
        assert_eq!(plain_of(&answer), "plain text");
        assert!(spans_of(&answer).is_empty());
    }

    #[test]
    fn emphasis_in_the_middle_of_a_line_is_found_where_it_is() {
        // The case that needed overlays at all: a bold word inside a sentence,
        // which no syntax rule could match on.
        let (ctx, env) = editor();
        let answer = run(
            "(parse-overstrike \"see t\u{8}th\u{8}he\u{8}e file\")",
            &env,
            &ctx,
        );
        assert_eq!(plain_of(&answer), "see the file");
        assert_eq!(spans_of(&answer), vec![(4, 7, "bold".to_string())]);
    }

    #[test]
    fn two_runs_of_emphasis_are_two_spans() {
        let (ctx, env) = editor();
        let answer = run(
            "(parse-overstrike \"a\u{8}a b c\u{8}c\")",
            &env,
            &ctx,
        );
        assert_eq!(plain_of(&answer), "a b c");
        assert_eq!(
            spans_of(&answer),
            vec![(0, 1, "bold".to_string()), (4, 5, "bold".to_string())]
        );
    }

    #[test]
    fn bold_and_underline_next_to_each_other_do_not_merge() {
        let (ctx, env) = editor();
        let answer = run(
            "(parse-overstrike \"a\u{8}a_\u{8}b\")",
            &env,
            &ctx,
        );
        assert_eq!(plain_of(&answer), "ab");
        assert_eq!(
            spans_of(&answer),
            vec![(0, 1, "bold".to_string()), (1, 2, "underline".to_string())]
        );
    }

    #[test]
    fn a_backspace_at_the_start_removes_nothing_and_does_not_panic() {
        let (ctx, env) = editor();
        let answer = run("(parse-overstrike \"\u{8}abc\")", &env, &ctx);
        assert_eq!(plain_of(&answer), "abc");
    }

    #[test]
    fn a_trailing_backspace_does_not_panic() {
        let (ctx, env) = editor();
        let answer = run("(parse-overstrike \"abc\u{8}\")", &env, &ctx);
        assert_eq!(plain_of(&answer), "ab");
    }

    #[test]
    fn spans_are_offsets_into_the_plain_text_not_the_original() {
        // They are handed straight to `make-overlay`, so they have to index
        // the text that was actually inserted.
        let (ctx, env) = editor();
        let answer = run(
            "(parse-overstrike \"xx t\u{8}th\u{8}he\u{8}e\")",
            &env,
            &ctx,
        );
        let plain = plain_of(&answer);
        let (start, end, _) = spans_of(&answer)[0].clone();
        assert_eq!(&plain[start..end], "the");
    }

    // ----------------------------------------------------------------
    // Where rsedit's own pages live
    // ----------------------------------------------------------------

    #[test]
    fn the_data_directory_is_beside_the_executable() {
        let (ctx, env) = editor();
        let man = text_of(&run(r#"(data-directory "man")"#, &env, &ctx));
        assert!(man.ends_with("data/man") || man.ends_with("data\\man"), "got {man}");
    }

    #[test]
    fn data_directory_without_a_kind_is_the_root_of_it() {
        let (ctx, env) = editor();
        let root = text_of(&run("(data-directory)", &env, &ctx));
        assert!(root.ends_with("data"), "got {root}");
    }

    #[test]
    fn a_section_and_compression_suffix_is_not_part_of_the_name() {
        let (ctx, env) = editor();
        for (file, expected) in [
            ("ls.1.gz", "ls"),
            ("printf.3", "printf"),
            ("git-rebase.1.bz2", "git-rebase"),
            ("noextension", "noextension"),
        ] {
            assert_eq!(
                text_of(&run(
                    &format!(r#"(manpage--strip-extensions "{file}")"#),
                    &env,
                    &ctx
                )),
                expected,
                "for {file}"
            );
        }
    }

    #[test]
    fn manpath_is_built_from_the_variable_with_the_platforms_separator() {
        let (ctx, env) = editor();
        let joined = text_of(&run(
            r#"(manpage--join '("/one" "/two") (path-separator))"#,
            &env,
            &ctx,
        ));
        assert_eq!(joined, if cfg!(windows) { "/one;/two" } else { "/one:/two" });
    }

    // ----------------------------------------------------------------
    // Lookup order
    // ----------------------------------------------------------------

    #[test]
    fn rsedits_own_page_is_found() {
        // Shipped in man/ and copied to data/man by the build.
        let (ctx, env) = editor();
        let page = run(r#"(manpage--rsedit-page "buffers")"#, &env, &ctx);
        assert!(!page.is_nil(), "the shipped `buffers` page should be found");
        assert!(text_of(&page).contains("where text lives"));
    }

    #[test]
    fn a_name_with_no_rsedit_page_falls_through() {
        let (ctx, env) = editor();
        assert!(run(r#"(manpage--rsedit-page "no-such-topic")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn opening_an_rsedit_page_shows_it_in_the_manual_buffer() {
        let (ctx, env) = editor();
        run(r#"(manpage "windows")"#, &env, &ctx);
        assert_eq!(ctx.get_current_buffer_name(), "*Manual*");
        assert!(text_of(&run("(buffer-string)", &env, &ctx)).contains("what shows a buffer"));
    }

    #[test]
    fn the_manual_buffer_is_read_only() {
        let (ctx, env) = editor();
        run(r#"(manpage "windows")"#, &env, &ctx);
        let before = text_of(&run("(buffer-string)", &env, &ctx));
        run(r#"(goto-char 0) (insert "typed")"#, &env, &ctx);
        assert_eq!(text_of(&run("(buffer-string)", &env, &ctx)), before);
    }

    #[test]
    fn a_name_that_is_nowhere_says_so() {
        let (ctx, env) = editor();
        let before = ctx.get_current_buffer_name();
        run(r#"(manpage "definitely-not-a-manual-page-xyzzy")"#, &env, &ctx);
        assert_eq!(
            ctx.get_current_buffer_name(),
            before,
            "nothing should have been opened"
        );
    }

    #[test]
    fn rsedits_pages_are_offered_for_completion() {
        let (ctx, env) = editor();
        let found = run(r#"(manpage-candidates "buf")"#, &env, &ctx);
        let names: Vec<String> = found.iter().map(|item| text_of(&item)).collect();
        assert!(names.contains(&"buffers".to_string()), "got {names:?}");
    }

    // ----------------------------------------------------------------
    // The system's pages, where there are any
    // ----------------------------------------------------------------

    #[test]
    fn a_system_page_is_rendered_without_overstrike() {
        if !has_man() {
            return;
        }
        let (ctx, env) = editor();
        run(r#"(manpage "man")"#, &env, &ctx);
        let shown = text_of(&run("(buffer-string)", &env, &ctx));
        if shown.is_empty() {
            // `man` is installed but has no pages -- a minimal container.
            return;
        }
        assert!(
            !shown.contains('\u{8}'),
            "no backspace should survive into the buffer"
        );
    }
}
