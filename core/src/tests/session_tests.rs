//! What is kept from one run to the next: the session file's lines, read back
//! as they were written; the recent files and the places, each path once and
//! newest first; and two editors, one quitting and one starting, sharing a
//! state directory.
//!
//! Design: `doc/architettura-editor.typ, <memoria>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::session::{SESSION_FILE, Session};
    use crate::editor::{EditorState, create_global_env};
    use crate::managers::{History, PLACES_LIMIT, Visits};
    use crate::paths::DirError;
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// A directory of the test's own, holding the files it opens and, under
    /// `state`, the state directory.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsedit-session-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("state")).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn file(&self, name: &str, contents: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("writing a file");
            path.display().to_string()
        }

        fn state(&self) -> Result<PathBuf, DirError> {
            Ok(self.0.join("state"))
        }

        fn session_file(&self) -> PathBuf {
            self.0.join("state").join(SESSION_FILE)
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

    fn strings(values: &LispExp<Ctx>) -> Vec<String> {
        values
            .iter()
            .filter_map(|item| match item {
                LispExp::String(text) => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d())
    }

    fn open(path: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(&format!("(find-file {path:?})"), env, ctx);
    }

    /// Open a prompt, type TEXT into it, and press Return.
    fn answer(prompt: &str, text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!(r#"(minibuffer-read "{prompt}" nil nil nil)"#),
            env,
            ctx,
        );
        ctx.with_current_buffer_mut(|buf| {
            for c in text.chars() {
                buf.text.insert(c);
            }
        });
        run("(minibuffer-confirm)", env, ctx);
    }

    fn logged(ctx: &Ctx, needle: &str) -> bool {
        ctx.get_logs().iter().any(|line| line.contains(needle))
    }

    // ----------------------------------------------------------------
    // The file's lines
    // ----------------------------------------------------------------

    #[test]
    fn what_is_written_reads_back_the_same_even_with_tabs_newlines_and_backslashes() {
        let session = Session {
            history: vec![
                ("M-x".into(), vec!["find-file".into(), "grep".into()]),
                (
                    "Eval: ".into(),
                    vec!["(insert \"a\\tb\")".into(), "line one\nline two\r".into()],
                ),
                ("key\twith a tab".into(), vec!["C:\\path\\".into()]),
            ],
            recent: vec!["/home/me/a file.txt".into(), "/tmp/b\\c".into()],
            places: vec![("/home/me/a file.txt".into(), 42)],
        };
        let text = session.render();
        assert_eq!(
            text.lines().count(),
            1 + 5 + 2 + 1,
            "a header, then one line per answer, file and place:\n{text}"
        );
        assert_eq!(Session::parse(&text), Some((session, 0)));
    }

    #[test]
    fn a_file_without_the_header_is_not_read() {
        assert_eq!(Session::parse(""), None);
        assert_eq!(Session::parse("recent\t/a\n"), None);
        assert_eq!(Session::parse("rsedit-session 2\nrecent\t/a\n"), None);
    }

    #[test]
    fn lines_not_understood_are_counted_and_left_out() {
        let text = "rsedit-session 1\n\
                    recent\t/a\n\
                    place\t/a\tnot-a-number\n\
                    something-newer\tx\n\
                    \n\
                    history\tM-x\n\
                    place\t/a\t7\r\n";
        let (session, skipped) = Session::parse(text).expect("the header is right");
        assert_eq!(
            skipped, 3,
            "the bad place, the unknown kind, the short history"
        );
        assert_eq!(session.recent, vec!["/a".to_string()]);
        assert_eq!(session.places, vec![("/a".to_string(), 7)]);
        assert!(session.history.is_empty());
    }

    // ----------------------------------------------------------------
    // The compartments
    // ----------------------------------------------------------------

    #[test]
    fn answers_read_back_come_after_those_given_in_this_run() {
        let mut history = History::default();
        history.remember("M-x", "typed-now", 100);
        history.restore("M-x", vec!["older".into(), "".into(), "oldest".into()], 3);
        assert_eq!(
            history.entries("M-x"),
            vec!["typed-now", "older", "oldest"],
            "the empty answer is left out, as a prompt leaves it out"
        );
        history.restore("Find file: ", vec!["a".into(), "b".into()], 1);
        assert_eq!(
            history.entries("Find file: "),
            vec!["a"],
            "capped at the limit"
        );
        history.restore("never", vec!["x".into()], 0);
        assert!(history.rings().iter().all(|(key, _)| key != "never"));
    }

    #[test]
    fn the_rings_are_listed_in_the_order_of_their_keys() {
        let mut history = History::default();
        history.remember("b", "1", 10);
        history.remember("a", "2", 10);
        let keys: Vec<String> = history.rings().into_iter().map(|(key, _)| key).collect();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn a_file_visited_again_moves_to_the_front_and_is_listed_once() {
        let mut visits = Visits::default();
        visits.visited("/a", 3);
        visits.visited("/b", 3);
        visits.visited("/a", 3);
        assert_eq!(visits.recent(), vec!["/a", "/b"]);
        visits.visited("/c", 3);
        visits.visited("/d", 3);
        assert_eq!(visits.recent(), vec!["/d", "/c", "/a"], "the oldest goes");
    }

    #[test]
    fn a_place_at_the_start_is_forgotten_rather_than_kept() {
        let mut visits = Visits::default();
        visits.left_at("/a", 10);
        assert_eq!(visits.place("/a"), Some(10));
        visits.left_at("/a", 0);
        assert_eq!(visits.place("/a"), None);
    }

    #[test]
    fn the_places_are_capped_the_oldest_going_first() {
        let mut visits = Visits::default();
        for n in 0..=PLACES_LIMIT {
            visits.left_at(&format!("/{n}"), 1);
        }
        assert_eq!(visits.places().len(), PLACES_LIMIT);
        assert_eq!(visits.place("/0"), None);
        assert_eq!(visits.place(&format!("/{PLACES_LIMIT}")), Some(1));
    }

    #[test]
    fn what_is_read_back_does_not_replace_what_this_run_knows() {
        let mut visits = Visits::default();
        visits.visited("/now", 10);
        visits.left_at("/now", 5);
        visits.restore(
            vec!["/old".into(), "/now".into()],
            vec![("/now".into(), 99), ("/old".into(), 3)],
            10,
        );
        assert_eq!(visits.recent(), vec!["/now", "/old"]);
        assert_eq!(visits.place("/now"), Some(5));
        assert_eq!(visits.place("/old"), Some(3));
    }

    // ----------------------------------------------------------------
    // Within one run
    // ----------------------------------------------------------------

    #[test]
    fn recent_files_lists_what_was_opened_newest_first() {
        let sandbox = Sandbox::new("recent");
        let a = sandbox.file("a.txt", "a");
        let b = sandbox.file("b.txt", "b");
        let (ctx, env) = editor();
        open(&a, &env, &ctx);
        open(&b, &env, &ctx);
        assert_eq!(
            strings(&run("(recent-files)", &env, &ctx)),
            vec![b.clone(), a.clone()]
        );
        open(&a, &env, &ctx);
        assert_eq!(
            strings(&run("(recent-files)", &env, &ctx)),
            vec![a, b],
            "going back to a file open already counts as a visit"
        );
    }

    #[test]
    fn a_new_file_and_one_written_under_a_new_name_are_recent_files_too() {
        let sandbox = Sandbox::new("written");
        let new = sandbox.0.join("new.txt").display().to_string();
        let copy = sandbox.0.join("copy.txt").display().to_string();
        let (ctx, env) = editor();
        open(&new, &env, &ctx);
        run(&format!("(write-file {copy:?})"), &env, &ctx);
        assert_eq!(strings(&run("(recent-files)", &env, &ctx)), vec![copy, new]);
    }

    #[test]
    fn recent_files_length_caps_the_list() {
        let sandbox = Sandbox::new("length");
        let a = sandbox.file("a.txt", "a");
        let b = sandbox.file("b.txt", "b");
        let (ctx, env) = editor();
        run("(setq recent-files-length 1)", &env, &ctx);
        open(&a, &env, &ctx);
        open(&b, &env, &ctx);
        assert_eq!(strings(&run("(recent-files)", &env, &ctx)), vec![b]);
    }

    #[test]
    fn clear_recent_files_forgets_the_list_and_says_whether_there_was_one() {
        let sandbox = Sandbox::new("clear");
        let a = sandbox.file("a.txt", "a");
        let (ctx, env) = editor();
        open(&a, &env, &ctx);
        assert!(run("(clear-recent-files)", &env, &ctx).is_truthy());
        assert!(run("(recent-files)", &env, &ctx).is_nil());
        assert!(run("(clear-recent-files)", &env, &ctx).is_nil());
    }

    #[test]
    fn a_file_closed_and_opened_again_opens_where_point_was() {
        let sandbox = Sandbox::new("closed");
        let a = sandbox.file("a.txt", "one\ntwo\nthree\n");
        let (ctx, env) = editor();
        open(&a, &env, &ctx);
        run("(goto-char 6)", &env, &ctx);
        run(r#"(close-buffer "a.txt")"#, &env, &ctx);
        open(&a, &env, &ctx);
        assert_eq!(point(&ctx), 6);
    }

    #[test]
    fn the_window_shows_the_line_point_was_put_back_on() {
        let sandbox = Sandbox::new("shown");
        let text: String = (1..=200).map(|n| format!("line {n}\n")).collect();
        let a = sandbox.file("a.txt", &text);
        let (ctx, env) = editor();
        open(&a, &env, &ctx);
        run("(goto-line 150)", &env, &ctx);
        run(r#"(close-buffer "a.txt")"#, &env, &ctx);
        open(&a, &env, &ctx);
        let shown = ctx.snapshot(&env, 80, 24).views[0].lines.clone();
        assert!(
            shown.iter().any(|line| line.trim_end() == "line 150"),
            "{shown:?}"
        );
    }

    #[test]
    fn with_save_place_off_a_file_opens_at_its_start() {
        let sandbox = Sandbox::new("no-place");
        let a = sandbox.file("a.txt", "one\ntwo\nthree\n");
        let (ctx, env) = editor();
        run("(setq save-place nil)", &env, &ctx);
        open(&a, &env, &ctx);
        run("(goto-char 6)", &env, &ctx);
        run(r#"(close-buffer "a.txt")"#, &env, &ctx);
        open(&a, &env, &ctx);
        assert_eq!(point(&ctx), 0);
    }

    #[test]
    fn find_file_hook_sees_point_where_the_file_was_left() {
        let sandbox = Sandbox::new("hook");
        let a = sandbox.file("a.txt", "one\ntwo\nthree\n");
        let (ctx, env) = editor();
        open(&a, &env, &ctx);
        run("(goto-char 6)", &env, &ctx);
        run(r#"(close-buffer "a.txt")"#, &env, &ctx);
        run(
            r#"(defvar seen nil)
               (defun note-point () (setq seen (point)))
               (add-hook nil "find-file-hook" 'note-point)"#,
            &env,
            &ctx,
        );
        open(&a, &env, &ctx);
        assert_eq!(run("seen", &env, &ctx), LispExp::number(6.0));
    }

    // ----------------------------------------------------------------
    // From one run to the next
    // ----------------------------------------------------------------

    #[test]
    fn the_next_run_has_the_answers_the_recent_files_and_the_places() {
        let sandbox = Sandbox::new("next-run");
        let a = sandbox.file("a.txt", "one\ntwo\nthree\n");
        let b = sandbox.file("b.txt", "bee\n");
        {
            let (ctx, env) = editor();
            answer("Colour:", "blue", &env, &ctx);
            answer("Colour:", "green", &env, &ctx);
            open(&a, &env, &ctx);
            run("(goto-char 8)", &env, &ctx);
            open(&b, &env, &ctx);
            assert!(ctx.save_session(&env, sandbox.state()));
        }
        let (ctx, env) = editor();
        assert!(ctx.load_session(&env, sandbox.state()));
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Colour:")"#, &env, &ctx)),
            vec!["green", "blue"]
        );
        assert_eq!(
            strings(&run("(recent-files)", &env, &ctx)),
            vec![b, a.clone()]
        );
        open(&a, &env, &ctx);
        assert_eq!(
            point(&ctx),
            8,
            "a buffer still open at exit has its place kept"
        );
    }

    #[test]
    fn a_place_past_the_end_of_a_file_that_shrank_is_its_end() {
        let sandbox = Sandbox::new("shrank");
        let a = sandbox.file("a.txt", "a longer text than it will be");
        {
            let (ctx, env) = editor();
            open(&a, &env, &ctx);
            run("(goto-char 20)", &env, &ctx);
            assert!(ctx.save_session(&env, sandbox.state()));
        }
        sandbox.file("a.txt", "short");
        let (ctx, env) = editor();
        ctx.load_session(&env, sandbox.state());
        open(&a, &env, &ctx);
        assert_eq!(point(&ctx), 5);
    }

    #[test]
    fn with_save_history_off_no_answer_is_written_or_read() {
        let sandbox = Sandbox::new("no-history");
        {
            let (ctx, env) = editor();
            run("(setq save-history nil)", &env, &ctx);
            answer("Colour:", "blue", &env, &ctx);
            assert!(ctx.save_session(&env, sandbox.state()));
        }
        let text = std::fs::read_to_string(sandbox.session_file()).expect("written");
        assert!(!text.contains("blue"), "{text}");

        {
            let (ctx, env) = editor();
            answer("Colour:", "red", &env, &ctx);
            assert!(ctx.save_session(&env, sandbox.state()));
        }
        let (ctx, env) = editor();
        run("(setq save-history nil)", &env, &ctx);
        ctx.load_session(&env, sandbox.state());
        assert!(run(r#"(minibuffer-history "Colour:")"#, &env, &ctx).is_nil());
    }

    #[cfg(unix)]
    #[test]
    fn a_new_session_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let sandbox = Sandbox::new("mode");
        let (ctx, env) = editor();
        assert!(ctx.save_session(&env, sandbox.state()));
        let mode = std::fs::metadata(sandbox.session_file())
            .expect("written")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn the_first_run_reads_nothing_and_says_nothing() {
        let sandbox = Sandbox::new("first-run");
        let (ctx, env) = editor();
        assert!(!ctx.load_session(&env, sandbox.state()));
        assert!(!ctx.load_session(&env, Err(DirError::NotFound(sandbox.0.join("missing")))));
        assert!(!logged(&ctx, "session"), "{:?}", ctx.get_logs());
    }

    #[test]
    fn a_file_of_another_kind_is_not_read_and_the_log_says_so() {
        let sandbox = Sandbox::new("other-kind");
        std::fs::write(sandbox.session_file(), "something else\n").expect("writing");
        let (ctx, env) = editor();
        assert!(!ctx.load_session(&env, sandbox.state()));
        assert!(
            logged(&ctx, "is not a session file"),
            "{:?}",
            ctx.get_logs()
        );
    }

    // ----------------------------------------------------------------
    // Two editors, and one file
    // ----------------------------------------------------------------

    #[test]
    fn two_editors_open_together_each_keep_what_the_other_learned() {
        let sandbox = Sandbox::new("two");
        let a = sandbox.file("a.txt", "one\ntwo\nthree\n");
        let b = sandbox.file("b.txt", "bee and more\n");
        let (first, first_env) = editor();
        let (second, second_env) = editor();
        first.load_session(&first_env, sandbox.state());
        second.load_session(&second_env, sandbox.state());

        answer("Colour:", "blue", &first_env, &first);
        open(&a, &first_env, &first);
        run("(goto-char 8)", &first_env, &first);
        answer("Colour:", "green", &second_env, &second);
        open(&b, &second_env, &second);
        run("(goto-char 3)", &second_env, &second);
        assert!(first.save_session(&first_env, sandbox.state()));
        assert!(second.save_session(&second_env, sandbox.state()));

        let (ctx, env) = editor();
        assert!(ctx.load_session(&env, sandbox.state()));
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Colour:")"#, &env, &ctx)),
            vec!["green", "blue"],
            "the one that wrote last has its own first"
        );
        assert_eq!(
            strings(&run("(recent-files)", &env, &ctx)),
            vec![b.clone(), a.clone()]
        );
        open(&a, &env, &ctx);
        assert_eq!(point(&ctx), 8);
        open(&b, &env, &ctx);
        assert_eq!(point(&ctx), 3);
    }

    #[test]
    fn what_another_editor_wrote_is_known_here_once_this_one_has_written() {
        let sandbox = Sandbox::new("known-here");
        let (first, first_env) = editor();
        let (second, second_env) = editor();
        answer("Colour:", "blue", &first_env, &first);
        answer("Colour:", "green", &second_env, &second);
        first.save_session(&first_env, sandbox.state());
        second.save_session(&second_env, sandbox.state());
        assert_eq!(
            strings(&run(
                r#"(minibuffer-history "Colour:")"#,
                &second_env,
                &second
            )),
            vec!["green", "blue"]
        );
    }

    #[test]
    fn writing_again_and_again_lists_nothing_twice() {
        let sandbox = Sandbox::new("again");
        let a = sandbox.file("a.txt", "one\n");
        let (first, first_env) = editor();
        let (second, second_env) = editor();
        answer("Colour:", "blue", &first_env, &first);
        open(&a, &first_env, &first);
        answer("Colour:", "green", &second_env, &second);
        for _ in 0..3 {
            first.save_session(&first_env, sandbox.state());
            second.save_session(&second_env, sandbox.state());
        }
        let (ctx, env) = editor();
        ctx.load_session(&env, sandbox.state());
        assert_eq!(
            strings(&run(r#"(minibuffer-history "Colour:")"#, &env, &ctx)),
            vec!["green", "blue"]
        );
        assert_eq!(strings(&run("(recent-files)", &env, &ctx)), vec![a]);
    }

    #[test]
    fn what_this_editor_forgot_is_not_brought_back_by_its_own_file() {
        let sandbox = Sandbox::new("forgot");
        let a = sandbox.file("a.txt", "one\ntwo\nthree\n");
        let (ctx, env) = editor();
        open(&a, &env, &ctx);
        run("(goto-char 8)", &env, &ctx);
        ctx.save_session(&env, sandbox.state());

        run("(clear-recent-files) (goto-char 0)", &env, &ctx);
        ctx.save_session(&env, sandbox.state());

        let (next, next_env) = editor();
        next.load_session(&next_env, sandbox.state());
        assert!(run("(recent-files)", &next_env, &next).is_nil());
        open(&a, &next_env, &next);
        assert_eq!(point(&next), 0, "a place given up stays given up");
    }

    #[test]
    fn a_session_that_has_not_changed_is_not_written_again() {
        let sandbox = Sandbox::new("unchanged");
        let (ctx, env) = editor();
        answer("Colour:", "blue", &env, &ctx);
        assert_eq!(ctx.write_session(&env, sandbox.state()), Ok(true));
        assert_eq!(ctx.write_session(&env, sandbox.state()), Ok(false));
        answer("Colour:", "green", &env, &ctx);
        assert_eq!(ctx.write_session(&env, sandbox.state()), Ok(true));
    }

    // ----------------------------------------------------------------
    // Written while the editor runs
    // ----------------------------------------------------------------

    /// Lets the jobs run for SPAN, and the writes they started finish.
    fn wait(ctx: &Ctx, env: &Arc<Env<Ctx>>, span: std::time::Duration) {
        ctx.work().run_for(ctx, span);
        crate::tests::file_io_tests::settle(ctx, env);
    }

    #[test]
    fn the_session_is_written_every_interval_while_the_editor_runs() {
        let sandbox = Sandbox::new("interval");
        let (ctx, env) = editor();
        run("(setq session-save-interval 1)", &env, &ctx);
        ctx.start_session_saver(&env, sandbox.state());
        answer("Colour:", "blue", &env, &ctx);

        wait(&ctx, &env, std::time::Duration::from_secs(3));
        let written = std::fs::read_to_string(sandbox.session_file()).unwrap_or_default();
        assert!(written.contains("history\tColour:\tblue"), "{written:?}");
    }

    #[test]
    fn with_no_interval_the_session_is_written_only_at_exit() {
        let sandbox = Sandbox::new("no-interval");
        let (ctx, env) = editor();
        run("(setq session-save-interval nil)", &env, &ctx);
        ctx.start_session_saver(&env, sandbox.state());
        answer("Colour:", "blue", &env, &ctx);

        wait(&ctx, &env, std::time::Duration::from_secs(2));
        assert!(!sandbox.session_file().exists());
        assert!(ctx.save_session(&env, sandbox.state()));
        assert!(sandbox.session_file().exists());
    }

    #[test]
    fn no_state_directory_means_no_session_written_and_the_log_says_why() {
        let (ctx, env) = editor();
        let undetermined = Err(DirError::Undetermined(crate::paths::AppDir::State));
        assert!(!ctx.save_session(&env, undetermined));
        assert!(logged(&ctx, "No session written"), "{:?}", ctx.get_logs());
    }

    // ----------------------------------------------------------------
    // recent-files.lisp
    // ----------------------------------------------------------------

    fn with_recent_files_module(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/recent-files.lisp"),
        ] {
            try_run(source, env, ctx).expect("loading the shipped lisp");
        }
    }

    #[test]
    fn find_recent_file_with_none_says_so_and_opens_no_prompt() {
        let (ctx, env) = editor();
        with_recent_files_module(&env, &ctx);
        run("(find-recent-file)", &env, &ctx);
        assert_eq!(ctx.get_echo_message(), "No recent files");
        assert!(!ctx.minibuffer_is_open());
    }

    #[test]
    fn find_recent_file_opens_the_file_chosen() {
        let sandbox = Sandbox::new("chosen");
        let a = sandbox.file("a.txt", "a");
        let b = sandbox.file("b.txt", "b");
        let (ctx, env) = editor();
        with_recent_files_module(&env, &ctx);
        open(&a, &env, &ctx);
        open(&b, &env, &ctx);
        assert_eq!(
            strings(&run(
                r#"(find-recent-file--candidates "a.txt")"#,
                &env,
                &ctx
            ))
            .first(),
            Some(&a)
        );
        run("(find-recent-file)", &env, &ctx);
        assert!(ctx.minibuffer_is_open());
        ctx.with_current_buffer_mut(|buf| {
            for c in a.chars() {
                buf.text.insert(c);
            }
        });
        run("(minibuffer-confirm)", &env, &ctx);
        assert_eq!(ctx.get_current_buffer_name(), "a.txt");
    }
}
