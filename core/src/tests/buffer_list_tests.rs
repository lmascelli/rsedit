//! `buffer-list.lisp`: the listing and `C-x b`, and the names read back off
//! the screen, by column, refused when the buffer is gone.
//!
//! Design: `doc/architettura-editor.typ, <mod-buffer-list>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
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
        env.set_variable("frame-width".into(), LispExp::number(100.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/completion.lisp"),
            include_str!("../../lisp/features/buffer-list.lisp"),
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
            shown.lines().any(|l| l.starts_with("    *scratch*")),
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
        assert!(!ctx.has_buffer("spare"));
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
            ctx.has_buffer("precious"),
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
        assert!(ctx.has_buffer("*Buffer List*"));
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

    // ----------------------------------------------------------------
    // Marks
    // ----------------------------------------------------------------

    fn press(c: char, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    /// The listing's line for the buffer NAME, counting from 1.
    fn line_of(name: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> usize {
        listing(env, ctx)
            .lines()
            .position(|line| {
                line.get(4..)
                    .is_some_and(|rest| rest.starts_with(&format!("{name} ")))
            })
            .map(|n| n + 1)
            .unwrap_or_else(|| panic!("{name} is not listed:\n{}", listing(env, ctx)))
    }

    /// The first two columns of NAME's line: its mark and its save flag.
    fn marks_of(name: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        listing(env, ctx)
            .lines()
            .nth(line_of(name, env, ctx) - 1)
            .map(|line| line.chars().take(2).collect())
            .unwrap_or_default()
    }

    fn press_on(name: &str, key: char, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!("(goto-line {})", line_of(name, env, ctx)),
            env,
            ctx,
        );
        press(key, env, ctx);
    }

    fn answer(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for c in text.chars() {
            run(&format!(r#"(self-insert "{c}")"#), env, ctx);
        }
        run("(minibuffer-confirm)", env, ctx);
    }

    /// Three spare buffers, and the listing open on them.
    fn spares(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            r#"(buffer-create "one" 'fundamental-mode)
               (buffer-create "two" 'fundamental-mode)
               (buffer-create "three" 'fundamental-mode)
               (buffer-list)"#,
            env,
            ctx,
        );
    }

    /// A file of this test's own, open in a buffer with unsaved changes.
    fn dirty_file(name: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rsedit-buffer-list-{name}-{}.txt",
            std::process::id()
        ));
        std::fs::write(&path, "before").expect("write");
        run(
            &format!(
                r#"(find-file "{}") (goto-char (point-max)) (insert " after")"#,
                path.display()
            ),
            env,
            ctx,
        );
        path
    }

    #[test]
    fn m_d_and_s_mark_the_columns_and_u_clears_both() {
        let (ctx, env) = editor();
        let path = dirty_file("columns", &env, &ctx);
        let file_buffer = path.file_name().unwrap().to_string_lossy().to_string();
        spares(&env, &ctx);
        press_on("one", 'm', &env, &ctx);
        press_on("two", 'd', &env, &ctx);
        press_on(&file_buffer, 's', &env, &ctx);
        assert_eq!(marks_of("one", &env, &ctx), "> ");
        assert_eq!(marks_of("two", &env, &ctx), "D ");
        assert_eq!(marks_of(&file_buffer, &env, &ctx), " S");
        press_on("two", 'u', &env, &ctx);
        assert_eq!(marks_of("two", &env, &ctx), "  ");
        run("(buffer-list-refresh)", &env, &ctx);
        assert_eq!(marks_of("one", &env, &ctx), "> ", "kept when drawn again");
        press('U', &env, &ctx);
        assert_eq!(marks_of("one", &env, &ctx), "  ");
        assert_eq!(marks_of(&file_buffer, &env, &ctx), "  ");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn s_refuses_a_buffer_that_visits_no_file() {
        let (ctx, env) = editor();
        spares(&env, &ctx);
        press_on("one", 's', &env, &ctx);
        assert_eq!(marks_of("one", &env, &ctx), "  ");
        assert!(ctx.get_echo_message().contains("visits no file"));
    }

    #[test]
    fn x_kills_the_flagged_buffers_and_saves_the_flagged_ones() {
        let (ctx, env) = editor();
        let path = dirty_file("execute", &env, &ctx);
        let file_buffer = path.file_name().unwrap().to_string_lossy().to_string();
        spares(&env, &ctx);
        press_on("one", 'd', &env, &ctx);
        press_on("three", 'k', &env, &ctx);
        press_on(&file_buffer, 's', &env, &ctx);
        press('x', &env, &ctx);
        assert!(!ctx.has_buffer("one"));
        assert!(ctx.has_buffer("two"));
        assert!(!ctx.has_buffer("three"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before after");
        assert_eq!(
            marks_of(&file_buffer, &env, &ctx),
            "  ",
            "the flag goes once saved"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn x_asks_once_for_the_modified_ones_and_no_keeps_only_those() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "a" 'fundamental-mode)
               (buffer-create "b" 'fundamental-mode)
               (with-current-buffer "a" (lambda () (insert "x")))
               (with-current-buffer "b" (lambda () (insert "y")))"#,
            &env,
            &ctx,
        );
        spares(&env, &ctx);
        for name in ["a", "b", "one"] {
            press_on(name, 'd', &env, &ctx);
        }
        press('x', &env, &ctx);
        assert!(
            ctx.has_buffer("a") && ctx.has_buffer("one"),
            "nothing before the answer"
        );
        answer("no", &env, &ctx);
        assert!(
            ctx.has_buffer("a") && ctx.has_buffer("b"),
            "no keeps the modified ones"
        );
        assert!(!ctx.has_buffer("one"), "and kills the others");
        assert!(
            ctx.get_echo_message().contains("kept 2"),
            "{}",
            ctx.get_echo_message()
        );
    }

    #[test]
    fn shift_d_kills_the_marked_buffers_and_yes_takes_the_modified_too() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "a" 'fundamental-mode)
               (with-current-buffer "a" (lambda () (insert "x")))"#,
            &env,
            &ctx,
        );
        spares(&env, &ctx);
        press_on("a", 'm', &env, &ctx);
        press_on("two", 'm', &env, &ctx);
        run(
            &format!("(goto-line {})", line_of("one", &env, &ctx)),
            &env,
            &ctx,
        );
        press('D', &env, &ctx);
        answer("yes", &env, &ctx);
        assert!(!ctx.has_buffer("a"));
        assert!(!ctx.has_buffer("two"));
        assert!(
            ctx.has_buffer("one"),
            "the marked ones, not the one under the cursor"
        );
    }

    #[test]
    fn shift_s_saves_the_buffer_under_the_cursor_when_none_is_marked() {
        let (ctx, env) = editor();
        let path = dirty_file("save", &env, &ctx);
        let file_buffer = path.file_name().unwrap().to_string_lossy().to_string();
        run("(buffer-list)", &env, &ctx);
        press_on(&file_buffer, 'S', &env, &ctx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before after");
        assert!(
            listing(&env, &ctx)
                .lines()
                .nth(line_of(&file_buffer, &env, &ctx) - 1)
                .is_some_and(|line| !line.starts_with("  *")),
            "no longer modified"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn with_short_answers_the_question_is_one_key() {
        let (ctx, env) = editor();
        run(
            r#"(setq use-short-answers t)
               (buffer-create "a" 'fundamental-mode)
               (with-current-buffer "a" (lambda () (insert "x")))"#,
            &env,
            &ctx,
        );
        run("(buffer-list)", &env, &ctx);
        press_on("a", 'D', &env, &ctx);
        assert!(ctx.has_buffer("a"));
        press('y', &env, &ctx);
        assert!(!ctx.has_buffer("a"));
    }

    #[test]
    fn a_name_longer_than_its_column_is_still_read_whole() {
        let (ctx, env) = editor();
        let long = "a-buffer-name-much-longer-than-the-column";
        run(
            &format!(r#"(buffer-create "{long}" 'fundamental-mode) (buffer-list)"#),
            &env,
            &ctx,
        );
        run(
            &format!("(goto-line {})", line_of(long, &env, &ctx)),
            &env,
            &ctx,
        );
        assert_eq!(
            run("(buffer-list--name-here)", &env, &ctx),
            LispExp::string(long.into())
        );
    }

    #[test]
    fn the_listing_is_not_modified_by_drawing_or_marking_itself() {
        let (ctx, env) = editor();
        spares(&env, &ctx);
        let modified = || run(r#"(buffer-modified-p "*Buffer List*")"#, &env, &ctx).is_truthy();
        assert!(!modified(), "drawn");
        press_on("one", 'm', &env, &ctx);
        press_on("two", 'd', &env, &ctx);
        assert!(!modified(), "marked");
        run("(buffer-list-refresh)", &env, &ctx);
        assert!(!modified(), "drawn again");
        assert!(
            listing(&env, &ctx)
                .lines()
                .any(|line| line.starts_with("    *Buffer List*")),
            "and its own line has no *:\n{}",
            listing(&env, &ctx)
        );
    }

    #[test]
    fn set_buffer_modified_p_says_and_changes_nothing_else() {
        let (ctx, env) = editor();
        run(r#"(insert "text")"#, &env, &ctx);
        assert!(run("(buffer-modified-p)", &env, &ctx).is_truthy());
        assert!(run("(set-buffer-modified-p nil)", &env, &ctx).is_nil());
        assert!(run("(buffer-modified-p)", &env, &ctx).is_nil());
        assert_eq!(
            run("(buffer-string)", &env, &ctx),
            LispExp::string("text".into())
        );
        run("(set-buffer-modified-p t)", &env, &ctx);
        assert!(run("(buffer-modified-p)", &env, &ctx).is_truthy());
    }
}
