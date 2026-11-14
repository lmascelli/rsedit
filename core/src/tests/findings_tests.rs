//! The findings: each producer's findings in a file replaced as a whole and
//! asked for by file, by producer and by directory; the markers that make
//! them follow the text of an open file, and `:stale` once that text has
//! changed; and what `finding-set` refuses.
//!
//! Design: `doc/architettura-editor.typ, <segnalazioni>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::managers::{Finding, Findings, Placed, Severity};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// A directory of this test's own, removed when it goes.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsedit-findings-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn file(&self, name: &str, contents: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("writing a file");
            path.display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
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

    /// Returns the value after KEY in the property list PLIST.
    fn get(plist: &LispExp<Ctx>, key: &str) -> LispExp<Ctx> {
        let items: Vec<LispExp<Ctx>> = plist.iter().collect();
        items
            .chunks(2)
            .find(|pair| matches!(&pair[0], LispExp::Symbol(name) if name.as_str() == key))
            .map(|pair| pair[1].clone())
            .unwrap_or_else(|| panic!("no {key} in {plist:?}"))
    }

    /// Returns the `(LINE . COLUMN)` after KEY in PLIST.
    fn place(plist: &LispExp<Ctx>, key: &str) -> (f64, f64) {
        match get(plist, key) {
            LispExp::Cons(cell) => match (&cell.car, &cell.cdr) {
                (LispExp::Number(line), LispExp::Number(column)) => (*line, *column),
                _ => panic!("{key} is not two numbers"),
            },
            other => panic!("{key} is {other:?}"),
        }
    }

    fn text(exp: &LispExp<Ctx>) -> String {
        match exp {
            LispExp::String(text) | LispExp::Symbol(text) => text.to_string(),
            other => panic!("not a name: {other:?}"),
        }
    }

    fn list(exp: &LispExp<Ctx>) -> Vec<LispExp<Ctx>> {
        exp.iter().collect()
    }

    fn finding(line: usize, message: &str) -> Finding {
        Finding {
            start: (line, 0),
            end: (line, 1),
            severity: Severity::Warning,
            message: message.into(),
            code: None,
        }
    }

    fn messages(findings: &Findings, file: &str) -> Vec<String> {
        findings
            .for_file(file)
            .into_iter()
            .flat_map(|found| found.set.findings)
            .map(|finding| finding.message)
            .collect()
    }

    // -- The compartment -----------------------------------------------------

    #[test]
    fn a_replacement_takes_the_place_of_the_producers_findings_in_that_file_only() {
        let mut findings = Findings::default();
        findings.replace("lint", "/p/a.c", vec![finding(1, "old")]);
        findings.replace("lint", "/p/b.c", vec![finding(1, "other file")]);
        findings.replace("build", "/p/a.c", vec![finding(2, "other producer")]);
        findings.replace(
            "lint",
            "/p/a.c",
            vec![finding(3, "new"), finding(4, "newer")],
        );
        assert_eq!(
            messages(&findings, "/p/a.c"),
            ["other producer", "new", "newer"]
        );
        assert_eq!(messages(&findings, "/p/b.c"), ["other file"]);
    }

    #[test]
    fn an_empty_replacement_forgets_the_file_for_that_producer() {
        let mut findings = Findings::default();
        findings.replace("lint", "/p/a.c", vec![finding(1, "gone")]);
        findings.replace("lint", "/p/a.c", Vec::new());
        assert!(findings.for_file("/p/a.c").is_empty());
        assert!(findings.of_producer("lint").is_empty());
    }

    #[test]
    fn a_producers_findings_come_by_file() {
        let mut findings = Findings::default();
        findings.replace("lint", "/p/b.c", vec![finding(1, "b")]);
        findings.replace("lint", "/p/a.c", vec![finding(1, "a")]);
        findings.replace("build", "/p/c.c", vec![finding(1, "c")]);
        let files: Vec<String> = findings
            .of_producer("lint")
            .into_iter()
            .map(|found| found.file)
            .collect();
        assert_eq!(files, ["/p/a.c", "/p/b.c"]);
    }

    #[test]
    fn a_directory_holds_the_files_below_it_by_whole_names() {
        let mut findings = Findings::default();
        for file in ["/p/a.c", "/p/sub/b.c", "/pq/c.c", "/p"] {
            findings.replace("lint", file, vec![finding(1, file)]);
        }
        let files = |directory: &str| -> Vec<String> {
            findings
                .under(directory)
                .into_iter()
                .map(|found| found.file)
                .collect()
        };
        assert_eq!(files("/p"), ["/p/a.c", "/p/sub/b.c"]);
        assert_eq!(files("/p/"), ["/p/a.c", "/p/sub/b.c"]);
        assert_eq!(files("/p/sub"), ["/p/sub/b.c"]);
        assert_eq!(files("/"), ["/p", "/p/a.c", "/p/sub/b.c", "/pq/c.c"]);
    }

    #[test]
    fn clearing_a_producer_forgets_every_file_of_it_and_no_other_producers() {
        let mut findings = Findings::default();
        findings.replace("lint", "/p/a.c", vec![finding(1, "a")]);
        findings.replace("lint", "/p/b.c", vec![finding(1, "b")]);
        findings.replace("build", "/p/a.c", vec![finding(1, "kept")]);
        findings.clear("lint", None);
        assert!(findings.of_producer("lint").is_empty());
        assert_eq!(messages(&findings, "/p/a.c"), ["kept"]);
        assert!(findings.for_file("/p/b.c").is_empty());
    }

    #[test]
    fn markers_made_for_a_set_since_replaced_are_handed_back() {
        let mut findings = Findings::default();
        let first = findings.replace("lint", "/p/a.c", vec![finding(1, "a")]);
        findings.replace("lint", "/p/a.c", vec![finding(2, "b")]);
        let placed = Placed {
            markers: vec![(1, 2)],
            version: 0,
        };
        assert_eq!(
            findings.place("lint", "/p/a.c", first.generation.unwrap(), placed.clone()),
            Err(placed)
        );
    }

    // -- From Lisp -----------------------------------------------------------

    #[test]
    fn a_finding_comes_back_with_everything_it_was_given() {
        let sandbox = Sandbox::new("round-trip");
        let path = sandbox.file("a.c", "int x;\n");
        let (ctx, env) = editor();
        run(
            &format!(
                r#"(finding-set 'lint "{path}"
                     '((:start (1 . 4) :end (1 . 5) :severity warning
                        :message "unused" :code 42)))"#
            ),
            &env,
            &ctx,
        );
        let found = list(&run(
            &format!(r#"(findings-for-file "{path}")"#),
            &env,
            &ctx,
        ));
        assert_eq!(found.len(), 1);
        let one = &found[0];
        assert_eq!(text(&get(one, ":file")), path);
        assert_eq!(text(&get(one, ":producer")), "lint");
        assert!(matches!(get(one, ":producer"), LispExp::Symbol(_)));
        assert_eq!(place(one, ":start"), (1.0, 4.0));
        assert_eq!(place(one, ":end"), (1.0, 5.0));
        assert_eq!(text(&get(one, ":severity")), "warning");
        assert_eq!(text(&get(one, ":message")), "unused");
        assert_eq!(text(&get(one, ":code")), "42");
        assert!(get(one, ":stale").is_nil());
    }

    #[test]
    fn an_end_not_given_is_the_start_and_a_code_not_given_is_nil() {
        let (ctx, env) = editor();
        run(
            r#"(finding-set "lint" "/nowhere/a.c" '((:start (2 . 0) :severity hint :message "m")))"#,
            &env,
            &ctx,
        );
        let found = list(&run(r#"(findings-of-producer 'lint)"#, &env, &ctx));
        assert_eq!(place(&found[0], ":end"), (2.0, 0.0));
        assert!(get(&found[0], ":code").is_nil());
    }

    #[test]
    fn a_relative_file_is_read_in_the_default_directory() {
        let (ctx, env) = editor();
        run(
            r#"(cd "/tmp") (finding-set 'lint "a.c" '((:start (1 . 0) :severity info :message "m")))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            list(&run(r#"(findings-for-file "/tmp/a.c")"#, &env, &ctx)).len(),
            1
        );
    }

    #[test]
    fn findings_under_a_directory_come_from_every_producer() {
        let (ctx, env) = editor();
        run(
            r#"(finding-set 'lint "/p/a.c" '((:start (1 . 0) :severity error :message "a")))
               (finding-set 'build "/p/sub/b.c" '((:start (1 . 0) :severity error :message "b")))
               (finding-set 'lint "/pq/c.c" '((:start (1 . 0) :severity error :message "c")))"#,
            &env,
            &ctx,
        );
        let found = list(&run(r#"(findings-under "/p")"#, &env, &ctx));
        let messages: Vec<String> = found.iter().map(|f| text(&get(f, ":message"))).collect();
        assert_eq!(messages, ["a", "b"]);
    }

    #[test]
    fn a_property_is_read_with_memq_as_the_page_says() {
        let (ctx, env) = editor();
        let messages = run(
            r#"(finding-set 'lint "/p/a.c"
                 '((:start (1 . 0) :severity error :message "kept")
                   (:start (2 . 0) :severity hint :message "skipped")))
               (let ((out nil))
                 (dolist (f (findings-for-file "/p/a.c"))
                   (when (eq (nth 1 (memq :severity f)) 'error)
                     (setq out (cons (nth 1 (memq :message f)) out))))
                 out)"#,
            &env,
            &ctx,
        );
        let messages: Vec<String> = list(&messages).iter().map(text).collect();
        assert_eq!(messages, ["kept"]);
    }

    #[test]
    fn finding_clear_forgets_one_file_or_all_of_a_producers() {
        let (ctx, env) = editor();
        run(
            r#"(finding-set 'lint "/p/a.c" '((:start (1 . 0) :severity error :message "a")))
               (finding-set 'lint "/p/b.c" '((:start (1 . 0) :severity error :message "b")))
               (finding-clear 'lint "/p/a.c")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            list(&run("(findings-of-producer 'lint)", &env, &ctx)).len(),
            1
        );
        run("(finding-clear 'lint)", &env, &ctx);
        assert!(run("(findings-of-producer 'lint)", &env, &ctx).is_nil());
    }

    #[test]
    fn an_entry_without_a_known_severity_or_a_message_is_refused() {
        let (ctx, env) = editor();
        for entry in [
            r#"(:start (1 . 0) :severity fatal :message "m")"#,
            r#"(:start (1 . 0) :message "m")"#,
            r#"(:start (1 . 0) :severity error)"#,
            r#"(:severity error :message "m")"#,
            r#"(:start (1 . -1) :severity error :message "m")"#,
        ] {
            let outcome = try_run(
                &format!(r#"(finding-set 'lint "/p/a.c" '({entry}))"#),
                &env,
                &ctx,
            );
            assert!(
                matches!(outcome, Err(EvalError::WrongArgumentType { .. })),
                "{entry} gave {outcome:?}"
            );
        }
        // Nothing of a refused list is kept.
        assert!(run(r#"(findings-for-file "/p/a.c")"#, &env, &ctx).is_nil());
    }

    // -- In an open file -----------------------------------------------------

    /// Opens PATH, and returns its buffer's name.
    fn open(path: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text(&run(&format!(r#"(find-file "{path}")"#), env, ctx))
    }

    fn markers(ctx: &Ctx, name: &str) -> usize {
        ctx.with_buffer(name, |buf| buf.markers.len()).unwrap_or(0)
    }

    const ONE_FINDING: &str = r#"'((:start (2 . 4) :end (2 . 7) :severity error :message "m"))"#;

    #[test]
    fn in_an_open_file_a_finding_follows_the_lines_added_above_it_and_turns_stale() {
        let sandbox = Sandbox::new("follows");
        let path = sandbox.file("a.c", "one line\nint count;\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        run(
            &format!(r#"(finding-set 'lint "{path}" {ONE_FINDING})"#),
            &env,
            &ctx,
        );
        assert_eq!(markers(&ctx, &name), 2);
        run(r#"(goto-char 0) (insert "zero line\n")"#, &env, &ctx);
        let found = list(&run(
            &format!(r#"(findings-for-file "{path}")"#),
            &env,
            &ctx,
        ));
        assert_eq!(place(&found[0], ":start"), (3.0, 4.0));
        assert_eq!(place(&found[0], ":end"), (3.0, 7.0));
        assert!(get(&found[0], ":stale").is_truthy());
    }

    #[test]
    fn a_new_replacement_is_fresh_and_takes_the_old_markers_away() {
        let sandbox = Sandbox::new("fresh");
        let path = sandbox.file("a.c", "one line\nint count;\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        run(
            &format!(r#"(finding-set 'lint "{path}" {ONE_FINDING})"#),
            &env,
            &ctx,
        );
        run(r#"(goto-char 0) (insert "zero line\n")"#, &env, &ctx);
        run(
            &format!(r#"(finding-set 'lint "{path}" {ONE_FINDING})"#),
            &env,
            &ctx,
        );
        assert_eq!(markers(&ctx, &name), 2);
        let found = list(&run(
            &format!(r#"(findings-for-file "{path}")"#),
            &env,
            &ctx,
        ));
        assert_eq!(place(&found[0], ":start"), (2.0, 4.0));
        assert!(get(&found[0], ":stale").is_nil());
        run(&format!(r#"(finding-clear 'lint "{path}")"#), &env, &ctx);
        assert_eq!(markers(&ctx, &name), 0);
    }

    #[test]
    fn a_file_opened_after_its_findings_came_gets_their_markers() {
        let sandbox = Sandbox::new("opened-after");
        let path = sandbox.file("a.c", "one line\nint count;\n");
        let (ctx, env) = editor();
        run(
            &format!(r#"(finding-set 'lint "{path}" {ONE_FINDING})"#),
            &env,
            &ctx,
        );
        let name = open(&path, &env, &ctx);
        assert_eq!(markers(&ctx, &name), 2);
        run(r#"(goto-char 0) (insert "zero line\n")"#, &env, &ctx);
        let found = list(&run(
            &format!(r#"(findings-for-file "{path}")"#),
            &env,
            &ctx,
        ));
        assert_eq!(place(&found[0], ":start"), (3.0, 4.0));
    }

    #[test]
    fn a_closed_file_puts_its_findings_back_where_the_producer_said_and_keeps_them_stale() {
        let sandbox = Sandbox::new("closed");
        let path = sandbox.file("a.c", "one line\nint count;\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        run(
            &format!(r#"(finding-set 'lint "{path}" {ONE_FINDING})"#),
            &env,
            &ctx,
        );
        run(r#"(goto-char 0) (insert "zero line\n")"#, &env, &ctx);
        run(
            &format!(r#"(kill-buffer-without-saving "{name}")"#),
            &env,
            &ctx,
        );
        let found = list(&run(
            &format!(r#"(findings-for-file "{path}")"#),
            &env,
            &ctx,
        ));
        assert_eq!(place(&found[0], ":start"), (2.0, 4.0));
        assert!(get(&found[0], ":stale").is_truthy());
    }

    #[test]
    fn a_closed_file_left_as_it_was_keeps_its_findings_fresh() {
        let sandbox = Sandbox::new("closed-unedited");
        let path = sandbox.file("a.c", "one line\nint count;\n");
        let (ctx, env) = editor();
        let name = open(&path, &env, &ctx);
        run(
            &format!(r#"(finding-set 'lint "{path}" {ONE_FINDING})"#),
            &env,
            &ctx,
        );
        run(&format!(r#"(close-buffer "{name}")"#), &env, &ctx);
        let found = list(&run(
            &format!(r#"(findings-for-file "{path}")"#),
            &env,
            &ctx,
        ));
        assert!(get(&found[0], ":stale").is_nil());
    }

    #[test]
    fn a_place_past_the_end_of_the_file_stops_at_its_end() {
        let sandbox = Sandbox::new("past-end");
        let path = sandbox.file("a.c", "one\n");
        let (ctx, env) = editor();
        open(&path, &env, &ctx);
        run(
            &format!(
                r#"(finding-set 'lint "{path}" '((:start (9 . 9) :severity error :message "m")))"#
            ),
            &env,
            &ctx,
        );
        let found = list(&run(
            &format!(r#"(findings-for-file "{path}")"#),
            &env,
            &ctx,
        ));
        assert_eq!(place(&found[0], ":start"), (2.0, 0.0));
    }
}
