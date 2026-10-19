//! Compilation mode: reading a line of output as a place in a file, and going
//! there.
//!
//! The parsing is most of what can go wrong, so most of this is a table of
//! real compiler output. The rest is the navigation, and the fact that the
//! command is read back off the buffer rather than kept beside it.
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
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/shell.lisp"),
            include_str!("../../lisp/compile.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    fn text_of(exp: &LispExp<Ctx>) -> Option<String> {
        match exp {
            LispExp::String(s) => Some(s.to_string()),
            other if other.is_nil() => None,
            other => panic!("expected a string or nil, got {other:?}"),
        }
    }

    /// `(compilation--parse LINE)` as (file, line, column).
    fn parse(
        line: &str,
        env: &Arc<Env<Ctx>>,
        ctx: &Ctx,
    ) -> Option<(String, String, Option<String>)> {
        let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
        let answer = run(&format!(r#"(compilation--parse "{escaped}")"#), env, ctx);
        if answer.is_nil() {
            return None;
        }
        let parts: Vec<LispExp<Ctx>> = answer.iter().collect();
        Some((
            text_of(&parts[0]).expect("a file"),
            text_of(&parts[1]).expect("a line"),
            text_of(&parts[2]),
        ))
    }

    // ----------------------------------------------------------------
    // Reading a line
    // ----------------------------------------------------------------

    #[test]
    fn gcc_and_clang_style_with_a_column() {
        let (ctx, env) = editor();
        assert_eq!(
            parse("src/main.c:42:17: error: expected ';'", &env, &ctx),
            Some(("src/main.c".into(), "42".into(), Some("17".into())))
        );
    }

    #[test]
    fn a_format_with_no_column() {
        let (ctx, env) = editor();
        assert_eq!(
            parse("Makefile:12: *** missing separator.  Stop.", &env, &ctx),
            Some(("Makefile".into(), "12".into(), None))
        );
    }

    #[test]
    fn rustcs_arrow_line_is_where_the_path_usually_is() {
        // rustc puts the message on one line and the location on the next,
        // indented, behind an arrow. Without this pattern the useful line is
        // the one you cannot press Return on.
        let (ctx, env) = editor();
        assert_eq!(
            parse("  --> core/src/editor.rs:1847:9", &env, &ctx),
            Some(("core/src/editor.rs".into(), "1847".into(), Some("9".into())))
        );
    }

    #[test]
    fn a_python_traceback_line() {
        let (ctx, env) = editor();
        assert_eq!(
            parse(r#"  File "script.py", line 7, in <module>"#, &env, &ctx),
            Some(("script.py".into(), "7".into(), None))
        );
    }

    #[test]
    fn an_absolute_path_is_read_whole() {
        let (ctx, env) = editor();
        assert_eq!(
            parse("/home/user/p/src/lib.rs:3:1: warning", &env, &ctx),
            Some((
                "/home/user/p/src/lib.rs".into(),
                "3".into(),
                Some("1".into())
            ))
        );
    }

    #[test]
    fn a_line_that_names_nothing_parses_as_nothing() {
        let (ctx, env) = editor();
        assert_eq!(parse("warning: 3 warnings emitted", &env, &ctx), None);
        assert_eq!(parse("", &env, &ctx), None);
        assert_eq!(parse("$ cargo build", &env, &ctx), None);
    }

    #[test]
    fn the_exit_trailer_is_not_a_location() {
        // It contains no colon-number, but it is worth pinning: `n` walking
        // onto the trailer and calling it an error would be maddening.
        let (ctx, env) = editor();
        assert_eq!(parse("--- exited 101 ---", &env, &ctx), None);
    }

    #[test]
    fn the_patterns_are_a_variable_anyone_can_add_to() {
        // Adding a compiler is adding a line.
        let (ctx, env) = editor();
        assert_eq!(parse("oddball at [zig.zig|55]", &env, &ctx), None);
        run(
            r#"(setq compilation-patterns
                 (cons '("at \[([^|]+)\|([0-9]+)\]" 1 2 nil) compilation-patterns))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            parse("oddball at [zig.zig|55]", &env, &ctx),
            Some(("zig.zig".into(), "55".into(), None))
        );
    }

    #[test]
    fn the_first_matching_pattern_decides() {
        // `path:line:col` and `path:line` both match a line with a column;
        // the more specific one comes first, so the column is not lost.
        let (ctx, env) = editor();
        let (_, _, column) = parse("a.c:1:2: oops", &env, &ctx).expect("a location");
        assert_eq!(column, Some("2".into()));
    }

    // ----------------------------------------------------------------
    // Walking
    // ----------------------------------------------------------------

    /// A compilation buffer holding `output`, with point at the top.
    fn compilation_with(output: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        let escaped = output.replace('\\', "\\\\").replace('"', "\\\"");
        run(
            &format!(
                r#"(buffer-create "*compile*" 'compilation-mode)
                   (switch-to-buffer "*compile*")
                   (insert "{escaped}")
                   (goto-char 0)
                   (setq compilation--buffer "*compile*")"#
            ),
            env,
            ctx,
        );
    }

    fn line_number(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> f64 {
        match run("(line-number-at-point)", env, ctx) {
            LispExp::Number(n) => n,
            other => panic!("expected a number, got {other:?}"),
        }
    }

    #[test]
    fn n_moves_to_the_next_line_that_names_a_place() {
        let (ctx, env) = editor();
        compilation_with(
            "$ cargo build\nnoise here\nsrc/a.rs:1:1: bad\nmore noise\nsrc/b.rs:2:1: worse\n",
            &env,
            &ctx,
        );
        run("(compilation-next)", &env, &ctx);
        assert_eq!(line_number(&env, &ctx), 3.0);
        run("(compilation-next)", &env, &ctx);
        assert_eq!(line_number(&env, &ctx), 5.0);
    }

    #[test]
    fn n_at_the_last_one_stays_put() {
        let (ctx, env) = editor();
        compilation_with("$ make\nsrc/a.rs:1:1: bad\ntail\n", &env, &ctx);
        run("(compilation-next)", &env, &ctx);
        assert_eq!(line_number(&env, &ctx), 2.0);
        run("(compilation-next)", &env, &ctx);
        assert_eq!(line_number(&env, &ctx), 2.0, "point should not wander off");
    }

    #[test]
    fn p_walks_back() {
        let (ctx, env) = editor();
        compilation_with(
            "$ make\nsrc/a.rs:1:1: bad\nx\nsrc/b.rs:2:1: worse\n",
            &env,
            &ctx,
        );
        run("(goto-line 4) (compilation-previous)", &env, &ctx);
        assert_eq!(line_number(&env, &ctx), 2.0);
    }

    // ----------------------------------------------------------------
    // The command, read back off the buffer
    // ----------------------------------------------------------------

    #[test]
    fn the_command_is_read_off_the_first_line() {
        // `shell-command-start` writes it there, so the buffer says what made
        // it and `g` needs no table mapping buffers to commands.
        let (ctx, env) = editor();
        compilation_with("$ cargo build --release\nsrc/a.rs:1:1: bad\n", &env, &ctx);
        assert_eq!(
            text_of(&run("(compilation--command-here)", &env, &ctx)),
            Some("cargo build --release".into())
        );
    }

    #[test]
    fn reading_the_command_does_not_move_point() {
        let (ctx, env) = editor();
        compilation_with("$ make\nsrc/a.rs:1:1: bad\nx\n", &env, &ctx);
        run("(goto-line 3)", &env, &ctx);
        run("(compilation--command-here)", &env, &ctx);
        assert_eq!(line_number(&env, &ctx), 3.0);
    }

    #[test]
    fn a_buffer_that_does_not_say_what_it_ran_answers_nil() {
        let (ctx, env) = editor();
        compilation_with("no dollar here\n", &env, &ctx);
        assert!(run("(compilation--command-here)", &env, &ctx).is_nil());
    }

    // ----------------------------------------------------------------
    // Actually going there
    // ----------------------------------------------------------------

    struct Sandbox(std::path::PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "rsedit-compile-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("sandbox");
            Sandbox(root)
        }
        fn file(&self, relative: &str, contents: &str) -> String {
            let path = self.0.join(relative);
            std::fs::write(&path, contents).expect("sandbox file");
            path.display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn return_opens_the_file_at_the_line_it_named() {
        let sandbox = Sandbox::new("goto");
        let path = sandbox.file("thing.rs", "one\ntwo\nthree\nfour\n");
        let (ctx, env) = editor();
        compilation_with(
            &format!("$ make\n{path}:3:2: something is wrong\n"),
            &env,
            &ctx,
        );
        run("(goto-line 2) (compilation-goto)", &env, &ctx);

        // The file is open in the other window, at line 3, one column in.
        run(r#"(switch-to-buffer "thing.rs")"#, &env, &ctx);
        assert_eq!(line_number(&env, &ctx), 3.0);
    }

    #[test]
    fn the_column_is_one_based_the_way_compilers_count() {
        // `foo.rs:3:1` means the first character, not the second.
        let sandbox = Sandbox::new("column");
        let path = sandbox.file("thing.rs", "one\ntwo\nthree\n");
        let (ctx, env) = editor();
        compilation_with(&format!("$ make\n{path}:3:1: here\n"), &env, &ctx);
        run("(goto-line 2) (compilation-goto)", &env, &ctx);
        run(r#"(switch-to-buffer "thing.rs")"#, &env, &ctx);
        assert_eq!(
            run("(current-column)", &env, &ctx),
            LispExp::number(0.0),
            "column 1 is offset 0"
        );
    }

    #[test]
    fn the_compilation_output_stays_visible_beside_the_file() {
        // The point of having it in a buffer: you read the next complaint
        // without going back for it.
        let sandbox = Sandbox::new("beside");
        let path = sandbox.file("thing.rs", "one\ntwo\nthree\n");
        let (ctx, env) = editor();
        compilation_with(&format!("$ make\n{path}:2:1: here\n"), &env, &ctx);
        let before = ctx.window_count();
        run("(goto-line 2) (compilation-goto)", &env, &ctx);
        assert!(ctx.window_count() > before, "the file opened beside it");
        let shown: Vec<String> = ctx
            .snapshot(&env, 80, 24)
            .views
            .iter()
            .map(|v| v.buffer_name.clone())
            .collect();
        assert!(shown.contains(&"*compile*".to_string()), "got {shown:?}");
        assert!(shown.contains(&"thing.rs".to_string()), "got {shown:?}");
    }

    #[test]
    fn walking_twice_reuses_the_same_window() {
        // Otherwise a list of forty errors slices the frame into forty.
        let sandbox = Sandbox::new("reuse");
        let one = sandbox.file("one.rs", "a\nb\nc\n");
        let two = sandbox.file("two.rs", "a\nb\nc\n");
        let (ctx, env) = editor();
        compilation_with(
            &format!("$ make\n{one}:2:1: first\n{two}:3:1: second\n"),
            &env,
            &ctx,
        );
        run("(goto-line 2) (compilation-goto)", &env, &ctx);
        let after_first = ctx.window_count();
        run(r#"(switch-to-buffer "*compile*")"#, &env, &ctx);
        run("(goto-line 3) (compilation-goto)", &env, &ctx);
        assert_eq!(ctx.window_count(), after_first);
    }

    #[test]
    fn a_file_that_is_not_there_says_so_rather_than_opening_something_else() {
        let (ctx, env) = editor();
        compilation_with("$ make\nno/such/file.rs:3:1: bad\n", &env, &ctx);
        run("(goto-line 2) (compilation-goto)", &env, &ctx);
        // Still in the compilation buffer; nothing was opened.
        assert!(!ctx.has_buffer("file.rs"));
    }

    #[test]
    fn return_on_a_line_naming_nothing_says_so() {
        let (ctx, env) = editor();
        compilation_with("$ make\njust some noise\n", &env, &ctx);
        let before = ctx.window_count();
        run("(goto-line 2) (compilation-goto)", &env, &ctx);
        assert_eq!(ctx.window_count(), before);
    }

    // ----------------------------------------------------------------
    // Marking
    // ----------------------------------------------------------------

    /// How many marks are on LINE of the compilation buffer.
    fn marks_on(line: usize, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> usize {
        run(
            &format!(
                r#"(with-current-buffer "*compile*"
                     (lambda () (goto-line {line}) (overlays-at (point))))"#
            ),
            env,
            ctx,
        )
        .iter()
        .count()
    }

    #[test]
    fn visiting_marks_the_line_in_the_compilation_buffer() {
        let (ctx, env) = editor();
        compilation_with("$ make\nsrc/a.rs:1:1: bad\n", &env, &ctx);
        run("(goto-line 2) (compilation-goto)", &env, &ctx);
        assert_eq!(marks_on(2, &env, &ctx), 1);
    }

    #[test]
    fn only_one_line_is_ever_marked() {
        // A list of forty errors is only legible if exactly one is marked --
        // which is what the category is for.
        let (ctx, env) = editor();
        compilation_with(
            "$ make\nsrc/a.rs:1:1: bad\nsrc/b.rs:2:1: worse\n",
            &env,
            &ctx,
        );
        run("(goto-line 2) (compilation-goto)", &env, &ctx);
        run(
            r#"(switch-to-buffer "*compile*") (goto-line 3) (compilation-goto)"#,
            &env,
            &ctx,
        );
        assert_eq!(marks_on(2, &env, &ctx), 0);
        assert_eq!(marks_on(3, &env, &ctx), 1);
    }

    /// The walk's mark goes on the line that names the place. It went on the
    /// line one past the entry's index -- right for a search's listing, with
    /// its header line, and wrong for output where the compiler writes other
    /// lines between its complaints.
    #[test]
    fn m_g_n_marks_the_line_that_names_the_place() {
        let (ctx, env) = editor();
        compilation_with(
            "$ cargo build\n   Compiling foo\nerror: types\n --> src/a.rs:3:1\n",
            &env,
            &ctx,
        );
        run(
            r#"(compilation--rebuild "*compile*") (next-error)"#,
            &env,
            &ctx,
        );
        assert_eq!(marks_on(4, &env, &ctx), 1);
        assert_eq!(marks_on(2, &env, &ctx), 0);
    }

    /// Return opens a place through the walk, so M-g n goes on from it. It
    /// went on from wherever the walk had last been.
    #[test]
    fn return_moves_where_m_g_n_goes_on_from() {
        let (ctx, env) = editor();
        compilation_with(
            "$ make\nsrc/a.rs:1:1: one\nsrc/b.rs:2:1: two\nsrc/c.rs:3:1: three\n",
            &env,
            &ctx,
        );
        run("(goto-line 3) (compilation-goto)", &env, &ctx);
        run("(next-error)", &env, &ctx);
        assert_eq!(
            run(r#"(nth 5 (results-state "*compile*"))"#, &env, &ctx),
            LispExp::number(2.0),
            "the third place follows the second"
        );
    }

    /// The command on the first line is not a place, even when it names one.
    #[test]
    fn the_command_line_names_no_place() {
        let (ctx, env) = editor();
        compilation_with("$ echo src/main.rs:1:1\nsrc/main.rs:1:1\n", &env, &ctx);
        assert!(run("(goto-line 1) (compilation--location-here)", &env, &ctx).is_nil());
        assert_eq!(
            run("(length (compilation--entries))", &env, &ctx),
            LispExp::number(1.0)
        );
        run("(goto-line 1) (compilation-next)", &env, &ctx);
        assert_eq!(line_number(&env, &ctx), 2.0);
    }

    /// One strip for every build: running one again shows its output where
    /// the last one was, rather than stacking another strip, and the focus
    /// stays where it was.
    #[test]
    fn a_second_build_reuses_the_strip() {
        let (ctx, env) = editor();
        run(r#"(buffer-create "one") (buffer-create "two")"#, &env, &ctx);
        run(r#"(compilation--show "one")"#, &env, &ctx);
        let after_one = ctx.window_count();
        run(r#"(compilation--show "two")"#, &env, &ctx);
        assert_eq!(ctx.window_count(), after_one);
        assert_eq!(
            run("(window-buffer)", &env, &ctx),
            LispExp::string("*scratch*".into())
        );
        let shown: Vec<String> = ctx
            .snapshot(&env, 80, 24)
            .views
            .iter()
            .map(|v| v.buffer_name.clone())
            .collect();
        assert!(shown.contains(&"two".to_string()), "got {shown:?}");
        assert!(!shown.contains(&"one".to_string()), "got {shown:?}");
    }

    /// Return in the output opens the file in the window the build was started
    /// from, and leaves the output where it was. It split the output's strip,
    /// leaving the output a line tall.
    #[test]
    fn return_opens_the_file_in_the_window_the_build_came_from() {
        let sandbox = Sandbox::new("origin");
        let path = sandbox.file("thing.rs", "one\ntwo\nthree\n");
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "*compile*" 'compilation-mode)"#,
            &env,
            &ctx,
        );
        run(
            &format!(
                r#"(with-current-buffer "*compile*"
                     (lambda () (insert "$ make\n{path}:2:1: here\n")))"#
            ),
            &env,
            &ctx,
        );
        run(r#"(compilation--show "*compile*")"#, &env, &ctx);
        let windows = ctx.window_count();
        run(
            "(other-window 1) (goto-line 2) (compilation-goto)",
            &env,
            &ctx,
        );
        assert_eq!(ctx.window_count(), windows, "no window was split");
        assert_eq!(
            run("(list (window-buffer) (line-number-at-point))", &env, &ctx),
            run(r#"(list "thing.rs" 2)"#, &env, &ctx)
        );
        let shown: Vec<String> = ctx
            .snapshot(&env, 80, 24)
            .views
            .iter()
            .map(|v| v.buffer_name.clone())
            .collect();
        assert!(shown.contains(&"*compile*".to_string()), "got {shown:?}");
    }

    #[test]
    fn next_error_with_nothing_compiled_says_so_rather_than_failing() {
        let (ctx, env) = editor();
        run("(setq compilation--buffer nil)", &env, &ctx);
        run("(next-error)", &env, &ctx);
        // Reaching here is the test: it must not signal.
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
    }
}
