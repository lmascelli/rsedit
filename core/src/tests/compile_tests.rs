//! Compilation mode: a line of real compiler output read as a place in a
//! file, going there, and the command read back off the buffer.
//!
//! Design: `doc/architettura-editor.typ, <mod-compile>`.
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
            include_str!("../../lisp/features/shell.lisp"),
            include_str!("../../lisp/features/compile.lisp"),
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

    /// The walk's mark goes on the line that names the place, also when the
    /// compiler writes other lines between its complaints.
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

    // ----------------------------------------------------------------
    // Where a build runs
    // ----------------------------------------------------------------

    /// Wait until every running command has finished, or give up.
    fn settle(ctx: &Ctx) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while ctx.shell_commands_running() > 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "a build did not finish within ten seconds"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn buffer_text(name: &str, ctx: &Ctx) -> String {
        ctx.with_buffer(name, |buf| buf.text.to_string())
            .unwrap_or_else(|| panic!("no buffer called {name}"))
    }

    fn latest_build(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text_of(&run("compilation--buffer", env, ctx)).expect("a build has run")
    }

    #[cfg(unix)]
    #[test]
    fn recompiling_runs_where_the_build_ran_not_where_the_editor_is_now() {
        let built_in = Sandbox::new("recompile-here");
        let moved_to = Sandbox::new("recompile-elsewhere");
        let physical = built_in
            .0
            .canonicalize()
            .expect("the sandbox exists")
            .display()
            .to_string();
        let (ctx, env) = editor();
        run(&format!(r#"(cd "{}")"#, built_in.0.display()), &env, &ctx);
        run(r#"(compile "pwd -P")"#, &env, &ctx);
        settle(&ctx);
        run(&format!(r#"(cd "{}")"#, moved_to.0.display()), &env, &ctx);

        let first = latest_build(&env, &ctx);
        run(&format!(r#"(switch-to-buffer "{first}")"#), &env, &ctx);
        run("(compilation-recompile)", &env, &ctx);
        settle(&ctx);

        let again = latest_build(&env, &ctx);
        assert!(
            buffer_text(&again, &ctx).contains(&format!("\n{physical}\n")),
            "the build should have run again in {physical}: {:?}",
            buffer_text(&again, &ctx)
        );
    }

    /// The places a build names are read where it ran: after `cd`, and from
    /// another buffer, `next-error` still opens the file the compiler meant.
    #[cfg(unix)]
    #[test]
    fn a_place_a_build_names_is_read_where_it_ran() {
        let built_in = Sandbox::new("places-here");
        let moved_to = Sandbox::new("places-elsewhere");
        let file = built_in.file("broken.c", "int a\nint b\n");
        moved_to.file("broken.c", "a decoy, in the wrong directory\n");
        let (ctx, env) = editor();
        run(&format!(r#"(cd "{}")"#, built_in.0.display()), &env, &ctx);
        run(
            r#"(compile "printf 'broken.c:2: error: expected a semicolon\n'")"#,
            &env,
            &ctx,
        );
        settle(&ctx);
        run(&format!(r#"(cd "{}")"#, moved_to.0.display()), &env, &ctx);
        run(r#"(switch-to-buffer "*scratch*")"#, &env, &ctx);

        run("(next-error)", &env, &ctx);

        assert_eq!(
            ctx.with_current_buffer(|buf| buf.file_path.clone()),
            Some(file)
        );
        assert_eq!(line_number(&env, &ctx), 2.0);
    }

    // ----------------------------------------------------------------
    // The places as findings
    // ----------------------------------------------------------------

    /// What `compile` has reported, one string per finding: file, start,
    /// severity, message and code.
    fn reported(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Vec<String> {
        run(
            r#"(mapcar (lambda (f)
                         (let ((at (nth 1 (memq :start f)))
                               (code (nth 1 (memq :code f))))
                           (concat (nth 1 (memq :file f))
                                   " " (number-to-string (car at))
                                   "." (number-to-string (cdr at))
                                   " " (symbol-name (nth 1 (memq :severity f)))
                                   " " (nth 1 (memq :message f))
                                   " [" (or code "") "]")))
                       (findings-of-producer 'compile))"#,
            env,
            ctx,
        )
        .iter()
        .map(|exp| text_of(&exp).expect("a string"))
        .collect()
    }

    /// A compilation buffer holding `output`, read in `directory`, reported.
    fn report(output: &str, directory: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Vec<String> {
        compilation_with(output, env, ctx);
        run(
            &format!(
                r#"(set-buffer-directory "{directory}")
                   (compilation--report-anew)
                   (compilation--report "*compile*")"#
            ),
            env,
            ctx,
        );
        reported(env, ctx)
    }

    #[test]
    fn each_place_is_reported_with_the_severity_its_line_says() {
        let sandbox = Sandbox::new("severity");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        let found = report(
            "$ make\n\
             a.c:3:5: error: expected ';'\n\
             a.c:7:1: warning: unused variable 'x'\n\
             a.c:2:10: note: declared here\n\
             a.c:9: fatal error: stdio.h: no such file\n\
             a.c:4:2: help: add a semicolon\n",
            &dir,
            &env,
            &ctx,
        );
        let file = format!("{dir}/a.c");
        assert_eq!(
            found,
            vec![
                format!("{file} 3.4 error expected ';' []"),
                format!("{file} 7.0 warning unused variable 'x' []"),
                format!("{file} 2.9 info declared here []"),
                format!("{file} 9.0 error stdio.h: no such file []"),
                format!("{file} 4.1 hint add a semicolon []"),
            ]
        );
    }

    #[test]
    fn rustcs_header_gives_the_place_after_it_its_severity_message_and_code() {
        let sandbox = Sandbox::new("rustc");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        let found = report(
            "$ cargo build\n\
             error[E0425]: cannot find value `x` in this scope\n\
             \x20--> src/main.rs:2:5\n\
             \x20 |\n\
             2 |     x\n\
             \x20 |     ^ not found in this scope\n\
             \n\
             warning: unused variable: `y`\n\
             \x20--> src/lib.rs:4:9\n\
             \x20 |\n\
             \x20 = note: `#[warn(unused_variables)]` on by default\n\
             \n\
             error: could not compile `app`\n",
            &dir,
            &env,
            &ctx,
        );
        assert_eq!(
            found,
            vec![
                format!("{dir}/src/lib.rs 4.8 warning unused variable: `y` []"),
                format!("{dir}/src/main.rs 2.4 error cannot find value `x` in this scope [E0425]"),
            ]
        );
    }

    #[test]
    fn a_header_speaks_only_for_the_place_right_after_it() {
        let sandbox = Sandbox::new("header-once");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        let found = report(
            "$ cargo build\n\
             warning: unused import\n\
             \x20--> a.rs:1:5\n\
             \x20--> b.rs:2:1\n",
            &dir,
            &env,
            &ctx,
        );
        assert_eq!(
            found,
            vec![
                format!("{dir}/a.rs 1.4 warning unused import []"),
                format!("{dir}/b.rs 2.0 error --> b.rs:2:1 []"),
            ]
        );
    }

    #[test]
    fn a_place_nothing_qualifies_takes_the_default_severity_and_its_line() {
        let sandbox = Sandbox::new("default");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        run("(setq compilation-default-severity 'warning)", &env, &ctx);
        let found = report(
            "$ python3 t.py\n\
             Traceback (most recent call last):\n\
             \x20 File \"t.py\", line 3, in <module>\n",
            &dir,
            &env,
            &ctx,
        );
        assert_eq!(
            found,
            vec![format!(
                "{dir}/t.py 3.0 warning File \"t.py\", line 3, in <module> []"
            )]
        );
    }

    #[test]
    fn the_severity_patterns_are_a_variable_anyone_can_add_to() {
        let sandbox = Sandbox::new("severity-pattern");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        run(
            r#"(setq compilation-severity-patterns
                     (cons '("LINT-([0-9]+) (.*)" hint 2 1)
                           compilation-severity-patterns))"#,
            &env,
            &ctx,
        );
        let found = report("$ lint\nx.c:1:1: LINT-7 tabs\n", &dir, &env, &ctx);
        assert_eq!(found, vec![format!("{dir}/x.c 1.0 hint tabs [7]")]);
    }

    #[test]
    fn two_names_for_one_file_are_one_set_of_findings() {
        let sandbox = Sandbox::new("one-file");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        // Reported file by file, `finding-set' replaces: were ./a.c another
        // file, its finding would have replaced the first.
        let found = report(
            &format!(
                "$ make\na.c:1:1: error: one\n./a.c:2:1: error: two\n{dir}/a.c:3:1: error: three\n"
            ),
            &dir,
            &env,
            &ctx,
        );
        assert_eq!(found.len(), 3, "got {found:?}");
    }

    #[test]
    fn the_walk_and_the_findings_name_the_same_places() {
        let sandbox = Sandbox::new("same-places");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        report(
            "$ make\nnoise\na.c:3:5: error: one\nb.c:4: warning: two\n",
            &dir,
            &env,
            &ctx,
        );
        let shape: Vec<String> = run(
            r#"(mapcar (lambda (e)
                         (concat (nth 1 e) " " (number-to-string (nth 2 e))
                                 " " (number-to-string (nth 3 e))
                                 " " (number-to-string (nth 8 e))))
                       (with-current-buffer "*compile*"
                                            (lambda () (compilation--entries))))"#,
            &env,
            &ctx,
        )
        .iter()
        .map(|exp| text_of(&exp).expect("a string"))
        .collect();
        assert_eq!(shape, vec!["a.c 3 4 3", "b.c 4 0 4"]);
    }

    /// Run the background and its callbacks until `done` holds, or fail.
    fn drive_until(
        what: &str,
        env: &Arc<Env<Ctx>>,
        ctx: &Ctx,
        mut done: impl FnMut(&Arc<Env<Ctx>>, &Ctx) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(env, ctx) {
            assert!(
                std::time::Instant::now() < deadline,
                "{what}: not within ten seconds"
            );
            // The tests' scheduler keeps time by hand: a quarter of a second,
            // the watch's sleep, at each turn.
            ctx.work()
                .run_for(ctx, std::time::Duration::from_millis(250));
            ctx.tick(env);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn watching(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        !run(r#"(member "compile-findings" (running-workers))"#, env, ctx).is_nil()
    }

    #[cfg(unix)]
    #[test]
    fn a_build_reports_its_places_read_where_it_ran_and_then_stops_watching() {
        let built_in = Sandbox::new("report-here");
        let moved_to = Sandbox::new("report-elsewhere");
        let file = built_in.file("broken.c", "int a\nint b\n");
        let (ctx, env) = editor();
        run(&format!(r#"(cd "{}")"#, built_in.0.display()), &env, &ctx);
        run(
            r#"(compile "printf 'broken.c:2:3: warning: no semicolon\n'")"#,
            &env,
            &ctx,
        );
        run(&format!(r#"(cd "{}")"#, moved_to.0.display()), &env, &ctx);
        assert!(watching(&env, &ctx), "a build is watched while it runs");

        drive_until("the watch ending", &env, &ctx, |env, ctx| {
            !watching(env, ctx)
        });

        assert_eq!(
            reported(&env, &ctx),
            vec![format!("{file} 2.2 warning no semicolon []")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_new_build_forgets_what_the_last_one_reported() {
        let sandbox = Sandbox::new("forget");
        let (ctx, env) = editor();
        run(&format!(r#"(cd "{}")"#, sandbox.0.display()), &env, &ctx);
        run(r#"(compile "printf 'a.c:1:1: error: old\n'")"#, &env, &ctx);
        drive_until("the first build", &env, &ctx, |env, ctx| {
            !watching(env, ctx)
        });
        assert_eq!(reported(&env, &ctx).len(), 1);

        run(r#"(compile "true")"#, &env, &ctx);

        assert_eq!(reported(&env, &ctx), Vec::<String>::new());
        drive_until("the second build", &env, &ctx, |env, ctx| {
            !watching(env, ctx)
        });
        assert_eq!(reported(&env, &ctx), Vec::<String>::new());
    }

    #[test]
    fn closing_the_output_stops_the_watch_and_keeps_the_findings() {
        let sandbox = Sandbox::new("closed");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        report("$ make\na.c:1:1: error: kept\n", &dir, &env, &ctx);
        run(
            r#"(compilation--watch) (kill-buffer "*compile*")"#,
            &env,
            &ctx,
        );

        drive_until("the watch ending", &env, &ctx, |env, ctx| {
            !watching(env, ctx)
        });

        assert_eq!(reported(&env, &ctx).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_build_still_running_has_already_reported_what_it_printed() {
        let sandbox = Sandbox::new("running");
        let (ctx, env) = editor();
        run(&format!(r#"(cd "{}")"#, sandbox.0.display()), &env, &ctx);
        run(
            r#"(compile "printf 'a.c:1:1: error: early\n'; sleep 5")"#,
            &env,
            &ctx,
        );

        drive_until("the first finding", &env, &ctx, |env, ctx| {
            !reported(env, ctx).is_empty()
        });

        assert!(watching(&env, &ctx), "the build has not ended");
        let build = latest_build(&env, &ctx);
        run(&format!(r#"(kill-buffer "{build}")"#), &env, &ctx);
        settle(&ctx);
    }

    fn append(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!(
                r#"(with-current-buffer "*compile*"
                     (lambda () (goto-char (point-max)) (insert "{text}")))
                   (compilation--report "*compile*")"#
            ),
            env,
            ctx,
        );
    }

    #[test]
    fn a_report_reads_only_whole_lines_and_keeps_what_came_before() {
        let sandbox = Sandbox::new("growing");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        report("$ make\na.c:1:1: error: one\n", &dir, &env, &ctx);

        append("warning: unused\n --> b.rs:2:1\na.c:2:1: err", &env, &ctx);
        assert_eq!(
            reported(&env, &ctx),
            vec![
                format!("{dir}/a.c 1.0 error one []"),
                format!("{dir}/b.rs 2.0 warning unused []"),
            ],
            "a line not yet whole waits"
        );

        append("or: two\nnote: here\n", &env, &ctx);
        append(" --> c.rs:3:1\n", &env, &ctx);
        assert_eq!(
            reported(&env, &ctx),
            vec![
                format!("{dir}/a.c 1.0 error one []"),
                format!("{dir}/a.c 2.0 error two []"),
                format!("{dir}/b.rs 2.0 warning unused []"),
                format!("{dir}/c.rs 3.0 info here []"),
            ]
        );
    }

    #[test]
    fn a_long_output_is_reported_a_part_at_a_time() {
        let sandbox = Sandbox::new("parts");
        let dir = sandbox.0.display().to_string();
        let (ctx, env) = editor();
        run("(setq compilation--report-lines 2)", &env, &ctx);
        let found = report(
            "$ make\na.c:1:1: error: naïve ü\nnoise ü\nb.c:2:1: error: two\n",
            &dir,
            &env,
            &ctx,
        );
        assert_eq!(found, vec![format!("{dir}/a.c 1.0 error naïve ü []")]);
        assert!(!run(r#"(compilation--report "*compile*")"#, &env, &ctx).is_nil());
        assert_eq!(
            reported(&env, &ctx),
            vec![
                format!("{dir}/a.c 1.0 error naïve ü []"),
                format!("{dir}/b.c 2.0 error two []"),
            ]
        );
    }
}
