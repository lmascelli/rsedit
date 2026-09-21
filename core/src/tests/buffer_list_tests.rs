//! `buffer-list.lisp`: the listing, and `C-x b`.
//!
//! Like `dired`, this module keeps no state beside the buffer -- each line
//! carries a name and every command reads back what is on screen. Several of
//! these tests are about that: that a name is read by column rather than by
//! splitting on spaces, and that a line naming a buffer that has since been
//! killed is refused rather than acted on.
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
        env.set_variable("frame-width".into(), LispExp::number(100.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/completion.lisp"),
            include_str!("../../lisp/buffer-list.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    fn listing(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match run(
            r#"(with-current-buffer "*Buffer List*" (lambda () (buffer-string)))"#,
            env,
            ctx,
        ) {
            LispExp::String(s) => s.to_string(),
            other => panic!("expected the listing, got {other:?}"),
        }
    }

    // ----------------------------------------------------------------
    // The accessors the listing is built from
    // ----------------------------------------------------------------

    #[test]
    fn a_buffer_with_no_file_behind_it_has_no_file_name() {
        let (ctx, env) = editor();
        assert!(run(r#"(buffer-file-name "*scratch*")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn an_unknown_buffer_answers_nil_rather_than_failing() {
        // A listing asks about names it read off the screen, and one of them
        // may have been killed since.
        let (ctx, env) = editor();
        assert!(run(r#"(buffer-file-name "*nope*")"#, &env, &ctx).is_nil());
        assert!(run(r#"(buffer-modified-p "*nope*")"#, &env, &ctx).is_nil());
        assert!(run(r#"(major-mode "*nope*")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn typing_makes_a_buffer_modified() {
        let (ctx, env) = editor();
        assert!(run(r#"(buffer-modified-p "*scratch*")"#, &env, &ctx).is_nil());
        run(r#"(insert "something")"#, &env, &ctx);
        assert!(!run(r#"(buffer-modified-p "*scratch*")"#, &env, &ctx).is_nil());
    }

    // ----------------------------------------------------------------
    // The listing
    // ----------------------------------------------------------------

    #[test]
    fn every_buffer_gets_a_line() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "one" 'fundamental-mode)
               (buffer-create "two" 'fundamental-mode)
               (buffer-list)"#,
            &env,
            &ctx,
        );
        let shown = listing(&env, &ctx);
        for name in ["*scratch*", "one", "two", "*Buffer List*"] {
            assert!(shown.contains(name), "{name} should be listed:\n{shown}");
        }
    }

    #[test]
    fn a_modified_buffer_is_marked_and_a_clean_one_is_not() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "dirty" 'fundamental-mode)
               (switch-to-buffer "dirty")
               (insert "x")
               (buffer-list)"#,
            &env,
            &ctx,
        );
        let shown = listing(&env, &ctx);
        assert!(shown.contains("* dirty"), "expected a mark:\n{shown}");
        assert!(
            shown.lines().any(|l| l.starts_with("  *scratch*")),
            "a clean buffer gets no mark:\n{shown}"
        );
    }

    #[test]
    fn the_mode_is_shown() {
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'toy-mode)
               (buffer-create "toy" 'toy-mode)
               (buffer-list)"#,
            &env,
            &ctx,
        );
        assert!(listing(&env, &ctx).contains("toy-mode"));
    }

    #[test]
    fn a_long_name_pushes_the_columns_along_rather_than_being_cut() {
        // A truncated buffer name is one you cannot switch to by reading it,
        // and the columns after it are only decoration.
        let (ctx, env) = editor();
        let long = "a-really-quite-long-buffer-name-indeed";
        run(
            &format!(r#"(buffer-create "{long}" 'fundamental-mode) (buffer-list)"#),
            &env,
            &ctx,
        );
        assert!(listing(&env, &ctx).contains(long));
    }

    // ----------------------------------------------------------------
    // Reading a name back off the screen
    // ----------------------------------------------------------------

    #[test]
    fn return_switches_to_the_buffer_on_the_line() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "target" 'fundamental-mode)
               (buffer-list)"#,
            &env,
            &ctx,
        );
        // Find the line `target` is on and go to it.
        let shown = listing(&env, &ctx);
        let line = shown
            .lines()
            .position(|l| l.contains("target"))
            .expect("target should be listed")
            + 1;
        run(
            &format!("(goto-line {line}) (buffer-list-select)"),
            &env,
            &ctx,
        );
        assert_eq!(ctx.get_current_buffer_name(), "target");
    }

    #[test]
    fn a_name_with_spaces_in_it_is_still_read_whole() {
        // Why the name is read by column rather than split on whitespace --
        // `*Buffer List*` itself has a space in it.
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "two words" 'fundamental-mode) (buffer-list)"#,
            &env,
            &ctx,
        );
        let shown = listing(&env, &ctx);
        let line = shown
            .lines()
            .position(|l| l.contains("two words"))
            .expect("listed")
            + 1;
        run(
            &format!("(goto-line {line}) (buffer-list-select)"),
            &env,
            &ctx,
        );
        assert_eq!(ctx.get_current_buffer_name(), "two words");
    }

    #[test]
    fn a_line_naming_a_buffer_that_has_gone_is_refused() {
        // The listing is a drawing, and the world can move under it.
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "fleeting" 'fundamental-mode) (buffer-list)"#,
            &env,
            &ctx,
        );
        let shown = listing(&env, &ctx);
        let line = shown
            .lines()
            .position(|l| l.contains("fleeting"))
            .expect("listed")
            + 1;
        run(r#"(close-buffer "fleeting")"#, &env, &ctx);
        run(
            &format!(
                r#"(switch-to-buffer "*Buffer List*") (goto-line {line})
                   (buffer-list-select)"#
            ),
            &env,
            &ctx,
        );
        // Still in the listing: nothing was switched to.
        assert_eq!(ctx.get_current_buffer_name(), "*Buffer List*");
    }

    // ----------------------------------------------------------------
    // Killing
    // ----------------------------------------------------------------

    #[test]
    fn d_kills_an_unmodified_buffer_without_asking() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "spare" 'fundamental-mode) (buffer-list)"#,
            &env,
            &ctx,
        );
        let shown = listing(&env, &ctx);
        let line = shown
            .lines()
            .position(|l| l.contains("spare"))
            .expect("listed")
            + 1;
        run(
            &format!("(goto-line {line}) (buffer-list-kill)"),
            &env,
            &ctx,
        );
        assert!(ctx.get_buffer("spare").is_none());
        assert!(
            !listing(&env, &ctx).contains("spare"),
            "and the list redrew"
        );
    }

    #[test]
    fn d_on_a_modified_buffer_asks_first_and_kills_nothing_yet() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "precious" 'fundamental-mode)
               (switch-to-buffer "precious")
               (insert "work")
               (buffer-list)"#,
            &env,
            &ctx,
        );
        let shown = listing(&env, &ctx);
        let line = shown
            .lines()
            .position(|l| l.contains("precious"))
            .expect("listed")
            + 1;
        run(
            &format!("(goto-line {line}) (buffer-list-kill)"),
            &env,
            &ctx,
        );
        assert!(
            ctx.get_buffer("precious").is_some(),
            "the question is still open, so nothing has been killed"
        );
    }

    #[test]
    fn the_listing_refuses_to_kill_itself() {
        // Killing it from inside would leave the cursor in a buffer that is no
        // longer there.
        let (ctx, env) = editor();
        run("(buffer-list)", &env, &ctx);
        let shown = listing(&env, &ctx);
        let line = shown
            .lines()
            .position(|l| l.contains("*Buffer List*"))
            .expect("it lists itself")
            + 1;
        run(
            &format!("(goto-line {line}) (buffer-list-kill)"),
            &env,
            &ctx,
        );
        assert!(ctx.get_buffer("*Buffer List*").is_some());
    }

    // ----------------------------------------------------------------
    // C-x b
    // ----------------------------------------------------------------

    #[test]
    fn the_prompt_completes_over_buffer_names_fuzzily() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "notes.txt" 'fundamental-mode)"#,
            &env,
            &ctx,
        );
        let found = run(r#"(switch-to-buffer-candidates "nts")"#, &env, &ctx);
        let names: Vec<String> = found
            .iter()
            .map(|item| match item {
                LispExp::String(s) => s.to_string(),
                other => panic!("expected a name, got {other:?}"),
            })
            .collect();
        assert!(names.contains(&"notes.txt".to_string()), "got {names:?}");
    }

    #[test]
    fn an_empty_pattern_offers_every_buffer() {
        let (ctx, env) = editor();
        let all = run("(all-buffer-names)", &env, &ctx).iter().count();
        let found = run(r#"(switch-to-buffer-candidates "")"#, &env, &ctx);
        assert_eq!(found.iter().count(), all);
    }
}
