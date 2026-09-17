//! What `find-file` does with a path that is not an ordinary readable file.
//!
//! Three cases that used to be one error message: a directory, a file that
//! does not exist yet, and a file that is already open.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
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
        (ctx, env)
    }

    struct Sandbox(std::path::PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "rsedit-ff-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("sandbox");
            Sandbox(root)
        }
        fn file(&self, relative: &str, contents: &str) -> String {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("sandbox subdirectory");
            }
            std::fs::write(&path, contents).expect("sandbox file");
            path.display().to_string()
        }
        fn path(&self) -> String {
            self.0.display().to_string()
        }
        fn join(&self, relative: &str) -> String {
            self.0.join(relative).display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn name_of(answer: &LispExp<Ctx>) -> String {
        match answer {
            LispExp::String(s) => s.to_string(),
            other => panic!("expected a buffer name, got {other:?}"),
        }
    }

    // ----------------------------------------------------------------
    // A file that is not there yet
    // ----------------------------------------------------------------

    #[test]
    fn opening_a_file_that_does_not_exist_makes_an_empty_buffer() {
        let sandbox = Sandbox::new("new");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(find-file "{}")"#, sandbox.join("fresh.txt")),
            &env,
            &ctx,
        );
        assert_eq!(name_of(&answer), "fresh.txt");
        assert_eq!(run("(buffer-string)", &env, &ctx), LispExp::string(String::new()));
    }

    #[test]
    fn a_new_file_buffer_is_not_modified() {
        // Nothing has been written, so leaving without saving loses nothing
        // and should ask nothing.
        let sandbox = Sandbox::new("new-clean");
        let (ctx, env) = editor();
        run(
            &format!(r#"(find-file "{}")"#, sandbox.join("fresh.txt")),
            &env,
            &ctx,
        );
        assert!(run("(buffer-modified-p)", &env, &ctx).is_nil());
    }

    #[test]
    fn a_new_file_buffer_remembers_where_it_goes() {
        let sandbox = Sandbox::new("new-path");
        let (ctx, env) = editor();
        let target = sandbox.join("fresh.txt");
        run(&format!(r#"(find-file "{target}")"#), &env, &ctx);
        assert_eq!(
            run("(buffer-file-name)", &env, &ctx),
            LispExp::string(target.clone())
        );
    }

    #[test]
    fn saving_a_new_file_buffer_creates_the_file() {
        // The whole point of remembering the path.
        let sandbox = Sandbox::new("new-save");
        let (ctx, env) = editor();
        let target = sandbox.join("fresh.txt");
        run(&format!(r#"(find-file "{target}")"#), &env, &ctx);
        run(r#"(insert "written") (save-buffer)"#, &env, &ctx);
        assert_eq!(
            std::fs::read_to_string(&target).expect("the file should exist now"),
            "written"
        );
    }

    #[test]
    fn a_new_file_gets_its_mode_from_its_name() {
        // The mode comes from the path, which is all `auto-mode` ever had to
        // go on -- there is no content to inspect either way.
        let sandbox = Sandbox::new("new-mode");
        let (ctx, env) = editor();
        // Registered here rather than relying on a shipped module: the
        // association is Lisp policy, and this editor loads none.
        run(
            r#"(make-mode 'toy-mode) (add-auto-mode "\\.toy$" 'toy-mode)"#,
            &env,
            &ctx,
        );
        run(
            &format!(r#"(find-file "{}")"#, sandbox.join("fresh.toy")),
            &env,
            &ctx,
        );
        assert_eq!(
            run("(major-mode)", &env, &ctx),
            LispExp::symbol("toy-mode".to_string())
        );
    }

    // ----------------------------------------------------------------
    // A directory
    // ----------------------------------------------------------------

    #[test]
    fn a_directory_is_handed_to_the_callback() {
        let sandbox = Sandbox::new("dir");
        let (ctx, env) = editor();
        run(
            r#"(setq *opened* nil)
               (setq *open-directory-callback* (lambda (path) (setq *opened* path) "done"))"#,
            &env,
            &ctx,
        );
        let answer = run(
            &format!(r#"(find-file "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert_eq!(
            run("*opened*", &env, &ctx),
            LispExp::string(sandbox.path())
        );
        assert_eq!(name_of(&answer), "done", "the callback's answer is returned");
    }

    #[test]
    fn a_directory_with_no_callback_says_so_rather_than_failing_obscurely() {
        let sandbox = Sandbox::new("dir-none");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(find-file "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert!(answer.is_nil());
        // And no buffer was made for it.
        assert!(ctx.get_buffer("dir-none").is_none());
    }

    #[test]
    fn the_editor_never_mentions_dired() {
        // The variable is the whole of what the editor knows about file
        // managers. Anything can take the job by setting it.
        let sandbox = Sandbox::new("dir-other");
        let (ctx, env) = editor();
        run(
            r#"(setq *open-directory-callback* (lambda (path) "my-own-lister"))"#,
            &env,
            &ctx,
        );
        let answer = run(
            &format!(r#"(find-file "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert_eq!(name_of(&answer), "my-own-lister");
    }

    // ----------------------------------------------------------------
    // Files already open, and names that collide
    // ----------------------------------------------------------------

    #[test]
    fn opening_the_same_file_twice_returns_to_the_same_buffer() {
        let sandbox = Sandbox::new("same");
        let path = sandbox.file("notes.txt", "original");
        let (ctx, env) = editor();
        let first = name_of(&run(&format!(r#"(find-file "{path}")"#), &env, &ctx));
        run(r#"(end-of-buffer) (insert " edited")"#, &env, &ctx);
        let second = name_of(&run(&format!(r#"(find-file "{path}")"#), &env, &ctx));

        assert_eq!(first, second);
        // Re-reading the file would have thrown the edit away.
        assert_eq!(
            run("(buffer-string)", &env, &ctx),
            LispExp::string("original edited".to_string())
        );
    }

    #[test]
    fn two_files_with_the_same_name_both_get_a_buffer() {
        // The case that makes unique naming worth doing: `mod.rs` twice.
        let sandbox = Sandbox::new("collide");
        let one = sandbox.file("a/mod.rs", "first");
        let two = sandbox.file("b/mod.rs", "second");
        let (ctx, env) = editor();
        let first = name_of(&run(&format!(r#"(find-file "{one}")"#), &env, &ctx));
        let second = name_of(&run(&format!(r#"(find-file "{two}")"#), &env, &ctx));

        assert_eq!(first, "mod.rs");
        assert_eq!(second, "mod.rs<2>");
        assert_eq!(
            run("(buffer-string)", &env, &ctx),
            LispExp::string("second".to_string()),
            "and the second one really is the second file"
        );
    }

    #[test]
    fn an_existing_file_still_opens_the_way_it_always_did() {
        let sandbox = Sandbox::new("plain");
        let path = sandbox.file("notes.txt", "hello");
        let (ctx, env) = editor();
        let answer = run(&format!(r#"(find-file "{path}")"#), &env, &ctx);
        assert_eq!(name_of(&answer), "notes.txt");
        assert_eq!(
            run("(buffer-string)", &env, &ctx),
            LispExp::string("hello".to_string())
        );
    }
}
