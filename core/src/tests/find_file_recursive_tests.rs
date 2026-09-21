//! The recursive walk, and the .gitignore reading built on top of it.
//!
//! The Lisp half lives in `dired.lisp` now rather than in a module of its own:
//! it is a way of getting at files, which is what that module is for.
//!
//! Two layers with a deliberate seam between them. `directory-files-recursive'
//! knows how to walk and how to *not* descend; everything about which files are
//! worth offering is Lisp. The tests follow that split.
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
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/completion.lisp"),
            include_str!("../../lisp/dired.lisp"),
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
        assert!(ctx.get_buffer("notes").is_none());
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
    // .gitignore, as far as it is understood
    // -----------------------------------------------------------------------

    #[test]
    fn a_directory_named_in_gitignore_is_not_descended_into() {
        let sandbox = Sandbox::new("gitignore-dir");
        sandbox
            .file(".gitignore", "build/\n")
            .file("keep.txt", "")
            .file("build/gone.txt", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(find-file-recursive--candidates "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        let found = paths(&answer);
        assert!(found.contains(&"keep.txt".to_string()));
        assert!(!found.iter().any(|p| p.starts_with("build/")));
    }

    #[test]
    fn a_bare_name_in_gitignore_is_treated_as_a_directory_too() {
        let sandbox = Sandbox::new("gitignore-bare");
        sandbox
            .file(".gitignore", "coverage\n")
            .file("keep.txt", "")
            .file("coverage/gone.txt", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(find-file-recursive--candidates "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        // `.gitignore` is itself offered: it is a file people edit, and
        // nothing about being read by this module makes it less so.
        assert_eq!(paths(&answer), vec![".gitignore", "keep.txt"]);
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
        let answer = run(
            &format!(r#"(find-file-recursive--candidates "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert_eq!(paths(&answer), vec![".gitignore", "keep.txt"]);
    }

    #[test]
    fn comments_and_blank_lines_are_not_patterns() {
        let sandbox = Sandbox::new("gitignore-comments");
        sandbox
            .file(".gitignore", "# a comment\n\n   \n*.log\n")
            .file("keep.txt", "")
            .file("noise.log", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(find-file-recursive--candidates "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        // Were `#` taken as a name, nothing would break visibly -- which is
        // exactly why this is pinned.
        assert_eq!(paths(&answer), vec![".gitignore", "keep.txt"]);
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
        let answer = run(
            &format!(r#"(find-file-recursive--candidates "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        // `build\r` names no directory at all, which is a silent no-op rather
        // than an error -- the search would simply keep offering the files.
        assert_eq!(paths(&answer), vec![".gitignore", "keep.txt"]);
    }

    #[test]
    fn a_pattern_that_is_not_understood_drops_nothing() {
        let sandbox = Sandbox::new("gitignore-unknown");
        sandbox
            .file(".gitignore", "!keep-me.log\nsrc/**/generated\n")
            .file("keep-me.log", "")
            .file("other.txt", "");
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(find-file-recursive--candidates "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        // Half-understanding a pattern and dropping files on the strength of
        // it is how a search comes to not find a file that is there. Anything
        // unrecognised is ignored, and the file stays.
        let found = paths(&answer);
        assert!(found.contains(&"keep-me.log".to_string()));
        assert!(found.contains(&"other.txt".to_string()));
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
        let answer = run(
            &format!(r#"(find-file-recursive--candidates "{}")"#, sandbox.path()),
            &env,
            &ctx,
        );
        assert_eq!(paths(&answer), vec!["keep.txt"]);
    }
}
