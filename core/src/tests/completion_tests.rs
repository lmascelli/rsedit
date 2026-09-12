//! Listing directories, narrowing a list, and completing a file name at a
//! command's `f` argument.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::primitives::io::{expand_path, file_completions, split_for_completion};
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn setup() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/minibuffer.lisp"),
            include_str!("../../lisp/commands.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// A directory of our own, named after the test that asked for it so two
    /// running at once cannot see each other's files.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rsedit-completion-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        fn file(&self, name: &str) -> &Self {
            std::fs::write(self.0.join(name), "contents").expect("writing a file");
            self
        }

        fn dir(&self, name: &str) -> &Self {
            std::fs::create_dir_all(self.0.join(name)).expect("creating a directory");
            self
        }

        /// The sandbox's path with a trailing separator, which is what a user
        /// part-way through typing a path inside it would have.
        fn inside(&self) -> String {
            format!("{}/", self.0.display())
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn strings(exp: &LispExp<Ctx>) -> Vec<String> {
        exp.iter()
            .filter_map(|item| match item {
                LispExp::String(s) => Some(s.to_string()),
                _ => None,
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // list-dir
    // -----------------------------------------------------------------------

    #[test]
    fn list_dir_sorts_and_marks_directories() {
        let sandbox = Sandbox::new("list-dir");
        sandbox.file("zebra.txt").file("apple.txt").dir("sub");

        let (ctx, env) = setup();
        let listed = eval_str(
            &format!(r#"(list-dir "{}")"#, sandbox.0.display()),
            &env,
            &ctx,
        )
        .expect("list-dir");

        assert_eq!(
            strings(&listed),
            vec!["apple.txt", "sub/", "zebra.txt"],
            "sorted, with a separator marking the one that can be descended into"
        );
    }

    /// `.` and `..` are in every directory and are never what anyone is looking
    /// for, so they would only ever be two entries to scroll past.
    #[test]
    fn list_dir_omits_dot_and_dot_dot() {
        let sandbox = Sandbox::new("dots");
        sandbox.file("real");

        let (ctx, env) = setup();
        let listed = eval_str(
            &format!(r#"(list-dir "{}")"#, sandbox.0.display()),
            &env,
            &ctx,
        )
        .expect("list-dir");

        assert_eq!(strings(&listed), vec!["real"]);
    }

    #[test]
    fn list_dir_with_no_argument_lists_the_current_directory() {
        let (ctx, env) = setup();
        let listed = eval_str("(list-dir)", &env, &ctx).expect("list-dir");
        assert!(
            !strings(&listed).is_empty(),
            "the directory the tests run in is not empty"
        );
    }

    /// An unreadable directory is an ordinary outcome, not a reason to signal:
    /// completion asks about a directory the user is still typing the name of,
    /// and most of those do not exist yet.
    #[test]
    fn list_dir_returns_nil_for_a_directory_that_is_not_there() {
        let (ctx, env) = setup();
        assert_eq!(
            eval_str(r#"(list-dir "/no/such/place/at/all")"#, &env, &ctx).expect("list-dir"),
            LispExp::nil()
        );
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("Cannot list")),
            "it should still be written down somewhere"
        );
    }

    // -----------------------------------------------------------------------
    // match-list
    // -----------------------------------------------------------------------

    #[test]
    fn match_list_keeps_what_contains_the_pattern() {
        let (ctx, env) = setup();
        let kept = eval_str(
            r#"(match-list '("alpha.conf" "beta.txt" "gamma.conf") ".conf")"#,
            &env,
            &ctx,
        )
        .expect("match-list");

        assert_eq!(strings(&kept), vec!["alpha.conf", "gamma.conf"]);
    }

    /// Containment, not prefix -- that is the question this answers, and it is
    /// a different one from completing.
    #[test]
    fn match_list_matches_anywhere_in_the_string() {
        let (ctx, env) = setup();
        let kept = eval_str(r#"(match-list '("readme" "unread") "read")"#, &env, &ctx)
            .expect("match-list");

        assert_eq!(strings(&kept), vec!["readme", "unread"]);
    }

    #[test]
    fn match_list_skips_what_is_not_a_string() {
        let (ctx, env) = setup();
        let kept = eval_str(r#"(match-list '("a" 1 "ba") "a")"#, &env, &ctx).expect("match-list");

        assert_eq!(
            strings(&kept),
            vec!["a", "ba"],
            "a list that is not all strings should narrow, not signal"
        );
    }

    #[test]
    fn match_list_composes_with_list_dir() {
        let sandbox = Sandbox::new("compose");
        sandbox.file("one.conf").file("two.txt").file("three.conf");

        let (ctx, env) = setup();
        let kept = eval_str(
            &format!(
                r#"(match-list (list-dir "{}") ".conf")"#,
                sandbox.0.display()
            ),
            &env,
            &ctx,
        )
        .expect("match-list");

        assert_eq!(strings(&kept), vec!["one.conf", "three.conf"]);
    }

    // -----------------------------------------------------------------------
    // Paths
    // -----------------------------------------------------------------------

    /// `HOME` pointed at a sandbox for the duration of one test, and put back
    /// however the test ends.
    ///
    /// The environment is process-wide and the test runner is threaded, so
    /// every test that leans on `HOME` takes the same lock. Only the tests in
    /// this block touch it -- everything else here uses absolute paths, which
    /// never reach the expansion at all.
    struct HomeAs {
        previous: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    impl HomeAs {
        fn new(dir: &std::path::Path) -> Self {
            let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var("HOME").ok();
            // SAFETY: the lock above is the only way into this, and no other
            // test in the crate reads `HOME`.
            unsafe { std::env::set_var("HOME", dir) };
            HomeAs {
                previous,
                _guard: guard,
            }
        }
    }

    impl Drop for HomeAs {
        fn drop(&mut self) {
            // SAFETY: as above -- still under the lock, which is released when
            // the guard field drops after this.
            unsafe {
                match &self.previous {
                    Some(home) => std::env::set_var("HOME", home),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
    }

    #[test]
    fn a_leading_tilde_expands_to_the_home_directory() {
        let sandbox = Sandbox::new("tilde");
        let _home = HomeAs::new(&sandbox.0);
        let home = sandbox.0.display().to_string();

        assert_eq!(expand_path("~"), home);
        assert_eq!(expand_path("~/notes.txt"), format!("{home}/notes.txt"));
    }

    /// Expansion has to happen on the way *into* the filesystem, not just in
    /// the primitive that exposes it -- a listing of `~` that returned nothing
    /// would make completion inside the home directory silently useless.
    #[test]
    fn list_dir_expands_a_tilde() {
        let sandbox = Sandbox::new("tilde-list");
        sandbox.file("in-home.txt");
        // The editor is built *before* `HOME` is redirected: booting one
        // creates a config directory, and a sandbox with one in it would make
        // this assert on the editor's own housekeeping rather than the listing.
        let (ctx, env) = setup();
        let _home = HomeAs::new(&sandbox.0);

        let listed = eval_str(r#"(list-dir "~")"#, &env, &ctx).expect("list-dir");

        assert_eq!(strings(&listed), vec!["in-home.txt"]);
    }

    #[test]
    fn file_completion_expands_a_tilde() {
        let sandbox = Sandbox::new("tilde-complete");
        sandbox.file("target.txt");
        let _home = HomeAs::new(&sandbox.0);

        assert_eq!(
            file_completions("~/tar"),
            vec!["~/target.txt".to_string()],
            "the candidate stays written the way the user typed it"
        );
    }

    /// ... and the candidate `~/target.txt` has to be a path `find-file` can
    /// actually open, or completion would only ever lead to a failure.
    #[test]
    fn find_file_opens_a_tilde_path() {
        let sandbox = Sandbox::new("tilde-open");
        sandbox.file("target.txt");
        let (ctx, env) = setup();
        let _home = HomeAs::new(&sandbox.0);

        assert_eq!(
            eval_str(r#"(find-file "~/target.txt")"#, &env, &ctx).expect("find-file"),
            LispExp::string("target.txt".into())
        );
        assert_eq!(ctx.get_current_buffer_name(), "target.txt");
    }

    /// `~other` is a different feature -- it needs the password database -- and
    /// a file really can be called `~weird`, so anything that is not the home
    /// directory is left exactly as it was.
    #[test]
    fn a_tilde_that_is_not_the_home_directory_is_left_alone() {
        assert_eq!(expand_path("~other/file"), "~other/file");
        assert_eq!(expand_path("/tmp/~backup"), "/tmp/~backup");
        assert_eq!(expand_path("plain"), "plain");
    }

    #[test]
    fn expand_file_name_is_the_same_answer_from_lisp() {
        let sandbox = Sandbox::new("tilde-lisp");
        let (ctx, env) = setup();
        let _home = HomeAs::new(&sandbox.0);

        assert_eq!(
            eval_str(r#"(expand-file-name "~/x")"#, &env, &ctx).expect("expand"),
            LispExp::string(format!("{}/x", sandbox.0.display()))
        );
    }

    /// The split decides which directory gets listed. Getting the root wrong is
    /// the case worth pinning: `/us` must list `/`, not the working directory.
    #[test]
    fn a_part_typed_path_splits_into_a_directory_and_a_name() {
        assert_eq!(split_for_completion("/etc/ho"), ("/etc/".into(), "ho"));
        assert_eq!(split_for_completion("/etc/"), ("/etc/".into(), ""));
        assert_eq!(split_for_completion("/us"), ("/".into(), "us"));
        assert_eq!(split_for_completion("ho"), (String::new(), "ho"));
        assert_eq!(split_for_completion(""), (String::new(), ""));
    }

    // -----------------------------------------------------------------------
    // Completing a file name
    // -----------------------------------------------------------------------

    /// Whole paths, not bare names: the minibuffer replaces its entire contents
    /// with the candidate it shows, so a bare `notes.txt` would turn
    /// `/tmp/x/no` into `notes.txt` and open the wrong file.
    #[test]
    fn file_completion_returns_whole_paths() {
        let sandbox = Sandbox::new("whole-paths");
        sandbox.file("notes.txt").file("nonsense.txt").file("other");

        let completions = file_completions(&format!("{}no", sandbox.inside()));

        assert_eq!(
            completions,
            vec![
                format!("{}nonsense.txt", sandbox.inside()),
                format!("{}notes.txt", sandbox.inside()),
            ]
        );
    }

    #[test]
    fn a_directory_candidate_keeps_its_separator_so_completion_can_descend() {
        let sandbox = Sandbox::new("descend");
        sandbox.dir("project").file("project.txt");

        let completions = file_completions(&format!("{}proj", sandbox.inside()));

        assert!(
            completions.contains(&format!("{}project/", sandbox.inside())),
            "the directory must come back ready to be typed into, got {completions:?}"
        );
        assert!(completions.contains(&format!("{}project.txt", sandbox.inside())));
    }

    /// Confirming a directory candidate and pressing Tab again has to list what
    /// is inside it. That only works because the candidate ended in a
    /// separator.
    #[test]
    fn completing_a_directory_candidate_lists_what_is_in_it() {
        let sandbox = Sandbox::new("into");
        sandbox.dir("sub");
        std::fs::write(sandbox.0.join("sub/inner.txt"), "x").expect("writing inside");

        let first = file_completions(&format!("{}s", sandbox.inside()));
        let directory = first.first().expect("the directory must be offered");

        assert_eq!(
            file_completions(directory),
            vec![format!("{directory}inner.txt")],
            "completing again from the directory candidate must descend into it"
        );
    }

    #[test]
    fn an_empty_name_offers_everything_in_the_directory() {
        let sandbox = Sandbox::new("everything");
        sandbox.file("a").file("b");

        assert_eq!(
            file_completions(&sandbox.inside()),
            vec![
                format!("{}a", sandbox.inside()),
                format!("{}b", sandbox.inside()),
            ]
        );
    }

    /// Hidden files swamp a listing of a home directory, so they are offered
    /// only once the typed name says they are wanted -- the same rule every
    /// shell uses.
    #[test]
    fn hidden_files_are_offered_only_once_a_dot_is_typed() {
        let sandbox = Sandbox::new("hidden");
        sandbox.file(".hidden").file("visible");

        assert_eq!(
            file_completions(&sandbox.inside()),
            vec![format!("{}visible", sandbox.inside())],
            "a bare listing must not be swamped by dotfiles"
        );
        assert_eq!(
            file_completions(&format!("{}.", sandbox.inside())),
            vec![format!("{}.hidden", sandbox.inside())],
            "but typing a dot asks for exactly them"
        );
    }

    #[test]
    fn completing_inside_a_directory_that_is_not_there_offers_nothing() {
        assert!(file_completions("/no/such/place/at/all/x").is_empty());
    }

    // -----------------------------------------------------------------------
    // ... and through the prompt that uses it
    // -----------------------------------------------------------------------

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode) {
        ctx.handle_key_event(
            KeyEvent {
                code,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    fn minibuffer_text(ctx: &Ctx) -> String {
        ctx.get_buffer("*Minibuffer*")
            .expect("the prompt must be open")
            .read()
            .unwrap()
            .text
            .to_string()
    }

    /// The whole point of the `f` spec: Tab at a command's file-name argument
    /// fills the name in. Driven through real keystrokes, because the
    /// completion primitive answers a question only the prompt machinery asks.
    #[test]
    fn tab_completes_a_file_name_argument() {
        let sandbox = Sandbox::new("tab");
        sandbox.file("unmistakable.txt");

        let (ctx, env) = setup();
        eval_str(r#"(call-interactively "find-file")"#, &env, &ctx).expect("start find-file");

        for c in format!("{}unmist", sandbox.inside()).chars() {
            ctx.handle_key_event(
                KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers: KeyModifiers::default(),
                },
                &env,
            );
        }
        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(
            minibuffer_text(&ctx),
            format!("{}unmistakable.txt", sandbox.inside()),
            "Tab must complete the path, not just the bare name"
        );
    }

    /// And confirming the completed candidate opens it -- which is the step
    /// that would break if completion handed back a bare name.
    #[test]
    fn the_completed_path_is_one_find_file_can_open() {
        let sandbox = Sandbox::new("open");
        sandbox.file("openable.txt");

        let (ctx, env) = setup();
        eval_str(r#"(call-interactively "find-file")"#, &env, &ctx).expect("start find-file");
        for c in format!("{}open", sandbox.inside()).chars() {
            ctx.handle_key_event(
                KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers: KeyModifiers::default(),
                },
                &env,
            );
        }
        press(&ctx, &env, KeyCode::Tab);
        press(&ctx, &env, KeyCode::Enter);

        assert_eq!(ctx.get_current_buffer_name(), "openable.txt");
    }

    /// A buffer-name argument still completes over buffers. The file branch is
    /// an addition, not a replacement.
    #[test]
    fn a_buffer_argument_still_completes_over_buffers() {
        let (ctx, env) = setup();
        eval_str(
            r#"(progn (buffer-create "notes")
                      (defun take (b) b)
                      (register-command 'take '("bBuffer: "))
                      (call-interactively "take"))"#,
            &env,
            &ctx,
        )
        .expect("start the command");

        for c in "not".chars() {
            ctx.handle_key_event(
                KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers: KeyModifiers::default(),
                },
                &env,
            );
        }
        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(minibuffer_text(&ctx), "notes");
    }
}
