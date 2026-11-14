//! The editor's working directory: where it starts, what `cd` moves and what
//! it leaves alone, every way a relative name reaches the disk, a buffer with
//! a directory of its own, and a call given a directory by `with-directory`.
//!
//! Design: `doc/architettura-editor.typ, <directory-di-lavoro>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// A directory of this test's own, removed when it goes.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rsedit-cwd-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("creating the sandbox");
            Sandbox(dir)
        }

        /// Writes a file called NAME holding CONTENTS.
        fn file(&self, name: &str, contents: &str) -> &Self {
            std::fs::write(self.0.join(name), contents).expect("writing a file");
            self
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).display().to_string()
        }

        fn root(&self) -> String {
            self.0.display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env")
    }

    /// An editor whose working directory is SANDBOX's.
    fn editor_in(sandbox: &Sandbox) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor();
        run(&format!(r#"(cd "{}")"#, sandbox.root()), &env, &ctx);
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

    fn string(value: LispExp<Ctx>) -> String {
        match value {
            LispExp::String(text) => text.to_string(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    fn process_directory() -> String {
        std::env::current_dir()
            .expect("a working directory")
            .display()
            .to_string()
    }

    // ---------------- where it is, and what moves it ----------------

    #[test]
    fn it_starts_where_the_process_is() {
        let (ctx, env) = editor();
        assert_eq!(
            string(run("(working-directory)", &env, &ctx)),
            process_directory()
        );
    }

    #[test]
    fn cd_moves_the_editor_and_leaves_the_process_alone() {
        let sandbox = Sandbox::new("moves");
        let (ctx, env) = editor();
        let before = process_directory();

        let answer = run(&format!(r#"(cd "{}")"#, sandbox.root()), &env, &ctx);

        assert_eq!(string(answer), sandbox.root());
        assert_eq!(
            string(run("(working-directory)", &env, &ctx)),
            sandbox.root()
        );
        assert_eq!(process_directory(), before, "the process must not move");
        assert_eq!(
            ctx.get_echo_message(),
            format!("Directory {}", sandbox.root())
        );
    }

    #[test]
    fn cd_reads_a_relative_directory_in_the_one_it_leaves() {
        let sandbox = Sandbox::new("nested");
        std::fs::create_dir(sandbox.path("inner")).expect("making a directory");
        let (ctx, env) = editor_in(&sandbox);

        run(r#"(cd "inner")"#, &env, &ctx);
        assert_eq!(ctx.working_directory(), sandbox.path("inner"));
        run(r#"(cd "..")"#, &env, &ctx);
        assert_eq!(ctx.working_directory(), sandbox.root());
    }

    #[test]
    fn cd_to_what_is_not_a_directory_changes_nothing() {
        let sandbox = Sandbox::new("refused");
        sandbox.file("plain", "");
        let (ctx, env) = editor();
        let before = ctx.working_directory();

        for target in [sandbox.path("plain"), sandbox.path("missing")] {
            assert_eq!(
                run(&format!(r#"(cd "{target}")"#), &env, &ctx),
                LispExp::nil()
            );
            assert_eq!(ctx.working_directory(), before);
            assert_eq!(
                ctx.get_echo_message(),
                format!("{target} is not a directory")
            );
        }
    }

    #[test]
    fn pwd_says_where_names_are_read() {
        let sandbox = Sandbox::new("pwd");
        let (ctx, env) = editor_in(&sandbox);
        ctx.set_echo_message("");

        assert_eq!(string(run("(pwd)", &env, &ctx)), sandbox.root());
        assert_eq!(
            ctx.get_echo_message(),
            format!("Directory {}", sandbox.root())
        );
    }

    // ---------------- every way a relative name reaches the disk ----------------

    #[test]
    fn expand_file_name_reads_a_relative_name_in_it() {
        let sandbox = Sandbox::new("expand");
        let (ctx, env) = editor_in(&sandbox);

        assert_eq!(
            string(run(r#"(expand-file-name "notes.txt")"#, &env, &ctx)),
            sandbox.path("notes.txt")
        );
        // A relative DIRECTORY is read there too.
        assert_eq!(
            string(run(r#"(expand-file-name "x" "inner")"#, &env, &ctx)),
            sandbox.path("inner/x")
        );
    }

    #[test]
    fn find_file_opens_a_relative_name_in_it() {
        let sandbox = Sandbox::new("find-file");
        sandbox.file("notes.txt", "hello");
        let (ctx, env) = editor_in(&sandbox);

        let name = string(run(r#"(find-file "notes.txt")"#, &env, &ctx));
        assert_eq!(
            ctx.with_buffer(&name, |buf| buf.file_path.clone())
                .flatten(),
            Some(sandbox.path("notes.txt"))
        );
    }

    #[test]
    fn the_disk_primitives_read_a_relative_name_in_it() {
        let sandbox = Sandbox::new("disk");
        sandbox.file("notes.txt", "hello");
        let (ctx, env) = editor_in(&sandbox);

        assert_eq!(
            run(r#"(file-exists-p "notes.txt")"#, &env, &ctx),
            LispExp::t()
        );
        assert_eq!(
            string(run(r#"(read-file-to-string "notes.txt")"#, &env, &ctx)),
            "hello"
        );
    }

    #[test]
    fn directory_files_recursive_walks_it_by_default() {
        let sandbox = Sandbox::new("walk");
        sandbox.file("notes.txt", "");
        let (ctx, env) = editor_in(&sandbox);

        assert_eq!(
            run("(nth 1 (directory-files-recursive))", &env, &ctx),
            run(r#"(list "notes.txt")"#, &env, &ctx)
        );
    }

    #[test]
    fn list_dir_lists_it_by_default() {
        let sandbox = Sandbox::new("list-dir");
        sandbox.file("notes.txt", "");
        let (ctx, env) = editor_in(&sandbox);

        assert_eq!(
            run("(list-dir)", &env, &ctx),
            run(r#"(list "notes.txt")"#, &env, &ctx)
        );
    }

    #[test]
    fn eval_file_reads_a_relative_path_in_it() {
        let sandbox = Sandbox::new("eval-file");
        sandbox.file("mine.lisp", "(setq evaluated-here t)");
        let (ctx, env) = editor_in(&sandbox);

        run(r#"(eval-file "mine.lisp")"#, &env, &ctx);
        assert_eq!(run("evaluated-here", &env, &ctx), LispExp::t());
    }

    #[test]
    fn a_file_name_completes_in_it() {
        let sandbox = Sandbox::new("complete");
        sandbox.file("notes.txt", "");
        let (ctx, _env) = editor_in(&sandbox);

        assert_eq!(
            crate::paths::file_completions("no", &ctx.default_directory()),
            vec!["notes.txt".to_string()]
        );
    }

    // ---------------- a buffer with a directory of its own ----------------

    #[test]
    fn a_buffer_with_a_directory_of_its_own_reads_names_there() {
        let own = Sandbox::new("own");
        let editor_wide = Sandbox::new("editor-wide");
        let (ctx, env) = editor_in(&editor_wide);
        run(r#"(buffer-create "*output*")"#, &env, &ctx);
        ctx.with_buffer_mut("*output*", |buf| buf.directory = Some(own.root()));

        assert_eq!(ctx.directory_of("*output*"), own.root());
        assert_eq!(ctx.directory_of("*scratch*"), editor_wide.root());
        run(r#"(switch-to-buffer "*output*")"#, &env, &ctx);
        assert_eq!(
            string(run(r#"(expand-file-name "x")"#, &env, &ctx)),
            own.path("x")
        );
        // `cd` moves the editor's, which the buffer does not follow.
        let elsewhere = Sandbox::new("elsewhere");
        run(&format!(r#"(cd "{}")"#, elsewhere.root()), &env, &ctx);
        assert_eq!(ctx.working_directory(), elsewhere.root());
        assert_eq!(ctx.directory_of("*output*"), own.root());
    }

    // ---------------- a call with a directory of its own ----------------

    /// The call `(with-directory DIRECTORY (lambda () BODY))`.
    fn within(directory: &str, body: &str) -> String {
        format!(r#"(with-directory "{directory}" (lambda () {body}))"#)
    }

    #[test]
    fn with_directory_reads_names_there_and_moves_nothing() {
        let inside = Sandbox::new("within-inside");
        let editor_wide = Sandbox::new("within-editor-wide");
        let (ctx, env) = editor_in(&editor_wide);

        assert_eq!(
            string(run(
                &within(&inside.root(), r#"(expand-file-name "x")"#),
                &env,
                &ctx
            )),
            inside.path("x")
        );
        assert_eq!(
            string(run(
                &within(&inside.root(), "(working-directory)"),
                &env,
                &ctx
            )),
            inside.root()
        );
        assert_eq!(ctx.working_directory(), editor_wide.root());
        assert_eq!(ctx.directory_of("*scratch*"), editor_wide.root());
        assert_eq!(
            string(run("(working-directory)", &env, &ctx)),
            editor_wide.root()
        );
    }

    #[test]
    fn with_directory_returns_what_the_function_returns() {
        let inside = Sandbox::new("within-returns");
        let (ctx, env) = editor();

        assert_eq!(
            run(&within(&inside.root(), "(+ 1 2)"), &env, &ctx),
            LispExp::number(3.0)
        );
    }

    #[test]
    fn with_directory_wins_over_a_buffer_with_a_directory_of_its_own() {
        let own = Sandbox::new("within-own");
        let inside = Sandbox::new("within-over-own");
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "*output*") (switch-to-buffer "*output*")"#,
            &env,
            &ctx,
        );
        ctx.with_buffer_mut("*output*", |buf| buf.directory = Some(own.root()));

        assert_eq!(
            string(run(
                &within(&inside.root(), r#"(expand-file-name "x")"#),
                &env,
                &ctx
            )),
            inside.path("x")
        );
        // The buffer's own is the buffer's, and stays.
        assert_eq!(ctx.directory_of("*output*"), own.root());
        assert_eq!(ctx.default_directory(), own.root());
    }

    #[test]
    fn with_directory_holds_across_a_change_of_buffer() {
        let inside = Sandbox::new("within-switch");
        let (ctx, env) = editor();
        run(r#"(buffer-create "*other*")"#, &env, &ctx);

        let body = r#"(with-current-buffer "*other*" (lambda () (working-directory)))"#;
        assert_eq!(
            string(run(&within(&inside.root(), body), &env, &ctx)),
            inside.root()
        );
    }

    #[test]
    fn with_directory_reads_a_relative_directory_where_it_is_called_and_nests() {
        let outer = Sandbox::new("within-nested");
        std::fs::create_dir(outer.path("inner")).expect("making a directory");
        let (ctx, env) = editor();

        let body = r#"(list (with-directory "inner" (lambda () (working-directory))) (working-directory))"#;
        assert_eq!(
            run(&within(&outer.root(), body), &env, &ctx),
            run(
                &format!(r#"(list "{}" "{}")"#, outer.path("inner"), outer.root()),
                &env,
                &ctx
            )
        );
    }

    #[test]
    fn with_directory_puts_the_directory_back_when_the_function_fails() {
        let inside = Sandbox::new("within-fails");
        let (ctx, env) = editor();
        let before = ctx.default_directory();

        assert!(try_run(&within(&inside.root(), "(error \"no\")"), &env, &ctx).is_err());
        assert_eq!(ctx.default_directory(), before);
    }

    #[test]
    fn with_directory_refuses_what_is_not_a_directory_without_calling() {
        let sandbox = Sandbox::new("within-refused");
        sandbox.file("plain", "");
        let (ctx, env) = editor();
        run("(setq called nil)", &env, &ctx);

        for target in [sandbox.path("plain"), sandbox.path("missing")] {
            let why = try_run(&within(&target, "(setq called t)"), &env, &ctx)
                .expect_err("not a directory");
            assert!(
                format!("{why:?}").contains(&format!("{target} is not a directory")),
                "{why:?}"
            );
        }
        assert_eq!(run("called", &env, &ctx), LispExp::nil());
    }

    #[test]
    fn cd_inside_with_directory_moves_the_call_s_directory_and_not_the_editor_s() {
        let outer = Sandbox::new("within-cd");
        std::fs::create_dir(outer.path("inner")).expect("making a directory");
        let editor_wide = Sandbox::new("within-cd-editor-wide");
        let (ctx, env) = editor_in(&editor_wide);

        assert_eq!(
            string(run(
                &within(&outer.root(), r#"(cd "inner") (working-directory)"#),
                &env,
                &ctx
            )),
            outer.path("inner")
        );
        assert_eq!(ctx.working_directory(), editor_wide.root());
    }

    #[test]
    fn a_job_started_inside_with_directory_steps_and_reports_there() {
        let inside = Sandbox::new("within-job");
        let editor_wide = Sandbox::new("within-job-editor-wide");
        let (ctx, env) = editor_in(&editor_wide);
        run(
            "(setq stepped nil) (setq told nil)
             (defun where-stepped () (setq stepped (working-directory)))
             (defun where-told (name finished) (setq told (working-directory)))",
            &env,
            &ctx,
        );
        run(
            &within(
                &inside.root(),
                "(background-call 'scoped 'where-stepped nil 'where-told)",
            ),
            &env,
            &ctx,
        );
        // Out of the call: the step and the callback run after it returned.
        ctx.work().run_until_idle(&ctx);
        ctx.run_owed_callbacks(&env);

        assert_eq!(string(run("stepped", &env, &ctx)), inside.root());
        assert_eq!(string(run("told", &env, &ctx)), inside.root());
        assert_eq!(ctx.working_directory(), editor_wide.root());
    }

    #[test]
    fn a_job_started_outside_with_directory_goes_by_the_editor_s() {
        let inside = Sandbox::new("within-job-outside");
        let editor_wide = Sandbox::new("within-job-outside-editor-wide");
        let (ctx, env) = editor_in(&editor_wide);
        run(
            "(setq stepped nil)
             (defun where-stepped () (setq stepped (working-directory)))
             (background-call 'unscoped 'where-stepped)",
            &env,
            &ctx,
        );
        // Its step taken on a thread that is inside a call: it does not
        // take that call's directory.
        let _scope = crate::editor::DirectoryScope::enter(Some(inside.root()));
        ctx.work().run_until_idle(&ctx);

        assert_eq!(string(run("stepped", &env, &ctx)), editor_wide.root());
    }
}
