//! Lists of places: making one, walking one, and the seams between them.
//!
//! # What is worth testing here
//!
//! Three things, and none of them is "does the search find the word".
//!
//! The first is the **index arithmetic**, because a listing has a header line
//! and the entries are counted from zero: an off-by-one means `RET` on a line
//! opens the match above it, which looks like a search that is wrong about
//! where things are.
//!
//! The second is **who owns the walk**. `next-error` is pressed in the file
//! being fixed, not in the listing, so it cannot read the listing off the
//! screen -- and it has to keep working when the listing is not visible, and
//! stop working when it has been killed.
//!
//! The third is the **refresher**, which exists because a compilation's list
//! grows while you walk it. A walk that did not ask would show what the
//! compiler had said thirty seconds ago and give no sign of it.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    type Ctx = EditorState<GapBuffer>;

    /// A directory of this test's own, removed when it goes.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rsedit-results-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn file(&self, name: &str, contents: &str) -> String {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("creating a directory");
            }
            std::fs::write(&path, contents).expect("writing a file");
            path.display().to_string()
        }

        fn path(&self) -> String {
            self.0.display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn editor(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        ctx.with_current_buffer_mut(|buf| {
            buf.text = GapBuffer::from(text);
            buf.text.cursor_move(0, 0);
            buf.is_modified = false;
        });
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn text_of(ctx: &Ctx, name: &str) -> String {
        ctx.with_buffer(name, |buf| buf.text.to_string())
            .unwrap_or_default()
    }

    fn number(exp: &LispExp<Ctx>) -> f64 {
        match exp {
            LispExp::Number(n) => *n,
            other => panic!("expected a number, got {other:?}"),
        }
    }

    fn fields(entry: &LispExp<Ctx>) -> Vec<String> {
        entry
            .iter()
            .map(|item| match item {
                LispExp::String(text) => text.to_string(),
                LispExp::Number(n) => format!("{n}"),
                other => format!("{other:?}"),
            })
            .collect()
    }

    /// Wait for a background search to finish.
    fn settle(ctx: &Ctx, env: &Arc<Env<Ctx>>, buffer: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = run(&format!(r#"(results-state "{buffer}")"#), env, ctx);
            let done = state.iter().nth(3).map(|item| !item.is_nil());
            if done == Some(true) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the search did not finish within ten seconds"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // ----------------------------------------------------------------
    // Occur: one buffer, at once
    // ----------------------------------------------------------------

    #[test]
    fn occur_lists_what_it_found_and_attaches_it() {
        let (ctx, env) = editor("one TODO\ntwo\nthree TODO\n");
        let buffer = match run(r#"(occur--scan "TODO")"#, &env, &ctx) {
            LispExp::String(name) => name.to_string(),
            other => panic!("occur answered {other:?}"),
        };
        // The text, for reading.
        let listing = text_of(&ctx, &buffer);
        assert!(listing.contains("*scratch*:1: one TODO"), "{listing}");
        assert!(listing.contains("*scratch*:3: three TODO"), "{listing}");
        assert!(listing.contains("--- 2 matches ---"), "{listing}");
        // The set, for walking. Both, from one search: the whole point of the
        // arrangement is that the view never has to read the text back.
        assert_eq!(number(&run("(results-count)", &env, &ctx)), 2.0);
        assert_eq!(
            fields(&run("(results-entry 0)", &env, &ctx)),
            vec![
                "buffer".to_string(),
                "*scratch*".to_string(),
                "1".to_string(),
                "4".to_string(),
                "4".to_string(),
                "one TODO".to_string(),
                "4".to_string(),
                "8".to_string(),
            ]
        );
    }

    #[test]
    fn a_search_that_finds_nothing_says_so_rather_than_looking_broken() {
        let (ctx, env) = editor("nothing here\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        assert!(text_of(&ctx, "*Occur*").contains("--- nothing found ---"));
        assert_eq!(number(&run("(results-count)", &env, &ctx)), 0.0);
    }

    #[test]
    fn a_second_search_gets_a_listing_of_its_own() {
        // Two searches sharing one buffer would interleave into something
        // neither of them said -- the same reason two shell commands get two
        // output buffers.
        let (ctx, env) = editor("a b\n");
        assert_eq!(
            run(r#"(occur--scan "a")"#, &env, &ctx),
            LispExp::string("*Occur*".into())
        );
        assert_eq!(
            run(r#"(occur--scan "b")"#, &env, &ctx),
            LispExp::string("*Occur*<2>".into())
        );
    }

    #[test]
    fn the_listing_says_what_was_searched_for() {
        let (ctx, env) = editor("one TODO\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        assert!(text_of(&ctx, "*Occur*").starts_with("TODO in *scratch*\n"));
        let state = run("(results-state)", &env, &ctx);
        let parts = fields(&state);
        assert_eq!(parts[0], "TODO");
        assert_eq!(parts[1], "*scratch*");
    }

    // ----------------------------------------------------------------
    // Walking
    // ----------------------------------------------------------------

    #[test]
    fn next_error_walks_forward_and_stops_at_the_end() {
        // Not wrapped, deliberately: the end of a list is information, and a
        // walk that silently started again would have you fixing the first
        // one twice while believing you were making progress.
        let (ctx, env) = editor("one TODO\ntwo\nthree TODO\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        run(r#"(switch-to-buffer "*scratch*")"#, &env, &ctx);

        let first = fields(&run("(next-error)", &env, &ctx));
        assert_eq!(first[2], "1", "the first match is on line 1");
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(1.0)
        );

        let second = fields(&run("(next-error)", &env, &ctx));
        assert_eq!(second[2], "3");
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(3.0)
        );

        assert_eq!(run("(next-error)", &env, &ctx), LispExp::nil());
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(3.0),
            "and point stays where the last one left it"
        );
    }

    #[test]
    fn previous_error_walks_back() {
        let (ctx, env) = editor("one TODO\ntwo\nthree TODO\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        run(r#"(switch-to-buffer "*scratch*")"#, &env, &ctx);
        run("(next-error) (next-error)", &env, &ctx);
        let back = fields(&run("(previous-error)", &env, &ctx));
        assert_eq!(back[2], "1");
        assert_eq!(run("(previous-error)", &env, &ctx), LispExp::nil());
    }

    #[test]
    fn the_walk_is_over_the_list_that_was_made_last() {
        // `M-g n' is pressed in the file being fixed, so the listing cannot be
        // read off the screen -- it has to be remembered, and the one
        // remembered is the one just asked for.
        let (ctx, env) = editor("alpha\nbeta\n");
        run(r#"(occur--scan "alpha")"#, &env, &ctx);
        run(r#"(occur--scan "beta")"#, &env, &ctx);
        run(r#"(switch-to-buffer "*scratch*")"#, &env, &ctx);
        assert_eq!(
            run("(results-buffer)", &env, &ctx),
            LispExp::string("*Occur*<2>".into())
        );
        let found = fields(&run("(next-error)", &env, &ctx));
        assert_eq!(found[5], "beta");
    }

    #[test]
    fn an_older_listing_can_be_walked_again() {
        let (ctx, env) = editor("alpha\nbeta\n");
        run(r#"(occur--scan "alpha")"#, &env, &ctx);
        run(r#"(occur--scan "beta")"#, &env, &ctx);
        assert_eq!(
            run(r#"(results-select "*Occur*")"#, &env, &ctx),
            LispExp::string("*Occur*".into())
        );
        let found = fields(&run("(next-error)", &env, &ctx));
        assert_eq!(found[5], "alpha");
    }

    #[test]
    fn killing_the_listing_ends_the_walk() {
        let (ctx, env) = editor("one TODO\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        run(r#"(kill-buffer "*Occur*")"#, &env, &ctx);
        assert_eq!(run("(results-buffer)", &env, &ctx), LispExp::nil());
        assert_eq!(run("(next-error)", &env, &ctx), LispExp::nil());
    }

    #[test]
    fn walking_with_no_list_says_so() {
        let (ctx, env) = editor("text\n");
        assert_eq!(run("(next-error)", &env, &ctx), LispExp::nil());
        assert!(
            ctx.snapshot(&env, 80, 24)
                .echo_message
                .contains("No search results"),
            "the user should be told: {:?}",
            ctx.snapshot(&env, 80, 24).echo_message
        );
    }

    #[test]
    fn visiting_moves_the_walk_so_the_next_step_follows_it() {
        let (ctx, env) = editor("one TODO\ntwo TODO\nthree TODO\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        run("(results-visit 2)", &env, &ctx);
        assert_eq!(
            run("(next-error)", &env, &ctx),
            LispExp::nil(),
            "2 was last"
        );
        let back = fields(&run("(previous-error)", &env, &ctx));
        assert_eq!(back[2], "2");
    }

    // ----------------------------------------------------------------
    // Files
    // ----------------------------------------------------------------

    #[test]
    fn a_tree_is_searched_in_the_background_and_says_when_it_is_done() {
        let sandbox = Sandbox::new("tree");
        sandbox.file("one.txt", "nothing\na needle here\n");
        sandbox.file("deep/two.txt", "another needle\n");
        let (ctx, env) = editor("");
        let buffer = match run(
            &format!(r#"(grep--scan "needle" "{}")"#, sandbox.path()),
            &env,
            &ctx,
        ) {
            LispExp::String(name) => name.to_string(),
            other => panic!("grep answered {other:?}"),
        };
        settle(&ctx, &env, &buffer);
        assert_eq!(number(&run("(results-count)", &env, &ctx)), 2.0);
        let listing = text_of(&ctx, &buffer);
        assert!(listing.contains("one.txt:2: a needle here"), "{listing}");
        assert!(listing.contains("two.txt:1: another needle"), "{listing}");
        assert!(listing.contains("--- 2 matches ---"), "{listing}");
    }

    #[test]
    fn a_generated_directory_is_not_searched() {
        // `.git' alone holds thousands of objects. A search that walked it
        // would spend its whole budget before reaching a line of source.
        let sandbox = Sandbox::new("pruned");
        sandbox.file("kept.txt", "a needle\n");
        sandbox.file(".git/objects/thing", "a needle\n");
        sandbox.file("target/debug/thing", "a needle\n");
        let (ctx, env) = editor("");
        let buffer = match run(
            &format!(r#"(grep--scan "needle" "{}")"#, sandbox.path()),
            &env,
            &ctx,
        ) {
            LispExp::String(name) => name.to_string(),
            other => panic!("grep answered {other:?}"),
        };
        settle(&ctx, &env, &buffer);
        assert_eq!(number(&run("(results-count)", &env, &ctx)), 1.0);
    }

    #[test]
    fn a_file_open_with_unsaved_changes_is_searched_as_you_have_it() {
        // Otherwise the search reports text that is no longer there -- and
        // jumps to offsets that mean something else.
        let sandbox = Sandbox::new("unsaved");
        let path = sandbox.file("edited.txt", "the old line\n");
        let (ctx, env) = editor("");
        run(&format!(r#"(find-file "{path}")"#), &env, &ctx);
        run(
            r#"(end-of-buffer) (insert "a needle typed in\n")"#,
            &env,
            &ctx,
        );
        let buffer = match run(
            &format!(r#"(grep--scan "needle" "{}")"#, sandbox.path()),
            &env,
            &ctx,
        ) {
            LispExp::String(name) => name.to_string(),
            other => panic!("grep answered {other:?}"),
        };
        settle(&ctx, &env, &buffer);
        assert_eq!(
            number(&run("(results-count)", &env, &ctx)),
            1.0,
            "the buffer's text should have been searched"
        );
    }

    #[test]
    fn visiting_a_file_entry_opens_it_at_the_line() {
        let sandbox = Sandbox::new("visit");
        let path = sandbox.file("visit.txt", "one\ntwo needle\nthree\n");
        let (ctx, env) = editor("");
        let buffer = match run(
            &format!(r#"(grep--scan "needle" "{}")"#, sandbox.path()),
            &env,
            &ctx,
        ) {
            LispExp::String(name) => name.to_string(),
            other => panic!("grep answered {other:?}"),
        };
        settle(&ctx, &env, &buffer);
        run("(results-visit 0)", &env, &ctx);
        assert_eq!(run("(buffer-file-name)", &env, &ctx), LispExp::string(path));
        assert_eq!(
            run("(list (line-number-at-point) (current-column))", &env, &ctx),
            run("'(2 4)", &env, &ctx)
        );
    }

    #[test]
    fn searching_somewhere_that_is_not_a_directory_says_so() {
        let (ctx, env) = editor("");
        assert_eq!(
            run(r#"(grep--scan "x" "/no/such/place")"#, &env, &ctx),
            LispExp::nil()
        );
    }

    // ----------------------------------------------------------------
    // Lists that are not searches
    // ----------------------------------------------------------------

    #[test]
    fn anything_can_attach_a_list_and_get_the_same_keys() {
        let (ctx, env) = editor("one\ntwo\nthree\n");
        run(
            r#"(buffer-create "*Fake*")
               (results-put "a linter" "the project"
                            '(("buffer" "*scratch*" 2) ("buffer" "*scratch*" 3))
                            "*Fake*")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run(r#"(results-count "*Fake*")"#, &env, &ctx),
            LispExp::number(2.0)
        );
        let found = fields(&run("(next-error)", &env, &ctx));
        assert_eq!(found[2], "2");
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(2.0)
        );
    }

    #[test]
    fn a_list_that_is_still_growing_is_asked_before_each_step() {
        // What a compilation needs: its output arrives line by line, so the
        // list is out of date the moment it is attached. A walk that did not
        // ask would show what the compiler had said thirty seconds ago.
        let (ctx, env) = editor("one\ntwo\nthree\n");
        run(
            r#"(buffer-create "*Growing*")
               (setq grown 1)
               (defun growing--refresh (buffer)
                 (results-put "grows" "here"
                              (if (> grown 1)
                                  '(("buffer" "*scratch*" 1) ("buffer" "*scratch*" 3))
                                  '(("buffer" "*scratch*" 1)))
                              buffer)
                 (setq grown (+ grown 1)))
               (results-put "grows" "here" '(("buffer" "*scratch*" 1)) "*Growing*")
               (buffer-put 'results-refresh 'growing--refresh "*Growing*")"#,
            &env,
            &ctx,
        );
        // One entry to begin with, so a walk would end after the first...
        assert_eq!(
            run(r#"(results-count "*Growing*")"#, &env, &ctx),
            LispExp::number(1.0)
        );
        run("(next-error)", &env, &ctx);
        // ...but by the second step the refresher has added another.
        let second = fields(&run("(next-error)", &env, &ctx));
        assert_eq!(
            second[2], "3",
            "the entry that arrived since should be found"
        );
    }

    #[test]
    fn a_list_with_no_refresher_costs_nothing_to_walk() {
        // The other half of the same contract: a producer whose list is
        // complete attaches nothing and nothing is called.
        let (ctx, env) = editor("one\ntwo\n");
        run(
            r#"(buffer-create "*Fixed*")
               (setq asked nil)
               (results-put "fixed" "here" '(("buffer" "*scratch*" 1)) "*Fixed*")"#,
            &env,
            &ctx,
        );
        run("(next-error)", &env, &ctx);
        assert_eq!(run("asked", &env, &ctx), LispExp::nil());
    }

    // ----------------------------------------------------------------
    // The view
    // ----------------------------------------------------------------

    /// An editor with the modules a listing's view needs under it.
    fn with_view(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor(text);
        for (name, source) in [
            ("commands", include_str!("../../lisp/commands.lisp")),
            ("debug", include_str!("../../lisp/debug.lisp")),
            ("occur", include_str!("../../lisp/occur.lisp")),
        ] {
            let ast = Parser::new(&format!("(progn {source})"))
                .next()
                .unwrap_or_else(|why| panic!("{name}.lisp must parse: {why:?}"));
            eval(&ast, env.clone(), &ctx)
                .unwrap_or_else(|why| panic!("loading {name}.lisp: {why:?}"));
        }
        (ctx, env)
    }

    #[test]
    fn the_line_under_the_cursor_names_the_entry_it_shows() {
        // The arithmetic worth testing: a listing has a header line and its
        // entries count from zero, so an off-by-one here opens the match
        // above the one you pressed RET on -- which reads as a search that is
        // wrong about where things are.
        let (ctx, env) = with_view("one TODO\ntwo\nthree TODO\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        run(r#"(switch-to-buffer "*Occur*") (goto-line 2)"#, &env, &ctx);
        run("(occur-goto)", &env, &ctx);
        assert_eq!(
            run("(list (current-buffer) (line-number-at-point))", &env, &ctx),
            run(r#"(list "*scratch*" 1)"#, &env, &ctx)
        );

        run(r#"(switch-to-buffer "*Occur*") (goto-line 3)"#, &env, &ctx);
        run("(occur-goto)", &env, &ctx);
        assert_eq!(
            run("(list (current-buffer) (line-number-at-point))", &env, &ctx),
            run(r#"(list "*scratch*" 3)"#, &env, &ctx)
        );
    }

    #[test]
    fn the_header_and_the_trailer_name_no_entry() {
        let (ctx, env) = with_view("one TODO\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        run(
            r#"(switch-to-buffer "*Occur*") (goto-line 1) (occur-goto)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(current-buffer)", &env, &ctx),
            LispExp::string("*Occur*".into()),
            "the header should not have taken us anywhere"
        );
        run("(end-of-buffer) (occur-goto)", &env, &ctx);
        assert_eq!(
            run("(current-buffer)", &env, &ctx),
            LispExp::string("*Occur*".into())
        );
    }

    #[test]
    fn showing_an_entry_leaves_you_in_the_listing() {
        // For reading down a list of forty: the file follows the cursor and
        // the cursor stays where it can be moved again.
        let (ctx, env) = with_view("one TODO\ntwo TODO\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        run(
            r#"(switch-to-buffer "*Occur*") (goto-line 3) (occur-show)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(current-buffer)", &env, &ctx),
            LispExp::string("*Occur*".into())
        );
        assert_eq!(
            run(r#"(results-entry (results-state))"#, &env, &ctx),
            LispExp::nil(),
            "a state is not an index"
        );
        // And the walk moved with it.
        let state = fields(&run("(results-state)", &env, &ctx));
        assert_eq!(state[5], "1", "the second entry is the current one");
    }

    // ----------------------------------------------------------------
    // The set's own lifetime
    // ----------------------------------------------------------------

    #[test]
    fn the_set_goes_when_its_listing_does() {
        let (ctx, env) = editor("one TODO\n");
        run(r#"(occur--scan "TODO")"#, &env, &ctx);
        assert_eq!(
            run(r#"(results-count "*Occur*")"#, &env, &ctx),
            LispExp::number(1.0)
        );
        run(r#"(kill-buffer "*Occur*")"#, &env, &ctx);
        assert_eq!(
            run(r#"(results-count "*Occur*")"#, &env, &ctx),
            LispExp::nil()
        );
    }

    #[test]
    fn a_buffer_that_is_no_listing_has_no_count() {
        let (ctx, env) = editor("text\n");
        assert_eq!(run("(results-count)", &env, &ctx), LispExp::nil());
        assert_eq!(run("(results-entry 0)", &env, &ctx), LispExp::nil());
        assert_eq!(run("(results-state)", &env, &ctx), LispExp::nil());
        assert_eq!(run("(results-select)", &env, &ctx), LispExp::nil());
    }
}
