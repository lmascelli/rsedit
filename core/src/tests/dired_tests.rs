//! `core/lisp/dired.lisp` -- a directory in a buffer -- and the filesystem
//! primitives it is built out of.
//!
//! Driven end to end against a real editor with the module loaded, because
//! almost everything worth checking is a collaboration: a listing is
//! `list-dir` and `insert` and `set-buffer-read-only` together, and what makes
//! `D` safe is the confirmation prompt, the primitive's refusal to recurse,
//! *and* the refresh afterwards. Testing the pieces separately would leave the
//! joins untested, and the joins are where a file manager loses somebody's
//! work.
//!
//! Every test that touches the filesystem gets a directory of its own, named
//! after the test, so two running in parallel cannot see each other's files.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    /// Evaluate and unwrap, naming the source in the panic so a failure says
    /// which line of Lisp it was.
    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn string_of(exp: &LispExp<Ctx>) -> String {
        match exp {
            LispExp::String(s) => (**s).clone(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    /// An editor with the whole shipped Lisp stack loaded, in the order the
    /// default `init.lisp` loads it: `dired.lisp` is written with
    /// `defcommand` from `commands.lisp` and reports through `message` from
    /// `debug.lisp`, so neither can be left out.
    fn setup() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/minibuffer.lisp"),
            include_str!("../../lisp/dired.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// A directory of this test's own, removed when it ends.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsedit-dired-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("sandbox");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        /// The sandbox as Lisp would write it: a string, with a trailing slash.
        fn lisp(&self) -> String {
            format!("{}/", self.0.display())
        }

        fn file(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(&path, contents).expect("write");
            path
        }

        fn dir(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(&path).expect("mkdir");
            path
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Open a listing of the sandbox and put the cursor on NAME.
    ///
    /// Positioning by searching the listing rather than by counting lines: the
    /// tests should say "the cursor is on `keep.txt`", not "the cursor is on
    /// line 4", which would have to be recounted every time the fixture gains
    /// a file.
    fn open_on(sandbox: &Sandbox, name: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), env, ctx);
        goto_entry(name, env, ctx);
    }

    fn goto_entry(name: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        let listing = string_of(&run("(buffer-string)", env, ctx));
        let line = listing
            .lines()
            .position(|line| line.trim_start() == name)
            .unwrap_or_else(|| panic!("{name} is not in the listing:\n{listing}"));
        run(&format!("(goto-line {})", line + 1), env, ctx);
    }

    fn listing(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        string_of(&run("(buffer-string)", env, ctx))
    }

    /// What the open prompt is asking, read off the floating window it opened.
    fn prompt_title(ctx: &Ctx) -> String {
        ctx.floating_windows
            .read()
            .expect("read floating_windows")
            .iter()
            .find(|float| float.window.buffer_name == "*Minibuffer*")
            .and_then(|float| float.title.clone())
            .expect("a prompt should be open")
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, c: char) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    fn press_return(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    fn answer(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for c in text.chars() {
            run(&format!(r#"(self-insert "{c}")"#), env, ctx);
        }
        run("(minibuffer-confirm)", env, ctx);
    }

    // ---------------- paths ----------------

    #[test]
    fn a_directory_path_gets_exactly_one_trailing_slash() {
        let (ctx, env) = setup();
        for (given, expected) in [("/etc", "/etc/"), ("/etc/", "/etc/"), ("/", "/"), ("", "/")] {
            assert_eq!(
                run(&format!(r#"(dired--as-directory "{given}")"#), &env, &ctx),
                LispExp::string(expected.into()),
                "normalising {given}"
            );
        }
    }

    #[test]
    fn the_parent_of_a_directory_drops_its_last_component() {
        let (ctx, env) = setup();
        for (given, expected) in [
            ("/home/user/project/", "/home/user/"),
            ("/home/user/", "/home/"),
            ("/home/", "/"),
        ] {
            assert_eq!(
                run(&format!(r#"(dired--parent "{given}")"#), &env, &ctx),
                LispExp::string(expected.into()),
                "parent of {given}"
            );
        }
    }

    /// Going up from the root stays at the root rather than producing an empty
    /// path. `^` held down is not a way to break the listing.
    #[test]
    fn the_root_is_its_own_parent() {
        let (ctx, env) = setup();
        assert_eq!(
            run(r#"(dired--parent "/")"#, &env, &ctx),
            LispExp::string("/".into())
        );
    }

    /// A relative name is relative to the *listing*, not to wherever the
    /// editor happened to be started -- the process working directory is not
    /// something the user can see on screen.
    #[test]
    fn a_rename_target_without_a_slash_stays_in_the_listed_directory() {
        let sandbox = Sandbox::new("dest-relative");
        sandbox.file("a.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "a.txt", &env, &ctx);

        assert_eq!(
            run(
                r#"(dired--destination (dired--directory) "b.txt")"#,
                &env,
                &ctx
            ),
            LispExp::string(format!("{}b.txt", sandbox.lisp()))
        );
        assert_eq!(
            run(
                r#"(dired--destination (dired--directory) "sub/b.txt")"#,
                &env,
                &ctx
            ),
            LispExp::string(format!("{}sub/b.txt", sandbox.lisp()))
        );
    }

    #[test]
    fn an_absolute_rename_target_is_taken_as_given() {
        let sandbox = Sandbox::new("dest-absolute");
        sandbox.file("a.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "a.txt", &env, &ctx);

        assert_eq!(
            run(
                r#"(dired--destination (dired--directory) "/tmp/elsewhere.txt")"#,
                &env,
                &ctx
            ),
            LispExp::string("/tmp/elsewhere.txt".into())
        );
    }

    // ---------------- the listing ----------------

    #[test]
    fn a_listing_names_its_directory_and_then_its_entries() {
        let sandbox = Sandbox::new("listing");
        sandbox.file("beta.txt", "");
        sandbox.file("alpha.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);

        let shown = listing(&env, &ctx);
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(lines[0], format!("{}:", sandbox.lisp()));
        assert_eq!(lines[1], "  ..");
        // Sorted, because `list-dir` sorts -- the listing is not in whatever
        // order the filesystem happened to hand them over.
        assert_eq!(lines[2], "  alpha.txt");
        assert_eq!(lines[3], "  beta.txt");
    }

    #[test]
    fn a_directory_in_a_listing_is_marked_with_a_slash() {
        let sandbox = Sandbox::new("marks-dirs");
        sandbox.dir("sub");
        sandbox.file("file.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);

        let shown = listing(&env, &ctx);
        assert!(shown.contains("\n  sub/\n"), "{shown}");
        assert!(shown.contains("\n  file.txt\n"), "{shown}");
    }

    /// `list-dir` deliberately omits `..`, so the module puts it back: a
    /// listing that shows no way out of itself reads as a dead end.
    #[test]
    fn the_parent_is_always_offered_even_in_an_empty_directory() {
        let sandbox = Sandbox::new("empty");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);

        let shown = listing(&env, &ctx);
        assert_eq!(shown.lines().count(), 2, "{shown}");
        assert_eq!(shown.lines().nth(1), Some("  .."));
    }

    #[test]
    fn the_cursor_starts_on_the_first_thing_that_can_be_acted_on() {
        let sandbox = Sandbox::new("cursor-start");
        sandbox.file("a.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);

        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(2.0)
        );
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("  ..".into())
        );
    }

    #[test]
    fn dired_refuses_something_that_is_not_a_directory() {
        let sandbox = Sandbox::new("not-a-dir");
        let file = sandbox.file("a.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, file.display()), &env, &ctx);

        assert_ne!(ctx.get_current_buffer_name(), "*dired*");
        assert!(
            ctx.get_echo_message().contains("is not a directory"),
            "{}",
            ctx.get_echo_message()
        );
    }

    // ---------------- the listing is read-only ----------------

    #[test]
    fn a_listing_is_read_only() {
        let sandbox = Sandbox::new("read-only");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);

        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::t());
    }

    /// The keys a mode does not bind still reach `self-insert`, because a mode
    /// keymap only shadows what it binds and every printable key is bound
    /// globally. What stops the listing being typed into is the flag at the
    /// edit chokepoint, not the keymap -- so this is the test that the
    /// protection is in the right place.
    #[test]
    fn typing_an_unbound_letter_into_a_listing_changes_nothing_and_says_so() {
        let sandbox = Sandbox::new("typing");
        sandbox.file("a.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);
        let before = listing(&env, &ctx);

        run(r#"(self-insert "z")"#, &env, &ctx);

        assert_eq!(listing(&env, &ctx), before);
        assert_eq!(ctx.get_echo_message(), "Buffer is read-only");
    }

    /// Deleting is refused as well as inserting. A mode that unbound the
    /// printable keys would still leave `C-k` and `C-d` pointed at the
    /// listing.
    #[test]
    fn a_kill_command_cannot_take_a_line_out_of_a_listing() {
        let sandbox = Sandbox::new("kill");
        sandbox.file("a.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);
        let before = listing(&env, &ctx);

        run("(kill-whole-line)", &env, &ctx);
        run("(delete-char)", &env, &ctx);
        run("(clear-buffer)", &env, &ctx);

        assert_eq!(listing(&env, &ctx), before);
    }

    /// Reading is the entire point of a read-only buffer, so point has to move
    /// freely in one.
    #[test]
    fn point_still_moves_in_a_listing() {
        let sandbox = Sandbox::new("movement");
        sandbox.file("a.txt", "");
        sandbox.file("b.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);

        run("(next-line)", &env, &ctx);
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("  a.txt".into())
        );
        run("(next-line)", &env, &ctx);
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("  b.txt".into())
        );
    }

    // ---------------- reading the buffer back ----------------

    #[test]
    fn the_file_under_the_cursor_is_the_one_on_that_line() {
        let sandbox = Sandbox::new("path-here");
        sandbox.file("target.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "target.txt", &env, &ctx);

        assert_eq!(
            run("(dired--path-here)", &env, &ctx),
            LispExp::string(format!("{}target.txt", sandbox.lisp()))
        );
    }

    /// The header is not an entry, and a command that wants a file has to be
    /// able to tell.
    #[test]
    fn the_header_line_names_no_file() {
        let sandbox = Sandbox::new("header");
        sandbox.file("a.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);
        run("(goto-line 1)", &env, &ctx);

        assert_eq!(run("(dired--name-here)", &env, &ctx), LispExp::nil());
        assert_eq!(run("(dired--path-here)", &env, &ctx), LispExp::nil());
    }

    /// The directory comes out of the buffer's first line, and asking for it
    /// leaves the cursor where it was -- otherwise every command that needed
    /// to know which directory this is would move the cursor off the file the
    /// user is pointing at.
    #[test]
    fn reading_the_directory_does_not_move_the_cursor() {
        let sandbox = Sandbox::new("no-move");
        sandbox.file("a.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "a.txt", &env, &ctx);
        let before = run("(point)", &env, &ctx);

        assert_eq!(
            run("(dired--directory)", &env, &ctx),
            LispExp::string(sandbox.lisp())
        );
        assert_eq!(run("(point)", &env, &ctx), before);
    }

    // ---------------- opening ----------------

    #[test]
    fn return_on_a_file_opens_it_in_this_window() {
        let sandbox = Sandbox::new("open-file");
        sandbox.file("notes.txt", "the contents");
        let (ctx, env) = setup();
        open_on(&sandbox, "notes.txt", &env, &ctx);

        run("(dired-find-file)", &env, &ctx);

        assert_eq!(ctx.get_current_buffer_name(), "notes.txt");
        assert_eq!(
            run("(buffer-string)", &env, &ctx),
            LispExp::string("the contents".into())
        );
        assert_eq!(run("(count-windows)", &env, &ctx), LispExp::number(1.0));
    }

    #[test]
    fn return_on_a_directory_lists_it() {
        let sandbox = Sandbox::new("open-dir");
        sandbox.file("sub/inner.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "sub/", &env, &ctx);

        run("(dired-find-file)", &env, &ctx);

        assert_eq!(ctx.get_current_buffer_name(), "*dired*");
        assert_eq!(
            run("(dired--directory)", &env, &ctx),
            LispExp::string(format!("{}sub/", sandbox.lisp()))
        );
        assert!(listing(&env, &ctx).contains("  inner.txt"));
    }

    /// `..` is an entry like any other to look at, and not a file to open.
    #[test]
    fn return_on_the_parent_entry_goes_up() {
        let sandbox = Sandbox::new("up-via-ret");
        sandbox.dir("sub");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}sub")"#, sandbox.lisp()), &env, &ctx);
        goto_entry("..", &env, &ctx);

        run("(dired-find-file)", &env, &ctx);

        assert_eq!(
            run("(dired--directory)", &env, &ctx),
            LispExp::string(sandbox.lisp())
        );
    }

    #[test]
    fn up_directory_lists_the_directory_above() {
        let sandbox = Sandbox::new("up");
        sandbox.dir("sub");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}sub")"#, sandbox.lisp()), &env, &ctx);

        run("(dired-up-directory)", &env, &ctx);

        assert_eq!(
            run("(dired--directory)", &env, &ctx),
            LispExp::string(sandbox.lisp())
        );
        assert!(listing(&env, &ctx).contains("  sub/"));
    }

    /// `o` is the whole reason a file manager is nicer than `C-x C-f`: the
    /// listing stays on screen while the file opens beside it.
    #[test]
    fn o_opens_a_file_beside_the_listing_and_leaves_it_visible() {
        let sandbox = Sandbox::new("other-window");
        sandbox.file("notes.txt", "beside");
        let (ctx, env) = setup();
        open_on(&sandbox, "notes.txt", &env, &ctx);

        run("(dired-find-file-other-window)", &env, &ctx);

        assert_eq!(run("(count-windows)", &env, &ctx), LispExp::number(2.0));
        assert_eq!(ctx.get_current_buffer_name(), "notes.txt");
        // The listing is still shown, in the other window.
        let windows: Vec<String> = (0..2)
            .map(|_| {
                let name = ctx.get_current_buffer_name();
                run("(other-window 1)", &env, &ctx);
                name
            })
            .collect();
        assert!(windows.contains(&"*dired*".to_string()), "{windows:?}");
    }

    /// A second listing would have to live in the one buffer both windows
    /// show, so a directory opens here instead of splitting. Better a
    /// keystroke that does the obvious thing than two windows showing the same
    /// listing.
    #[test]
    fn o_on_a_directory_lists_it_here_rather_than_splitting() {
        let sandbox = Sandbox::new("other-window-dir");
        sandbox.dir("sub");
        let (ctx, env) = setup();
        open_on(&sandbox, "sub/", &env, &ctx);

        run("(dired-find-file-other-window)", &env, &ctx);

        assert_eq!(run("(count-windows)", &env, &ctx), LispExp::number(1.0));
        assert_eq!(
            run("(dired--directory)", &env, &ctx),
            LispExp::string(format!("{}sub/", sandbox.lisp()))
        );
    }

    #[test]
    fn opening_reports_rather_than_failing_on_the_header_line() {
        let sandbox = Sandbox::new("open-header");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);
        run("(goto-line 1)", &env, &ctx);

        run("(dired-find-file)", &env, &ctx);

        assert_eq!(ctx.get_echo_message(), "No file on this line");
        assert_eq!(ctx.get_current_buffer_name(), "*dired*");
    }

    // ---------------- refreshing ----------------

    /// The listing is rebuilt from the filesystem rather than edited to match
    /// what was expected, which is how the module recovers from anything at
    /// all -- including a change nothing in the editor made.
    #[test]
    fn refreshing_picks_up_a_file_created_behind_the_editors_back() {
        let sandbox = Sandbox::new("refresh");
        sandbox.file("first.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);
        assert!(!listing(&env, &ctx).contains("second.txt"));

        sandbox.file("second.txt", "");
        run("(dired-refresh)", &env, &ctx);

        assert!(listing(&env, &ctx).contains("  second.txt"));
    }

    #[test]
    fn refreshing_keeps_the_cursor_on_the_same_line() {
        let sandbox = Sandbox::new("refresh-cursor");
        sandbox.file("a.txt", "");
        sandbox.file("b.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "b.txt", &env, &ctx);
        let before = run("(line-number-at-point)", &env, &ctx);

        run("(dired-refresh)", &env, &ctx);

        assert_eq!(run("(line-number-at-point)", &env, &ctx), before);
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("  b.txt".into())
        );
    }

    /// A refresh writes the listing, so it has to lift the flag and put it
    /// back -- and a listing left writable afterwards would be typed into by
    /// the next keystroke.
    #[test]
    fn a_listing_is_read_only_again_after_a_refresh() {
        let sandbox = Sandbox::new("refresh-read-only");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);

        run("(dired-refresh)", &env, &ctx);

        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::t());
    }

    // ---------------- deleting ----------------

    #[test]
    fn deleting_asks_before_it_acts() {
        let sandbox = Sandbox::new("delete-asks");
        let file = sandbox.file("doomed.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "doomed.txt", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);

        assert!(
            prompt_title(&ctx).contains("Delete"),
            "{}",
            prompt_title(&ctx)
        );
        assert!(file.exists(), "nothing should be gone before the answer");
    }

    #[test]
    fn answering_yes_deletes_the_file_and_the_listing_loses_it() {
        let sandbox = Sandbox::new("delete-yes");
        let file = sandbox.file("doomed.txt", "");
        sandbox.file("keep.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "doomed.txt", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);
        answer("yes", &env, &ctx);

        assert!(!file.exists());
        let shown = listing(&env, &ctx);
        assert!(!shown.contains("doomed.txt"), "{shown}");
        assert!(shown.contains("  keep.txt"), "{shown}");
    }

    #[test]
    fn answering_anything_else_leaves_the_file_alone() {
        let sandbox = Sandbox::new("delete-no");
        let file = sandbox.file("spared.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "spared.txt", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);
        answer("no", &env, &ctx);

        assert!(file.exists());
        assert_eq!(ctx.get_echo_message(), "Not deleted");
    }

    /// An empty answer is a no. Return pressed by reflex must not be a yes.
    #[test]
    fn an_empty_answer_is_not_a_yes() {
        let sandbox = Sandbox::new("delete-empty");
        let file = sandbox.file("spared.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "spared.txt", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);
        run("(minibuffer-confirm)", &env, &ctx);

        assert!(file.exists());
    }

    /// Cancelling the prompt is not an answer at all, so nothing happens --
    /// and in particular the path the question was about is not remembered
    /// anywhere that a later `yes` could pick it up.
    #[test]
    fn cancelling_the_question_deletes_nothing() {
        let sandbox = Sandbox::new("delete-cancel");
        let file = sandbox.file("spared.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "spared.txt", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);
        run("(minibuffer-cancel)", &env, &ctx);

        assert!(file.exists());
    }

    /// "Delete src/?" is a question nobody can answer honestly, so the count
    /// is in the question.
    #[test]
    fn deleting_a_full_directory_says_how_much_would_go_with_it() {
        let sandbox = Sandbox::new("delete-deep-asks");
        sandbox.file("tree/a.txt", "");
        sandbox.file("tree/b.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "tree/", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);

        let asked = prompt_title(&ctx);
        assert!(asked.contains("Recursively delete"), "{asked}");
        assert!(asked.contains("2 entries"), "{asked}");
    }

    #[test]
    fn a_confirmed_recursive_delete_takes_the_whole_tree() {
        let sandbox = Sandbox::new("delete-deep");
        sandbox.file("tree/deep/a.txt", "");
        let tree = sandbox.path().join("tree");
        let (ctx, env) = setup();
        open_on(&sandbox, "tree/", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);
        answer("yes", &env, &ctx);

        assert!(!tree.exists());
    }

    /// An empty directory is not a tree, so it is not a recursive question --
    /// and the plain wording is what tells the user there is nothing inside.
    #[test]
    fn deleting_an_empty_directory_is_an_ordinary_question() {
        let sandbox = Sandbox::new("delete-empty-dir");
        let empty = sandbox.dir("hollow");
        let (ctx, env) = setup();
        open_on(&sandbox, "hollow/", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);
        let asked = prompt_title(&ctx);
        assert!(!asked.contains("Recursively"), "{asked}");
        answer("yes", &env, &ctx);

        assert!(!empty.exists());
    }

    #[test]
    fn the_parent_entry_cannot_be_deleted() {
        let sandbox = Sandbox::new("delete-parent");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);
        goto_entry("..", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);

        assert_eq!(ctx.get_echo_message(), "Refusing to delete ..");
        assert!(sandbox.path().exists());
    }

    /// A delete that failed leaves the file there, and the listing has to go
    /// on showing it -- which the refresh guarantees rather than leaves to
    /// luck.
    #[test]
    fn a_delete_that_fails_says_so_and_the_listing_still_shows_the_file() {
        let sandbox = Sandbox::new("delete-fails");
        let file = sandbox.file("stubborn.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "stubborn.txt", &env, &ctx);

        run("(dired-delete-file)", &env, &ctx);
        // Gone from under the prompt: the delete now fails on a missing file,
        // which is the same shape of failure as a permission denied.
        std::fs::remove_file(&file).expect("remove");
        answer("yes", &env, &ctx);

        assert!(
            ctx.get_echo_message().contains("Could not delete"),
            "{}",
            ctx.get_echo_message()
        );
        assert!(!listing(&env, &ctx).contains("stubborn.txt"));
    }

    // ---------------- renaming ----------------

    #[test]
    fn renaming_starts_from_a_prompt_naming_the_file() {
        let sandbox = Sandbox::new("rename-asks");
        sandbox.file("before.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "before.txt", &env, &ctx);

        run("(dired-rename-file)", &env, &ctx);

        assert!(
            prompt_title(&ctx).contains("before.txt"),
            "{}",
            prompt_title(&ctx)
        );
    }

    #[test]
    fn renaming_moves_the_file_and_the_listing_shows_the_new_name() {
        let sandbox = Sandbox::new("rename");
        let before = sandbox.file("before.txt", "kept");
        let (ctx, env) = setup();
        open_on(&sandbox, "before.txt", &env, &ctx);

        run("(dired-rename-file)", &env, &ctx);
        answer("after.txt", &env, &ctx);

        assert!(!before.exists());
        let after = sandbox.path().join("after.txt");
        assert_eq!(std::fs::read_to_string(&after).unwrap(), "kept");
        let shown = listing(&env, &ctx);
        assert!(shown.contains("  after.txt"), "{shown}");
        assert!(!shown.contains("before.txt"), "{shown}");
    }

    #[test]
    fn renaming_into_a_subdirectory_moves_the_file_there() {
        let sandbox = Sandbox::new("rename-into");
        sandbox.file("loose.txt", "moved");
        sandbox.dir("archive");
        let (ctx, env) = setup();
        open_on(&sandbox, "loose.txt", &env, &ctx);

        run("(dired-rename-file)", &env, &ctx);
        answer("archive/loose.txt", &env, &ctx);

        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("archive/loose.txt")).unwrap(),
            "moved"
        );
        assert!(!sandbox.path().join("loose.txt").exists());
    }

    /// The system call would replace the target without a word. For a file
    /// manager that means one mistyped name destroys an unrelated file, so the
    /// rename is refused instead.
    #[test]
    fn renaming_onto_an_existing_file_is_refused_and_destroys_nothing() {
        let sandbox = Sandbox::new("rename-clobber");
        sandbox.file("source.txt", "source");
        sandbox.file("occupied.txt", "must survive");
        let (ctx, env) = setup();
        open_on(&sandbox, "source.txt", &env, &ctx);

        run("(dired-rename-file)", &env, &ctx);
        answer("occupied.txt", &env, &ctx);

        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("occupied.txt")).unwrap(),
            "must survive"
        );
        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("source.txt")).unwrap(),
            "source"
        );
        assert!(
            ctx.get_echo_message().contains("Could not rename"),
            "{}",
            ctx.get_echo_message()
        );
    }

    #[test]
    fn the_parent_entry_cannot_be_renamed() {
        let sandbox = Sandbox::new("rename-parent");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);
        goto_entry("..", &env, &ctx);

        run("(dired-rename-file)", &env, &ctx);

        assert_eq!(ctx.get_echo_message(), "Refusing to rename ..");
    }

    // ---------------- the filesystem primitives ----------------
    //
    // Tested directly as well as through the module, because the guarantees
    // they make are the ones standing between a keystroke and somebody's work.
    // A module can be rewritten; these have to be right.

    #[test]
    fn a_directory_with_anything_in_it_is_not_deleted_without_being_asked() {
        let sandbox = Sandbox::new("prim-refuse-deep");
        sandbox.file("tree/a.txt", "");
        let tree = sandbox.path().join("tree");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(delete-file "{}")"#, tree.display()),
                &env,
                &ctx
            ),
            LispExp::nil()
        );
        assert!(tree.exists(), "a single keystroke must not lose a tree");
    }

    #[test]
    fn an_empty_directory_is_deleted_without_asking_for_recursion() {
        let sandbox = Sandbox::new("prim-empty-dir");
        let hollow = sandbox.dir("hollow");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(delete-file "{}")"#, hollow.display()),
                &env,
                &ctx
            ),
            LispExp::t()
        );
        assert!(!hollow.exists());
    }

    #[test]
    fn a_recursive_delete_takes_everything_underneath() {
        let sandbox = Sandbox::new("prim-recursive");
        sandbox.file("tree/deep/deeper/a.txt", "");
        let tree = sandbox.path().join("tree");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(delete-file "{}" t)"#, tree.display()),
                &env,
                &ctx
            ),
            LispExp::t()
        );
        assert!(!tree.exists());
    }

    /// A link to a directory is unlinked, not descended into. Which needs
    /// saying because `is_dir` follows symlinks, so without asking
    /// `symlink_metadata` first the link would be treated as the directory it
    /// points at -- refused as "not empty" when asked politely, and emptied
    /// when asked recursively.
    ///
    /// Asked politely here, which is the case that actually distinguishes the
    /// two: `remove_dir` on a link fails, `remove_file` unlinks it.
    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_directory_is_unlinked_rather_than_descended_into() {
        let sandbox = Sandbox::new("prim-symlink");
        sandbox.file("target/kept.txt", "still here");
        let link = sandbox.path().join("link");
        std::os::unix::fs::symlink(sandbox.path().join("target"), &link).expect("symlink");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(delete-file "{}")"#, link.display()),
                &env,
                &ctx
            ),
            LispExp::t(),
            "no recursion asked for, and none needed: it is one link"
        );
        assert!(
            !std::fs::symlink_metadata(&link).is_ok(),
            "the link is gone"
        );
        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("target/kept.txt")).unwrap(),
            "still here",
            "and what it pointed at is untouched"
        );
    }

    /// And recursion does not change that: `t` says "everything under this
    /// directory", and a link is not a directory however much it looks like
    /// one.
    #[cfg(unix)]
    #[test]
    fn a_recursive_delete_of_a_symlink_still_only_removes_the_link() {
        let sandbox = Sandbox::new("prim-symlink-deep");
        sandbox.file("target/kept.txt", "still here");
        let link = sandbox.path().join("link");
        std::os::unix::fs::symlink(sandbox.path().join("target"), &link).expect("symlink");
        let (ctx, env) = setup();

        run(
            &format!(r#"(delete-file "{}" t)"#, link.display()),
            &env,
            &ctx,
        );

        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("target/kept.txt")).unwrap(),
            "still here"
        );
    }

    #[test]
    fn deleting_something_that_is_not_there_reports_rather_than_signalling() {
        let sandbox = Sandbox::new("prim-missing");
        let (ctx, env) = setup();

        let missing = sandbox.path().join("never-existed");
        assert_eq!(
            run(
                &format!(r#"(delete-file "{}")"#, missing.display()),
                &env,
                &ctx
            ),
            LispExp::nil()
        );
    }

    /// Reported, not signalled, so that a caller deleting six files is not
    /// stopped at the second with four still there and nothing refreshed.
    #[test]
    fn a_failed_delete_does_not_stop_the_next_one() {
        let sandbox = Sandbox::new("prim-carry-on");
        let second = sandbox.file("second.txt", "");
        let (ctx, env) = setup();

        let outcome = run(
            &format!(
                r#"(list (delete-file "{}") (delete-file "{}"))"#,
                sandbox.path().join("missing.txt").display(),
                second.display()
            ),
            &env,
            &ctx,
        );

        assert_eq!(format!("{outcome:?}"), "(nil t)");
        assert!(!second.exists());
    }

    /// The system call would replace the target silently. For a file manager
    /// that means one mistyped name destroys an unrelated file with nothing
    /// said, so the primitive itself refuses.
    #[test]
    fn a_rename_will_not_overwrite_what_is_already_there() {
        let sandbox = Sandbox::new("prim-clobber");
        let from = sandbox.file("from.txt", "source");
        let to = sandbox.file("to.txt", "must survive");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(rename-file "{}" "{}")"#, from.display(), to.display()),
                &env,
                &ctx
            ),
            LispExp::nil()
        );
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "must survive");
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "source");
    }

    /// Renaming something onto itself is a no-op rather than a refusal --
    /// otherwise confirming a rename prompt without editing it would report a
    /// failure for having changed nothing.
    #[test]
    fn renaming_a_file_onto_itself_succeeds_and_changes_nothing() {
        let sandbox = Sandbox::new("prim-self");
        let file = sandbox.file("same.txt", "unchanged");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(rename-file "{}" "{}")"#, file.display(), file.display()),
                &env,
                &ctx
            ),
            LispExp::t()
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "unchanged");
    }

    #[test]
    fn a_rename_moves_a_file_into_another_directory() {
        let sandbox = Sandbox::new("prim-move");
        let from = sandbox.file("loose.txt", "moved");
        sandbox.dir("into");
        let to = sandbox.path().join("into/loose.txt");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(rename-file "{}" "{}")"#, from.display(), to.display()),
                &env,
                &ctx
            ),
            LispExp::t()
        );
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "moved");
        assert!(!from.exists());
    }

    #[test]
    fn the_predicates_tell_a_file_from_a_directory_from_nothing() {
        let sandbox = Sandbox::new("prim-predicates");
        let file = sandbox.file("a.txt", "");
        let dir = sandbox.dir("d");
        let (ctx, env) = setup();

        for (src, expected) in [
            (format!(r#"(file-exists-p "{}")"#, file.display()), true),
            (format!(r#"(file-exists-p "{}")"#, dir.display()), true),
            (
                format!(
                    r#"(file-exists-p "{}")"#,
                    sandbox.path().join("nope").display()
                ),
                false,
            ),
            (format!(r#"(file-directory-p "{}")"#, dir.display()), true),
            (format!(r#"(file-directory-p "{}")"#, file.display()), false),
            (
                format!(
                    r#"(file-directory-p "{}")"#,
                    sandbox.path().join("nope").display()
                ),
                false,
            ),
        ] {
            let got = run(&src, &env, &ctx);
            let expected = if expected {
                LispExp::t()
            } else {
                LispExp::nil()
            };
            assert_eq!(got, expected, "{src}");
        }
    }

    /// A dangling link is *there* -- it is exactly the kind of thing a file
    /// manager is asked to delete, and calling it absent would have the
    /// listing show a file that nothing could act on.
    #[cfg(unix)]
    #[test]
    fn a_broken_symlink_exists() {
        let sandbox = Sandbox::new("prim-dangling");
        let link = sandbox.path().join("dangling");
        std::os::unix::fs::symlink(sandbox.path().join("gone"), &link).expect("symlink");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(file-exists-p "{}")"#, link.display()),
                &env,
                &ctx
            ),
            LispExp::t()
        );
    }

    #[test]
    fn the_entry_count_ignores_dot_and_dot_dot_and_does_not_descend() {
        let sandbox = Sandbox::new("prim-count");
        sandbox.file("a.txt", "");
        sandbox.file("b.txt", "");
        sandbox.file("sub/deep/c.txt", "");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(directory-entry-count "{}")"#, sandbox.lisp()),
                &env,
                &ctx
            ),
            LispExp::number(3.0),
            "two files and one subdirectory"
        );
    }

    #[test]
    fn the_entry_count_of_something_unreadable_is_nil() {
        let sandbox = Sandbox::new("prim-count-bad");
        let file = sandbox.file("a.txt", "");
        let (ctx, env) = setup();

        assert_eq!(
            run(
                &format!(r#"(directory-entry-count "{}")"#, file.display()),
                &env,
                &ctx
            ),
            LispExp::nil()
        );
    }

    // ---------------- keys ----------------

    /// Pressed, not looked up. A mode keymap is consulted before the global
    /// one, so `g` here has to reach `dired-refresh` even though `g` globally
    /// is bound to `self-insert` like every other printable key -- and that
    /// shadowing is the whole mechanism the module's keys rest on.
    #[test]
    fn the_dired_letters_do_dired_things_rather_than_typing_themselves() {
        let sandbox = Sandbox::new("keys-refresh");
        sandbox.file("first.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);
        sandbox.file("second.txt", "");

        press(&ctx, &env, 'g');

        assert!(listing(&env, &ctx).contains("  second.txt"));
    }

    #[test]
    fn n_and_p_move_between_entries() {
        let sandbox = Sandbox::new("keys-move");
        sandbox.file("a.txt", "");
        sandbox.file("b.txt", "");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}")"#, sandbox.lisp()), &env, &ctx);

        press(&ctx, &env, 'n');
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("  a.txt".into())
        );
        press(&ctx, &env, 'n');
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("  b.txt".into())
        );
        press(&ctx, &env, 'p');
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("  a.txt".into())
        );
    }

    #[test]
    fn return_pressed_on_a_file_opens_it() {
        let sandbox = Sandbox::new("keys-ret");
        sandbox.file("notes.txt", "opened");
        let (ctx, env) = setup();
        open_on(&sandbox, "notes.txt", &env, &ctx);

        press_return(&ctx, &env);

        assert_eq!(ctx.get_current_buffer_name(), "notes.txt");
    }

    #[test]
    fn caret_pressed_goes_up_a_directory() {
        let sandbox = Sandbox::new("keys-caret");
        sandbox.dir("sub");
        let (ctx, env) = setup();
        run(&format!(r#"(dired "{}sub")"#, sandbox.lisp()), &env, &ctx);

        press(&ctx, &env, '^');

        assert_eq!(
            run("(dired--directory)", &env, &ctx),
            LispExp::string(sandbox.lisp())
        );
    }

    /// `D` and `R` reach their commands, and neither acts before an answer --
    /// pressed rather than called, so the binding is part of what is tested.
    #[test]
    fn d_and_r_pressed_open_their_questions() {
        let sandbox = Sandbox::new("keys-dr");
        let file = sandbox.file("subject.txt", "");
        let (ctx, env) = setup();
        open_on(&sandbox, "subject.txt", &env, &ctx);

        press(&ctx, &env, 'D');
        assert!(
            prompt_title(&ctx).contains("Delete"),
            "{}",
            prompt_title(&ctx)
        );
        run("(minibuffer-cancel)", &env, &ctx);
        assert!(file.exists());

        goto_entry("subject.txt", &env, &ctx);
        press(&ctx, &env, 'R');
        assert!(
            prompt_title(&ctx).contains("Rename"),
            "{}",
            prompt_title(&ctx)
        );
    }
}
