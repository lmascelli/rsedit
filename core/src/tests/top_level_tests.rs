//! What is evaluated at the top level, outside any function: a file loaded
//! with `eval-file`, and an expression given to `eval-string-top-level`.
//!
//! A `defun`, a `defvar` or the `setq` of a new name binds in the environment
//! it is evaluated in. Evaluated inside a function it is that function's own,
//! and goes when the function returns -- which is what a file loaded from a
//! command, or an expression typed at `M-:`, must not do.
//!
//! Design: `doc/architettura-editor.typ, <valutazione>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, LispExp, Parser, eval};
    use std::path::PathBuf;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test's Lisp parses");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("{src}: {why:?}"))
    }

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "rsedit-top-level-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("sandbox");
            Sandbox(root)
        }
        fn file(&self, name: &str, contents: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, contents).expect("sandbox file");
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

    const MODULE: &str = "(defun probe-loaded () 11)\n(defvar probe-loaded-var 12)\n";

    #[test]
    fn a_file_loaded_from_inside_a_function_defines_what_init_lisp_would() {
        let sandbox = Sandbox::new("inside");
        let file = sandbox.file("probe.lisp", MODULE);
        let (ctx, env) = create_global_env::<GapBuffer>().expect("env");
        run(
            &format!("(defun probe-load () (eval-file {file:?})) (probe-load)"),
            &env,
            &ctx,
        );
        assert_eq!(run("(probe-loaded)", &env, &ctx), LispExp::number(11.0));
        assert_eq!(run("probe-loaded-var", &env, &ctx), LispExp::number(12.0));
    }

    #[test]
    fn a_file_is_found_through_the_lisp_path_of_the_call() {
        // Only the evaluation moves to the top level: a `lisp-path` bound
        // around the call is the one the file is looked for in.
        let sandbox = Sandbox::new("path");
        sandbox.file("probe.lisp", MODULE);
        let (ctx, env) = create_global_env::<GapBuffer>().expect("env");
        run(
            &format!(
                "(let ((lisp-path (list {:?}))) (eval-file \"probe\"))",
                sandbox.path()
            ),
            &env,
            &ctx,
        );
        assert_eq!(run("(probe-loaded)", &env, &ctx), LispExp::number(11.0));
    }

    #[test]
    fn an_expression_evaluated_at_the_top_level_defines_for_the_whole_editor() {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("env");
        let answer = run(
            r#"(defun probe-eval ()
                 (eval-string-top-level "(progn (defun probe-twice (n) (* 2 n)) (setq probe-fresh 3))"))
               (probe-eval)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            answer,
            LispExp::proper_list(vec![LispExp::t(), LispExp::number(3.0)])
        );
        assert_eq!(run("(probe-twice 4)", &env, &ctx), LispExp::number(8.0));
        assert_eq!(run("probe-fresh", &env, &ctx), LispExp::number(3.0));
    }

    #[test]
    fn a_failure_at_the_top_level_is_returned_and_leaves_no_frames() {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("env");
        assert_eq!(
            run(r#"(eval-string-top-level "(1 2 3)")"#, &env, &ctx),
            LispExp::proper_list(vec![
                LispExp::nil(),
                LispExp::string("UnvalidFunctionCall".into())
            ])
        );
        let unparsed = run(r#"(eval-string-top-level "(car")"#, &env, &ctx);
        assert!(unparsed.iter().next().is_some_and(|first| first.is_nil()));
        // The frames of the call that failed went with it, so the next
        // error's backtrace is its own.
        assert!(
            run(
                r#"(eval-string-top-level "(car 5)") (backtrace)"#,
                &env,
                &ctx
            )
            .is_nil()
        );
    }
}
