//! What asking for completions costs, and the three places it used to be paid
//! over and over.
//!
//! # What is worth testing here
//!
//! Nothing in this file is about whether completion *works* -- that is tested
//! elsewhere, and it always worked. It is about how often the same work is
//! done, which is invisible in every other kind of test: a candidate list
//! computed three times looks exactly like one computed once.
//!
//! The bug these came from was a three-second pause on every press of Tab in
//! the manual prompt, on the thread that draws. It had three causes stacked on
//! each other, and each one is checked below by *counting* rather than by
//! timing: a timing test on a shared machine measures the machine.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    type Ctx = EditorState<GapBuffer>;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn load(modules: &[(&str, &str)], env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for (name, source) in modules {
            let ast = Parser::new(&format!("(progn {source})"))
                .next()
                .unwrap_or_else(|why| panic!("{name}.lisp must parse: {why:?}"));
            eval(&ast, env.clone(), ctx)
                .unwrap_or_else(|why| panic!("loading {name}.lisp: {why:?}"));
        }
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn count(exp: &LispExp<Ctx>) -> f64 {
        match exp {
            LispExp::Number(n) => *n,
            other => panic!("expected a number, got {other:?}"),
        }
    }

    // ----------------------------------------------------------------
    // Compiling the same pattern twice
    // ----------------------------------------------------------------

    #[test]
    fn matching_the_same_pattern_repeatedly_compiles_it_once() {
        // Measured rather than counted, because the cache is in Rust and has
        // no visible door -- but the margin is not a close call: compiling a
        // pattern costs about a hundred times what matching one does, so a
        // thousand matches of one pattern are milliseconds with the cache and
        // the better part of a second without it. The bound is loose enough
        // to survive a loaded machine and tight enough to fail the moment the
        // cache goes.
        let (ctx, env) = editor();
        let started = Instant::now();
        run(
            r#"(dotimes (i 1000) (string-match "^[^.]+" "page123.1.gz"))"#,
            &env,
            &ctx,
        );
        let spent = started.elapsed();
        assert!(
            spent < Duration::from_millis(400),
            "a thousand matches of one pattern took {spent:?}; the compiled form is not being kept"
        );
    }

    #[test]
    fn a_pattern_that_does_not_compile_is_still_refused() {
        // A cache that answered from a stale entry, or that cached failures
        // as successes, would show up here first.
        let (ctx, env) = editor();
        let refused =
            r#"(condition-case nil (string-match "(unclosed" "text") (invalid-regexp 'refused))"#;
        assert_eq!(run(refused, &env, &ctx), LispExp::symbol("refused".into()));
        assert!(!run(r#"(string-match "^te" "text")"#, &env, &ctx).is_nil());
        assert_eq!(
            run(refused, &env, &ctx),
            LispExp::symbol("refused".into()),
            "refused the second time too, not answered from the cache"
        );
    }

    #[test]
    fn two_patterns_do_not_answer_for_each_other() {
        let (ctx, env) = editor();
        assert!(!run(r#"(string-match "^a" "abc")"#, &env, &ctx).is_nil());
        assert_eq!(
            run(r#"(string-match "^b" "abc")"#, &env, &ctx),
            LispExp::nil()
        );
        assert!(!run(r#"(string-match "^a" "abc")"#, &env, &ctx).is_nil());
    }

    // ----------------------------------------------------------------
    // Asking the candidate function
    // ----------------------------------------------------------------

    #[test]
    fn pressing_tab_again_does_not_ask_the_candidate_function_again() {
        // The second cause. With a presenter installed -- which the
        // completion module installs -- every press used to call the
        // candidate function, and for the manual that function walks every
        // page installed. Pressing Tab cannot change what the answer is.
        let (ctx, env) = editor();
        run(
            r#"(setq asked 0)
               (setq *completion-read-function* (lambda (items choose) nil))
               (minibuffer-read "Thing:"
                                (lambda (answer) nil)
                                (lambda (input) (setq asked (+ asked 1)) '("alpha" "beta"))
                                nil)"#,
            &env,
            &ctx,
        );
        run("(minibuffer-complete)", &env, &ctx);
        assert_eq!(count(&run("asked", &env, &ctx)), 1.0);
        run("(minibuffer-complete) (minibuffer-complete)", &env, &ctx);
        assert_eq!(
            count(&run("asked", &env, &ctx)),
            1.0,
            "the same input should not be asked about again"
        );
    }

    #[test]
    fn typing_something_else_does_ask_again() {
        // The other half: the list has to be recomputed when the input
        // changes, or `find-file' would go on offering one directory's
        // entries after you typed a `/'.
        let (ctx, env) = editor();
        run(
            r#"(setq asked 0)
               (setq *completion-read-function* (lambda (items choose) nil))
               (minibuffer-read "Thing:"
                                (lambda (answer) nil)
                                (lambda (input) (setq asked (+ asked 1)) '("alpha"))
                                nil)"#,
            &env,
            &ctx,
        );
        run("(minibuffer-complete)", &env, &ctx);
        run(r#"(insert "x")"#, &env, &ctx);
        run("(minibuffer-complete)", &env, &ctx);
        assert_eq!(count(&run("asked", &env, &ctx)), 2.0);
    }

    // ----------------------------------------------------------------
    // The manual's own list
    // ----------------------------------------------------------------

    /// A synthetic tree of manual pages, removed when it goes.
    struct Pages(std::path::PathBuf);

    impl Pages {
        fn new(name: &str, how_many: usize) -> Self {
            let root = std::env::temp_dir().join(format!("rsedit-pages-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            let section = root.join("man1");
            std::fs::create_dir_all(&section).expect("mkdir");
            for n in 0..how_many {
                std::fs::write(section.join(format!("page{n}.1.gz")), "x").expect("write");
            }
            Pages(root)
        }

        fn path(&self) -> String {
            self.0.display().to_string()
        }
    }

    impl Drop for Pages {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn with_manpage(pages: &Pages) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor();
        load(
            &[
                ("commands", include_str!("../../lisp/commands.lisp")),
                ("debug", include_str!("../../lisp/debug.lisp")),
                ("minibuffer", include_str!("../../lisp/minibuffer.lisp")),
                ("manpage", include_str!("../../lisp/manpage.lisp")),
            ],
            &env,
            &ctx,
        );
        run(
            &format!(r#"(setq manpage-path '("{}"))"#, pages.path()),
            &env,
            &ctx,
        );
        (ctx, env)
    }

    #[test]
    fn the_list_of_pages_is_built_once_and_kept() {
        let pages = Pages::new("kept", 50);
        let (ctx, env) = with_manpage(&pages);
        // Counted as a difference, because the editor ships pages of its own
        // and the list includes them.
        let first = count(&run("(length (manpage-names))", &env, &ctx));
        assert!(
            first >= 50.0,
            "the sandbox's pages should be in it: {first}"
        );
        // Nothing has changed, so nothing is walked again. Checked through
        // the file system rather than a counter: a page added behind the
        // cache's back is exactly what it must *not* notice.
        std::fs::write(pages.0.join("man1").join("late.1.gz"), "x").expect("write");
        assert_eq!(
            count(&run("(length (manpage-names))", &env, &ctx)),
            first,
            "the kept list should have been used"
        );
    }

    #[test]
    fn changing_the_path_rebuilds_the_list() {
        // The one thing that can make the list wrong without anybody saying
        // so, and therefore the one thing that invalidates it.
        let pages = Pages::new("rebuilt", 10);
        let other = Pages::new("rebuilt-other", 3);
        let (ctx, env) = with_manpage(&pages);
        let first = count(&run("(length (manpage-names))", &env, &ctx));
        run(
            &format!(r#"(setq manpage-path '("{}"))"#, other.path()),
            &env,
            &ctx,
        );
        assert_eq!(
            count(&run("(length (manpage-names))", &env, &ctx)),
            first - 7.0,
            "ten pages should have become three"
        );
    }

    /// Run turns and drain callbacks until the background index build has
    /// finished, the way the event loop would.
    ///
    /// A test cannot simply sleep: the callback that marks the list complete
    /// runs on the thread that runs commands, which here is this one, and
    /// nothing runs it unless something asks.
    fn settle(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            ctx.run_owed_callbacks(env);
            if run("manpage--building-for", env, ctx).is_nil() {
                return;
            }
            assert!(Instant::now() < deadline, "the index build never finished");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_page_installed_since_is_found_after_asking_for_it() {
        let pages = Pages::new("reindex", 5);
        let (ctx, env) = with_manpage(&pages);
        let first = count(&run("(length (manpage-names))", &env, &ctx));
        std::fs::write(pages.0.join("man1").join("new.1.gz"), "x").expect("write");
        run("(manpage--reindex)", &env, &ctx);
        settle(&ctx, &env);
        assert_eq!(
            count(&run("(length (manpage-names))", &env, &ctx)),
            first + 1.0
        );
    }

    #[test]
    fn the_previous_list_is_answered_until_the_new_one_has_something() {
        // A rebuild must not make the completion list empty for as long as it
        // takes to walk the tree. The old answer is out of date by one page;
        // no answer at all is wrong by all of them.
        let pages = Pages::new("during", 40);
        let (ctx, env) = with_manpage(&pages);
        let before = count(&run("(length (manpage-names))", &env, &ctx));
        run("(manpage--reindex)", &env, &ctx);
        assert_eq!(
            count(&run("(length (manpage-names))", &env, &ctx)),
            before,
            "asked in the moment between starting the build and its first turn"
        );
        settle(&ctx, &env);
        assert_eq!(
            count(&run("(length (manpage-names))", &env, &ctx)),
            before,
            "and the same afterwards, nothing having changed on disk"
        );
    }

    #[test]
    fn a_build_whose_path_changed_under_it_publishes_nothing() {
        // Rule three, tested the only way it can be tested without a race:
        // the fiber is built here and resumed here, so the path can be
        // changed between one turn and the next at a moment of the test's
        // choosing rather than the scheduler's.
        let pages = Pages::new("moved", 8);
        let other = Pages::new("moved-elsewhere", 2);
        let (ctx, env) = with_manpage(&pages);
        run("(setq manpage--names 'untouched)", &env, &ctx);
        run(
            &format!(
                r#"(setq walker (manpage--index-fiber (list "{}")))"#,
                pages.path()
            ),
            &env,
            &ctx,
        );
        // The user sets the path before the job has taken a single turn.
        run(
            &format!(r#"(setq manpage-path '("{}"))"#, other.path()),
            &env,
            &ctx,
        );
        run("(resume walker)", &env, &ctx);
        assert_eq!(
            run("manpage--names", &env, &ctx),
            LispExp::symbol("untouched".into()),
            "what it was building is the answer to a question nobody is asking"
        );
        assert!(
            !run("(fiber-done-p walker)", &env, &ctx).is_nil(),
            "and it gives up rather than walking directories for nothing"
        );
    }

    #[test]
    fn the_build_says_the_candidates_changed_as_it_goes() {
        // The signal the whole arrangement exists to deliver: a prompt with
        // the list open is told there is more of it, without anything being
        // typed. Counted rather than observed through the strip, because what
        // is being checked is that the signal is sent at all -- what a
        // presenter does with it is `completion.lisp`'s business.
        let pages = Pages::new("signalling", 40);
        let (ctx, env) = with_manpage(&pages);
        run(
            "(setq signals 0)
             (defun count-signal () (setq signals (+ signals 1)))
             (setq *completion-invalidated-function* 'count-signal)",
            &env,
            &ctx,
        );
        run("(manpage--reindex)", &env, &ctx);
        settle(&ctx, &env);
        assert!(
            count(&run("signals", &env, &ctx)) >= 1.0,
            "at least the one at the end, and one per directory before it"
        );
    }

    #[test]
    fn a_build_that_did_not_finish_does_not_leave_a_short_list_looking_complete() {
        // Rule six with its consequence spelt out: the job reports that it is
        // done whether or not it got there, and "done" is not the same as
        // "complete". A list marked complete when it is not is a completion
        // list permanently missing pages, with nothing anywhere to say so.
        //
        // The callback is called here rather than arranged for, because what
        // is being checked is what the *mode* does with the two answers, and
        // the two answers are its whole input.
        let pages = Pages::new("stopped", 10);
        let (ctx, env) = with_manpage(&pages);
        settle(&ctx, &env);
        run(
            "(setq manpage--names '(\"a\" \"b\"))
             (setq manpage--names-for nil)
             (setq manpage--building-for manpage-path)
             (manpage--index-done \"manpage-index\" nil)",
            &env,
            &ctx,
        );
        assert!(
            run("manpage--names-for", &env, &ctx).is_nil(),
            "a build that was stopped part way has not built a complete list"
        );
        assert!(
            run("manpage--building-for", &env, &ctx).is_nil(),
            "but nothing is building any more either, which the mode has to say"
        );
        assert_eq!(
            count(&run("(length (manpage-names))", &env, &ctx)),
            10.0 + rsedit_pages(&env, &ctx),
            "so the next ask walks the tree and answers in full"
        );
        assert!(
            !run("manpage--names-for", &env, &ctx).is_nil(),
            "which is what marks it complete"
        );
    }

    #[test]
    fn a_build_that_finished_marks_the_list_complete() {
        let pages = Pages::new("completed", 4);
        let (ctx, env) = with_manpage(&pages);
        settle(&ctx, &env);
        run(
            "(setq manpage--names-for nil)
             (setq manpage--building-for manpage-path)
             (manpage--index-done \"manpage-index\" t)",
            &env,
            &ctx,
        );
        assert!(
            !run("manpage--names-for", &env, &ctx).is_nil(),
            "and then nothing walks the tree again"
        );
    }

    /// How many pages the editor ships of its own, which every count in a
    /// sandbox is on top of.
    fn rsedit_pages(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> f64 {
        count(&run(
            "(let ((ours (manpage--rsedit-directory)))
               (if (and ours (file-directory-p ours))
                   (length (manpage--names-in ours))
                   0))",
            env,
            ctx,
        ))
    }

    #[test]
    fn the_index_is_built_in_the_background_when_the_module_loads() {
        // The whole point of doing it then: the first `C-h m' should find a
        // list waiting rather than pay for the walk while somebody holds Tab
        // down. The call is dispatched to the worker, so this waits for it --
        // and the list is correct either way, because `manpage-names' builds
        // it itself if the background has not got there yet.
        let pages = Pages::new("background", 20);
        let (ctx, env) = with_manpage(&pages);
        // Re-loading the module is what triggers it, now that the path is set.
        run("(manpage--reindex)", &env, &ctx);
        settle(&ctx, &env);
        let built = count(&run("(length manpage--names)", &env, &ctx));
        assert!(
            built >= 20.0,
            "the sandbox's pages should be in it: {built}"
        );
        assert_eq!(
            count(&run("(length (manpage-names))", &env, &ctx)),
            built,
            "and asking should use what the worker built rather than walking again"
        );
    }

    #[test]
    fn asking_for_candidates_does_not_walk_the_tree_again() {
        // The three fixes together, from the place the bug was reported: the
        // completion source for the manual prompt, asked as the prompt asks
        // it. A thousand pages and ten presses used to be half a minute.
        let pages = Pages::new("candidates", 1_000);
        let (ctx, env) = with_manpage(&pages);
        run(r#"(manpage-candidates "page")"#, &env, &ctx);
        let started = Instant::now();
        for _ in 0..10 {
            run(r#"(manpage-candidates "page1")"#, &env, &ctx);
        }
        let spent = started.elapsed();
        assert!(
            spent < Duration::from_secs(2),
            "ten presses took {spent:?}, which is the pause this was meant to remove"
        );
    }
}
