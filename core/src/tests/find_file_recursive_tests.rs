//! The recursive walk of `directory-files-recursive', and what
//! `dired-find-recursive' offers from the index of the tree: which directories
//! and files it leaves out, which ignore files it reads, and that it asks the
//! index again rather than keeping a list of its own.
//!
//! Design: `doc/architettura-editor.typ, <disco>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval, measure};
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
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/completion.lisp"),
            include_str!("../../lisp/features/dired.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// A directory tree built for one test and removed after it.
    struct Sandbox(std::path::PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "rsedit-ffr-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
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

        fn path(&self) -> String {
            self.0.display().to_string()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The PATHS half of a `(TRUNCATED PATHS)` answer, as plain strings.
    fn paths(answer: &LispExp<Ctx>) -> Vec<String> {
        answer
            .iter()
            .nth(1)
            .expect("a (TRUNCATED PATHS) pair")
            .iter()
            .map(|item| match item {
                LispExp::String(s) => s.to_string(),
                other => panic!("expected a path string, got {other:?}"),
            })
            .collect()
    }

    fn truncated(answer: &LispExp<Ctx>) -> bool {
        !answer.iter().next().expect("a TRUNCATED flag").is_nil()
    }

    // -----------------------------------------------------------------------
    // The walk
    // -----------------------------------------------------------------------

    #[test]
    fn every_file_is_found_at_every_depth() {
        let sandbox = Sandbox::new("depth");
        sandbox
            .file("top.txt", "")
            .file("a/one.txt", "")
            .file("a/b/two.txt", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(directory-files-recursive "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert_eq!(paths(&answer), vec!["a/b/two.txt", "a/one.txt", "top.txt"]);
    }

    #[test]
    fn directories_are_walked_but_not_listed() {
        let sandbox = Sandbox::new("dirs");
        sandbox.file("a/one.txt", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(directory-files-recursive "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        // `a` is not an answer to "which file do you want to open".
        assert_eq!(paths(&answer), vec!["a/one.txt"]);
    }

    #[test]
    fn a_pruned_directory_is_not_descended_into() {
        let sandbox = Sandbox::new("prune");
        sandbox.file("keep.txt", "").file("skip/gone.txt", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(
                r#"(directory-files-recursive "{}" 100 '("skip"))"#,
                sandbox.path()
            ),
            &env,
            &ctx,
        );
        assert_eq!(paths(&answer), vec!["keep.txt"]);
    }

    #[test]
    fn pruning_is_by_name_at_every_depth_not_by_path() {
        let sandbox = Sandbox::new("prune-deep");
        sandbox
            .file("keep.txt", "")
            .file("a/b/target/gone.txt", "")
            .file("a/b/kept.txt", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(
                r#"(directory-files-recursive "{}" 100 '("target"))"#,
                sandbox.path()
            ),
            &env,
            &ctx,
        );
        assert_eq!(paths(&answer), vec!["a/b/kept.txt", "keep.txt"]);
    }

    #[test]
    fn the_limit_stops_the_walk_and_says_so() {
        let sandbox = Sandbox::new("limit");
        for n in 0..10 {
            sandbox.file(&format!("file{n}.txt"), "");
        }
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(directory-files-recursive "{}" 4)"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert_eq!(paths(&answer).len(), 4);
        // The flag is the whole reason the answer is a pair. A truncated list
        // looks exactly like a complete one, and a file missing from it looks
        // exactly like a file that is not there.
        assert!(truncated(&answer));
    }

    #[test]
    fn a_complete_walk_says_it_is_complete() {
        let sandbox = Sandbox::new("complete");
        sandbox.file("only.txt", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(directory-files-recursive "{}" 100)"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert!(!truncated(&answer));
    }

    #[test]
    fn walking_something_that_is_not_a_directory_answers_nil() {
        let sandbox = Sandbox::new("notdir");
        sandbox.file("a-file", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(directory-files-recursive "{}/a-file")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert!(answer.is_nil());
    }

    #[test]
    fn an_empty_directory_is_an_empty_list_not_a_failure() {
        let sandbox = Sandbox::new("empty");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(directory-files-recursive "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert!(paths(&answer).is_empty());
        assert!(!truncated(&answer));
    }

    // -----------------------------------------------------------------------
    // Reading a file without opening it
    // -----------------------------------------------------------------------

    #[test]
    fn a_file_can_be_read_without_becoming_a_buffer() {
        let sandbox = Sandbox::new("read");
        sandbox.file("notes", "one\ntwo\n");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(read-file-to-string "{}/notes")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert_eq!(answer, LispExp::string("one\ntwo\n".to_string()));
        // Nothing to close afterwards: the point of this over `find-file'.
        assert!(!ctx.has_buffer("notes"));
    }

    #[test]
    fn a_missing_file_reads_as_nil() {
        let sandbox = Sandbox::new("missing");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(read-file-to-string "{}/nope")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert!(answer.is_nil());
    }

    // -----------------------------------------------------------------------
    // What C-x C-r offers
    // -----------------------------------------------------------------------

    /// The index `dired-find-recursive' offers for DIRECTORY, asked again
    /// until its walk has ended, as (STATE PATHS).
    fn offered(directory: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> (String, Vec<String>) {
        let src = format!("(find-file-recursive--index {directory:?})");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let answer = run(&src, env, ctx);
            let state = match answer.iter().next() {
                Some(LispExp::Symbol(state)) => state.to_string(),
                other => panic!("expected a state, got {other:?}"),
            };
            if state != "walking" && ctx.file_index_walks() == 0 {
                return (state, paths(&answer));
            }
            assert!(std::time::Instant::now() < deadline, "the walk did not end");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn a_directory_named_in_gitignore_is_not_descended_into() {
        let sandbox = Sandbox::new("gitignore-dir");
        sandbox
            .file(".gitignore", "build/\n")
            .file("keep.txt", "")
            .file("build/gone.txt", "");
        let (ctx, env) = editor();
        let found = offered(&sandbox.path(), &env, &ctx).1;
        assert!(found.contains(&"keep.txt".to_string()));
        assert!(!found.iter().any(|p| p.starts_with("build/")));
    }

    #[test]
    fn a_bare_name_in_gitignore_leaves_out_a_directory_too() {
        let sandbox = Sandbox::new("gitignore-bare");
        sandbox
            .file(".gitignore", "coverage\n")
            .file("keep.txt", "")
            .file("coverage/gone.txt", "");
        let (ctx, env) = editor();
        // `.gitignore` is itself offered: it is a file people edit.
        assert_eq!(
            offered(&sandbox.path(), &env, &ctx),
            (
                "complete".into(),
                vec![".gitignore".into(), "keep.txt".into()]
            )
        );
    }

    #[test]
    fn a_star_dot_extension_pattern_drops_those_files() {
        let sandbox = Sandbox::new("gitignore-ext");
        sandbox
            .file(".gitignore", "*.log\n")
            .file("keep.txt", "")
            .file("noise.log", "")
            .file("deep/also.log", "");
        let (ctx, env) = editor();
        assert_eq!(
            offered(&sandbox.path(), &env, &ctx).1,
            vec![".gitignore", "keep.txt"]
        );
    }

    #[test]
    fn comments_and_blank_lines_are_not_patterns() {
        let sandbox = Sandbox::new("gitignore-comments");
        sandbox
            .file(".gitignore", "# a comment\n\n   \n*.log\n")
            .file("keep.txt", "")
            .file("noise.log", "");
        let (ctx, env) = editor();
        assert_eq!(
            offered(&sandbox.path(), &env, &ctx).1,
            vec![".gitignore", "keep.txt"]
        );
    }

    #[test]
    fn a_gitignore_written_on_windows_still_matches() {
        let sandbox = Sandbox::new("gitignore-crlf");
        sandbox
            .file(".gitignore", "build/\r\n*.log\r\n")
            .file("keep.txt", "")
            .file("noise.log", "")
            .file("build/gone.txt", "");
        let (ctx, env) = editor();
        assert_eq!(
            offered(&sandbox.path(), &env, &ctx).1,
            vec![".gitignore", "keep.txt"]
        );
    }

    #[test]
    fn a_negation_brings_a_file_back_and_a_pattern_not_understood_drops_nothing() {
        let sandbox = Sandbox::new("gitignore-negation");
        sandbox
            .file(".gitignore", "*.log\n!keep-me.log\n[[:alpha:]]*\n")
            .file("keep-me.log", "")
            .file("noise.log", "")
            .file("other.txt", "");
        let (ctx, env) = editor();
        assert_eq!(
            offered(&sandbox.path(), &env, &ctx).1,
            vec![".gitignore", "keep-me.log", "other.txt"]
        );
    }

    #[test]
    fn a_gitignore_below_the_top_counts_under_its_own_directory() {
        let sandbox = Sandbox::new("gitignore-nested");
        sandbox
            .file("top.tmp", "")
            .file("sub/.gitignore", "*.tmp\n")
            .file("sub/inner.tmp", "")
            .file("sub/keep.rs", "");
        let (ctx, env) = editor();
        assert_eq!(
            offered(&sandbox.path(), &env, &ctx).1,
            vec!["sub/.gitignore", "sub/keep.rs", "top.tmp"]
        );
    }

    #[test]
    fn the_built_in_ignore_list_applies_without_a_gitignore() {
        let sandbox = Sandbox::new("builtin-ignore");
        sandbox
            .file("keep.txt", "")
            .file(".git/objects/abc", "")
            .file("target/debug/thing", "")
            .file("node_modules/pkg/index.js", "");
        let (ctx, env) = editor();
        assert_eq!(offered(&sandbox.path(), &env, &ctx).1, vec!["keep.txt"]);
    }

    #[test]
    fn the_ignore_files_read_are_a_setting() {
        let sandbox = Sandbox::new("ignore-files");
        sandbox
            .file(".gitignore", "*.log\n")
            .file(".ignore", "*.tmp\n")
            .file("a.log", "")
            .file("b.tmp", "");
        let (ctx, env) = editor();
        run(
            "(setq find-file-recursive-ignore-files '(\".ignore\"))",
            &env,
            &ctx,
        );
        assert_eq!(
            offered(&sandbox.path(), &env, &ctx).1,
            vec![".gitignore", ".ignore", "a.log"]
        );
    }

    #[test]
    fn the_candidates_are_asked_of_the_index_at_each_call() {
        let sandbox = Sandbox::new("candidates");
        sandbox.file("alpha.txt", "");
        let (ctx, env) = editor();
        offered(&sandbox.path(), &env, &ctx);
        run(
            &format!("(setq dired--recursive-directory {:?})", sandbox.path()),
            &env,
            &ctx,
        );
        let candidates = |ctx: &Ctx| -> Vec<String> {
            run("(dired-find-recursive--candidates \"\")", &env, ctx)
                .iter()
                .map(|item| match item {
                    LispExp::String(s) => s.to_string(),
                    other => panic!("expected a path, got {other:?}"),
                })
                .collect()
        };
        assert_eq!(candidates(&ctx), vec!["alpha.txt"]);
        sandbox.file("beta.txt", "");
        ctx.forget_file_index(None);
        offered(&sandbox.path(), &env, &ctx);
        assert_eq!(candidates(&ctx), vec!["alpha.txt", "beta.txt"]);
    }

    #[test]
    fn an_empty_tree_is_said_to_have_no_files_once_its_walk_has_ended() {
        let sandbox = Sandbox::new("empty-tree");
        let (ctx, env) = editor();
        run(&format!("(cd {:?})", sandbox.path()), &env, &ctx);
        offered(&sandbox.path(), &env, &ctx);
        run("(dired-find-recursive)", &env, &ctx);
        assert!(
            ctx.get_echo_message().starts_with("No files under"),
            "{}",
            ctx.get_echo_message()
        );
    }

    // -----------------------------------------------------------------------
    // Dropping files by suffix, and what it costs not to
    // -----------------------------------------------------------------------

    #[test]
    fn a_file_with_an_excluded_suffix_is_not_listed() {
        let sandbox = Sandbox::new("suffix");
        sandbox.file("keep.rs", "").file("drop.o", "");
        sandbox
            .file("nested/also.o", "")
            .file("nested/keep.txt", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(
                r#"(directory-files-recursive "{}" 100 nil '(".o"))"#,
                sandbox.path()
            ),
            &env,
            &ctx,
        );
        assert_eq!(paths(&answer), vec!["keep.rs", "nested/keep.txt"]);
    }

    /// The limit bounds what the caller is *offered*, not what the walk stepped
    /// over. Counting excluded files against it would mean a directory of build
    /// output could use up the whole limit and leave no room for the source
    /// beside it -- the same argument that makes pruning a walk-time job.
    #[test]
    fn an_excluded_file_does_not_count_against_the_limit() {
        let sandbox = Sandbox::new("suffix-limit");
        for n in 0..20 {
            sandbox.file(&format!("junk{n}.o"), "");
        }
        for n in 0..3 {
            sandbox.file(&format!("src{n}.rs"), "");
        }
        let (ctx, env) = editor();
        let answer = run(
            &format!(
                r#"(directory-files-recursive "{}" 3 nil '(".o"))"#,
                sandbox.path()
            ),
            &env,
            &ctx,
        );
        assert_eq!(paths(&answer), vec!["src0.rs", "src1.rs", "src2.rs"]);
        assert!(!truncated(&answer), "three kept files is the whole of it");
    }

    /// Asking for the candidates must not cost an evaluation per file per
    /// .gitignore pattern: a real project -- a few thousand files, forty
    /// `*.ext` patterns -- would spend a command's entire fuel before the
    /// prompt opened, and `C-x C-r` would answer with nothing at all, like a
    /// project with no files.
    ///
    /// Measured rather than merely run, because that failure is not a wrong
    /// answer but the right one costing too much. At this size, applying the
    /// patterns in Lisp would cost about 1.2 million units.
    #[test]
    fn gathering_candidates_is_cheap_against_a_long_gitignore() {
        let sandbox = Sandbox::new("fuel");
        for n in 0..600 {
            sandbox.file(&format!("d{}/f{n}.rs", n % 20), "");
        }
        let patterns: String = (0..40).map(|k| format!("*.ext{k}\n")).collect();
        sandbox.file(".gitignore", &patterns);

        let (ctx, env) = editor();
        assert_eq!(
            offered(&sandbox.path(), &env, &ctx).1.len(),
            601,
            "600 sources and the .gitignore"
        );
        let src = format!(r#"(find-file-recursive--index "{}")"#, sandbox.path());
        let (answer, spent) = measure(&ctx.fuel_meter(), || {
            let ast = Parser::new(&format!("(progn {src})"))
                .next()
                .expect("parse");
            eval(&ast, env.clone(), &ctx)
        });
        answer.expect("asking the index should succeed");
        assert!(
            spent < 200_000,
            "gathering 600 candidates behind 40 patterns cost {spent} units"
        );
    }
}
