//! The `project` module: which directory is a file's root, by the markers
//! above it or by a root declared by hand; the projects it remembers in the
//! session; and the C-x p commands, each working at the root.
//!
//! Design: `doc/architettura-editor.typ, <mod-project>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    type Ctx = EditorState<GapBuffer>;

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/dired.lisp"),
            include_str!("../../lisp/features/occur.lisp"),
            include_str!("../../lisp/features/shell.lisp"),
            include_str!("../../lisp/features/compile.lisp"),
            include_str!("../../lisp/features/project.lisp"),
        ] {
            run(source, &env, &ctx);
        }
        (ctx, env)
    }

    /// A directory tree built for one test and removed after it.
    struct Sandbox(std::path::PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "rsedit-project-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("sandbox");
            Sandbox(root)
        }

        fn file(&self, relative: &str, contents: &str) -> &Self {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("sandbox subdirectory");
            }
            std::fs::write(path, contents).expect("sandbox file");
            self
        }

        fn dir(&self, relative: &str) -> &Self {
            std::fs::create_dir_all(self.0.join(relative)).expect("sandbox subdirectory");
            self
        }

        /// RELATIVE under the sandbox, ending in a separator, as roots are
        /// written.
        fn root(&self, relative: &str) -> String {
            let path = if relative.is_empty() {
                self.0.clone()
            } else {
                self.0.join(relative)
            };
            format!("{}{}", path.display(), std::path::MAIN_SEPARATOR)
        }

        fn path(&self, relative: &str) -> String {
            self.0.join(relative).display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn string(value: LispExp<Ctx>) -> String {
        match value {
            LispExp::String(text) => text.to_string(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    fn strings(value: &LispExp<Ctx>) -> Vec<String> {
        value
            .iter()
            .map(|item| match item {
                LispExp::String(text) => text.to_string(),
                other => panic!("expected a string, got {other:?}"),
            })
            .collect()
    }

    fn root_of(path: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        run(&format!("(project-root {path:?})"), env, ctx)
    }

    /// Type TEXT into the open prompt and press Return.
    fn answer(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        assert!(ctx.minibuffer_is_open(), "no prompt is open");
        ctx.with_current_buffer_mut(|buf| {
            for c in text.chars() {
                buf.text.insert(c);
            }
        });
        run("(minibuffer-confirm)", env, ctx);
    }

    /// A directory as the editor gives it back, without the separator a
    /// root ends in.
    fn bare(directory: &str) -> &str {
        directory.trim_end_matches(std::path::MAIN_SEPARATOR)
    }

    fn settle(ctx: &Ctx) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while ctx.shell_commands_running() > 0 {
            assert!(Instant::now() < deadline, "a command did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // -----------------------------------------------------------------------
    // The root
    // -----------------------------------------------------------------------

    #[test]
    fn a_repository_wins_over_a_nearer_marker() {
        let sandbox = Sandbox::new("git-wins");
        sandbox.dir(".git").file("crate/Cargo.toml", "").dir("crate/src");
        let (ctx, env) = editor();
        assert_eq!(
            string(root_of(&sandbox.path("crate/src"), &env, &ctx)),
            sandbox.root("")
        );
    }

    #[test]
    fn without_a_repository_the_nearest_marker_wins() {
        let sandbox = Sandbox::new("nearest");
        sandbox
            .file("Makefile", "")
            .file("crate/Cargo.toml", "")
            .dir("crate/src");
        let (ctx, env) = editor();
        assert_eq!(
            string(root_of(&sandbox.path("crate/src"), &env, &ctx)),
            sandbox.root("crate")
        );
    }

    #[test]
    fn a_git_file_marks_a_root_as_a_git_directory_does() {
        // A worktree or a submodule has a .git file, not a directory.
        let sandbox = Sandbox::new("git-file");
        sandbox.file(".git", "gitdir: elsewhere\n").dir("src");
        let (ctx, env) = editor();
        assert_eq!(
            string(root_of(&sandbox.path("src"), &env, &ctx)),
            sandbox.root("")
        );
    }

    #[test]
    fn the_markers_are_a_setting() {
        let sandbox = Sandbox::new("setting");
        sandbox.dir(".git").file("app/WORKSPACE", "").dir("app/lib");
        let (ctx, env) = editor();
        run(r#"(setq project-markers '("WORKSPACE" ".git"))"#, &env, &ctx);
        assert_eq!(
            string(root_of(&sandbox.path("app/lib"), &env, &ctx)),
            sandbox.root("app")
        );
    }

    #[test]
    fn a_directory_with_no_marker_above_has_no_root() {
        let sandbox = Sandbox::new("none");
        sandbox.dir("lone");
        let (ctx, env) = editor();
        run("(setq project-markers '(\"NO-SUCH-MARKER\"))", &env, &ctx);
        assert_eq!(root_of(&sandbox.path("lone"), &env, &ctx), LispExp::nil());
    }

    #[test]
    fn a_root_declared_in_project_roots_wins_over_every_marker() {
        let sandbox = Sandbox::new("declared-var");
        sandbox.dir(".git").dir("mono/lib/src");
        let (ctx, env) = editor();
        run(
            &format!("(setq project-roots (list {:?}))", sandbox.path("mono")),
            &env,
            &ctx,
        );
        assert_eq!(
            string(root_of(&sandbox.path("mono/lib/src"), &env, &ctx)),
            sandbox.root("mono")
        );
        // Outside it, the markers decide again.
        assert_eq!(
            string(root_of(&sandbox.path("mono"), &env, &ctx)),
            sandbox.root("mono")
        );
        assert_eq!(string(root_of(&sandbox.path(""), &env, &ctx)), sandbox.root(""));
    }

    #[test]
    fn the_nearest_declared_root_wins_over_one_further_up() {
        let sandbox = Sandbox::new("declared-nearest");
        sandbox.dir("outer/inner/src");
        let (ctx, env) = editor();
        run(
            &format!(
                "(setq project-roots (list {:?} {:?}))",
                sandbox.path("outer"),
                sandbox.path("outer/inner")
            ),
            &env,
            &ctx,
        );
        assert_eq!(
            string(root_of(&sandbox.path("outer/inner/src"), &env, &ctx)),
            sandbox.root("outer/inner")
        );
    }

    #[test]
    fn a_declared_root_is_a_directory_not_a_prefix_of_names() {
        // ~/src/app is no root of ~/src/apple.
        let sandbox = Sandbox::new("declared-prefix");
        sandbox.dir("app").dir("apple");
        let (ctx, env) = editor();
        run("(setq project-markers nil)", &env, &ctx);
        run(
            &format!("(setq project-roots (list {:?}))", sandbox.path("app")),
            &env,
            &ctx,
        );
        assert_eq!(root_of(&sandbox.path("apple"), &env, &ctx), LispExp::nil());
    }

    #[test]
    fn the_root_is_found_from_the_current_buffer_s_file() {
        let sandbox = Sandbox::new("buffer-file");
        sandbox.dir(".git").file("src/main.rs", "fn main() {}\n");
        let (ctx, env) = editor();
        run(
            &format!("(find-file {:?})", sandbox.path("src/main.rs")),
            &env,
            &ctx,
        );
        assert_eq!(string(run("(project-root)", &env, &ctx)), sandbox.root(""));
    }

    #[test]
    fn declaring_a_root_keeps_it_in_the_session_and_makes_it_known() {
        let sandbox = Sandbox::new("declare");
        sandbox.dir(".git").dir("tool/src");
        let (ctx, env) = editor();
        run(
            &format!("(project-declare-root {:?})", sandbox.path("tool")),
            &env,
            &ctx,
        );
        assert_eq!(
            strings(&run("(session-value 'project-declared-roots)", &env, &ctx)),
            vec![sandbox.root("tool")]
        );
        assert_eq!(
            string(root_of(&sandbox.path("tool/src"), &env, &ctx)),
            sandbox.root("tool")
        );
        assert_eq!(
            strings(&run("(project-known-projects)", &env, &ctx)),
            vec![sandbox.root("tool")]
        );
    }

    #[test]
    fn declaring_what_is_not_a_directory_is_refused() {
        let sandbox = Sandbox::new("declare-refused");
        sandbox.file("plain", "");
        let (ctx, env) = editor();
        run(
            &format!("(project-declare-root {:?})", sandbox.path("plain")),
            &env,
            &ctx,
        );
        assert!(ctx.get_echo_message().ends_with("is not a directory"));
        assert_eq!(
            run("(session-value 'project-declared-roots)", &env, &ctx),
            LispExp::nil()
        );
    }

    // -----------------------------------------------------------------------
    // The projects known
    // -----------------------------------------------------------------------

    #[test]
    fn a_command_outside_any_project_says_so_and_asks_nothing() {
        let sandbox = Sandbox::new("outside");
        let (ctx, env) = editor();
        run("(setq project-markers '(\"NO-SUCH-MARKER\"))", &env, &ctx);
        run(&format!("(cd {:?})", sandbox.path("")), &env, &ctx);
        for command in ["project-find-file", "project-grep", "project-compile"] {
            run(&format!("({command})"), &env, &ctx);
            assert!(
                ctx.get_echo_message().starts_with("Not in a project"),
                "{command}: {}",
                ctx.get_echo_message()
            );
            assert!(!ctx.minibuffer_is_open());
        }
        assert_eq!(run("(project-known-projects)", &env, &ctx), LispExp::nil());
    }

    #[test]
    fn a_project_command_makes_its_project_known_newest_first_and_once() {
        let one = Sandbox::new("known-one");
        let two = Sandbox::new("known-two");
        one.dir(".git");
        two.dir(".git");
        let (ctx, env) = editor();
        for sandbox in [&one, &two, &one] {
            run(&format!("(cd {:?})", sandbox.path("")), &env, &ctx);
            run("(project-grep)", &env, &ctx);
            run("(minibuffer-cancel)", &env, &ctx);
        }
        assert_eq!(
            strings(&run("(session-value 'project-known)", &env, &ctx)),
            vec![one.root(""), two.root("")]
        );
    }

    #[test]
    fn forgetting_a_project_takes_it_out_of_the_known_and_the_declared() {
        let sandbox = Sandbox::new("forget");
        sandbox.dir("tool");
        let (ctx, env) = editor();
        run(
            &format!("(project-declare-root {:?})", sandbox.path("tool")),
            &env,
            &ctx,
        );
        run("(project-forget)", &env, &ctx);
        answer(&sandbox.root("tool"), &env, &ctx);
        assert_eq!(run("(project-known-projects)", &env, &ctx), LispExp::nil());
        assert_eq!(
            run("(session-value 'project-declared-roots)", &env, &ctx),
            LispExp::nil()
        );
    }

    // -----------------------------------------------------------------------
    // The commands
    // -----------------------------------------------------------------------

    /// The files `project-find-file' offers for ROOT, asked again until the
    /// walk of the tree has ended.
    fn offered(root: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let index = run(&format!("(project--index {root:?})"), env, ctx);
            let walking = matches!(index.iter().next(), Some(LispExp::Symbol(s)) if s.as_ref() == "walking");
            if !walking && ctx.file_index_walks() == 0 {
                return strings(&run("(project-find-file--candidates \"\")", env, ctx));
            }
            assert!(Instant::now() < deadline, "the walk did not end");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn project_find_file_offers_the_root_s_files_and_opens_the_one_chosen() {
        let sandbox = Sandbox::new("find-file");
        sandbox
            .dir(".git")
            .file(".git/config", "")
            .file(".gitignore", "build/\n")
            .file("build/out.o", "")
            .file("crate/Cargo.toml", "")
            .file("crate/src/main.rs", "fn main() {}\n");
        let (ctx, env) = editor();
        run(
            &format!("(find-file {:?})", sandbox.path("crate/Cargo.toml")),
            &env,
            &ctx,
        );
        run("(project-find-file)", &env, &ctx);
        assert!(ctx.minibuffer_is_open());
        let mut found = offered(&sandbox.root(""), &env, &ctx);
        found.sort();
        assert_eq!(
            found,
            vec![".gitignore", "crate/Cargo.toml", "crate/src/main.rs"]
        );
        answer("crate/src/main.rs", &env, &ctx);
        crate::tests::file_io_tests::settle(&ctx, &env);
        assert_eq!(ctx.get_current_buffer_name(), "main.rs");
    }

    #[test]
    fn project_grep_searches_under_the_root() {
        let sandbox = Sandbox::new("grep");
        sandbox
            .dir(".git")
            .file("top.txt", "a needle at the top\n")
            .file("sub/deep.txt", "a needle below\n")
            .dir("sub/here");
        let (ctx, env) = editor();
        run(&format!("(cd {:?})", sandbox.path("sub/here")), &env, &ctx);
        run("(project-grep)", &env, &ctx);
        answer("needle", &env, &ctx);
        let buffer = ctx.get_current_buffer_name();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = run(&format!("(results-state {buffer:?})"), &env, &ctx);
            if state.iter().nth(3).is_some_and(|done| !done.is_nil()) {
                break;
            }
            assert!(Instant::now() < deadline, "the search did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
        let listing = ctx
            .with_buffer(&buffer, |buf| buf.text.to_string())
            .expect("the list");
        assert!(listing.contains("top.txt:1: a needle at the top"), "{listing}");
        assert!(listing.contains("deep.txt:1: a needle below"), "{listing}");
        assert_eq!(bare(&ctx.working_directory()), bare(&sandbox.path("sub/here")));
    }

    #[cfg(unix)]
    #[test]
    fn project_compile_runs_at_the_root_and_its_output_keeps_it() {
        let sandbox = Sandbox::new("compile");
        sandbox.dir(".git").dir("src");
        let (ctx, env) = editor();
        run(&format!("(cd {:?})", sandbox.path("src")), &env, &ctx);
        run("(project-compile)", &env, &ctx);
        answer("true", &env, &ctx);
        let output = string(run("compilation--buffer", &env, &ctx));
        settle(&ctx);
        assert_eq!(bare(&ctx.directory_of(&output)), bare(&sandbox.root("")));
        assert_eq!(bare(&ctx.working_directory()), bare(&sandbox.path("src")));
    }

    #[cfg(unix)]
    #[test]
    fn project_shell_command_runs_at_the_root() {
        let sandbox = Sandbox::new("shell");
        sandbox.dir(".git").dir("src");
        let (ctx, env) = editor();
        run(&format!("(cd {:?})", sandbox.path("src")), &env, &ctx);
        run("(project-shell-command)", &env, &ctx);
        answer("true", &env, &ctx);
        settle(&ctx);
        assert_eq!(
            bare(&ctx.directory_of("*Shell Output*")),
            bare(&sandbox.root(""))
        );
    }

    #[test]
    fn project_switch_with_none_known_says_so() {
        let (ctx, env) = editor();
        run("(project-switch)", &env, &ctx);
        assert!(ctx.get_echo_message().starts_with("No projects known"));
        assert!(!ctx.minibuffer_is_open());
    }

    #[test]
    fn project_switch_offers_the_projects_known_and_finds_a_file_in_the_one_chosen() {
        let sandbox = Sandbox::new("switch");
        sandbox.file("tool/README", "");
        let (ctx, env) = editor();
        run(
            &format!("(project-declare-root {:?})", sandbox.path("tool")),
            &env,
            &ctx,
        );
        run("(project-switch)", &env, &ctx);
        assert_eq!(
            strings(&run("(project--known-candidates \"tool\")", &env, &ctx)),
            vec![sandbox.root("tool")]
        );
        answer(&sandbox.root("tool"), &env, &ctx);
        assert!(ctx.minibuffer_is_open(), "the file prompt should be open");
        assert_eq!(
            string(run("project--find-root", &env, &ctx)),
            sandbox.root("tool")
        );
        assert_eq!(offered(&sandbox.root("tool"), &env, &ctx), vec!["README"]);
    }
}
