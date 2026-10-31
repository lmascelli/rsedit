//! The editor's working directory: where it starts, what `cd` moves and what
//! it leaves alone, every way a relative name reaches the disk, and a buffer
//! with a directory of its own.
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
}
