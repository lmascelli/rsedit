//! Tests for the `LispContext::push_call_frame`/`pop_call_frame` protocol
//! (see the trait docs on `push_call_frame` in `lisp.rs` for the exact
//! contract): a frame pops when its call succeeds, but stays in place when
//! it fails, so the frames still standing once an error has finished
//! propagating are exactly the chain of calls active at the moment things
//! went wrong.
//!
//! Uses a minimal standalone `LispContext` rather than `EditorState`, to
//! test the core protocol in isolation from the editor.
#[cfg(test)]
mod tests {
    use crate::lisp::{Env, EvalError, LispContext, LispExp, Parser, eval, setup_base_env};
    use std::sync::RwLock;

    #[derive(Debug)]
    struct BacktraceCtx {
        call_stack: RwLock<Vec<String>>,
    }

    impl Clone for BacktraceCtx {
        fn clone(&self) -> Self {
            unreachable!()
        }
    }

    impl PartialEq for BacktraceCtx {
        fn eq(&self, _other: &Self) -> bool {
            unreachable!()
        }
    }

    impl LispContext for BacktraceCtx {
        fn consume_fuel(&self, _amount: u32) -> Result<(), EvalError<BacktraceCtx>> {
            Ok(())
        }
        fn log_diagnostic(&self, _msg: &str) {}
        fn push_call_frame(&self, frame: &str) {
            self.call_stack.write().unwrap().push(frame.to_string());
        }
        fn pop_call_frame(&self) {
            self.call_stack.write().unwrap().pop();
        }
        fn call_frame_depth(&self) -> usize {
            self.call_stack.read().unwrap().len()
        }
        fn truncate_call_frames(&self, depth: usize) {
            self.call_stack.write().unwrap().truncate(depth);
        }
    }

    impl BacktraceCtx {
        fn new() -> Self {
            Self {
                call_stack: RwLock::new(Vec::new()),
            }
        }

        /// Innermost (deepest) call first, matching `EditorState::backtrace`.
        fn backtrace(&self) -> Vec<String> {
            let mut frames = self.call_stack.read().unwrap().clone();
            frames.reverse();
            frames
        }
    }

    fn eval_script(
        ctx: &BacktraceCtx,
        script: &str,
    ) -> Result<LispExp<BacktraceCtx>, EvalError<BacktraceCtx>> {
        let env = Env::new_root();
        setup_base_env(env.clone());
        let wrapped = format!("(progn {})", script);
        let mut parser = Parser::new(&wrapped);
        let ast = parser.next().expect("failed to parse test script");
        eval(&ast, env, ctx)
    }

    #[test]
    fn a_successful_call_leaves_the_stack_empty() {
        let ctx = BacktraceCtx::new();
        eval_script(&ctx, "(defun f (x) (+ x 1)) (f 41)").unwrap();
        assert_eq!(ctx.backtrace(), Vec::<String>::new());
    }

    #[test]
    fn a_non_tail_call_stays_on_a_frozen_stack() {
        // `f`'s body is `(g) 1` -- two forms, so `(g)` is *not* the last
        // one and is evaluated by a genuine nested (non-tail) call. `g`'s
        // own body is just `(h)`, in tail position, so `g`'s frame is
        // popped again before `h` -- itself also a single tail-called
        // form -- ever runs; likewise for `h` calling the (unbound)
        // `undefined-fn`. Only `f`, which never got to pop because the
        // failure happened while it was still waiting on `(g)`, survives.
        let ctx = BacktraceCtx::new();
        let result = eval_script(
            &ctx,
            "(defun h () (undefined-fn))
             (defun g () (h))
             (defun f () (g) 1)
             (f)",
        );
        assert_eq!(
            result.unwrap_err(),
            EvalError::UndefinedFunction("undefined-fn".into())
        );
        assert_eq!(ctx.backtrace(), vec!["f"]);
    }

    #[test]
    fn a_primitive_that_evaluates_its_own_nested_expression_stays_on_the_stack() {
        // This is the shape that motivated the feature: `eval-string`
        // parses and evaluates its argument *inside* its own primitive
        // call, so if that nested evaluation fails, `eval-string` itself
        // is still genuinely "on the stack" (in Rust-call terms) when it
        // does -- unlike a plain Lisp function in tail position, which
        // would already have been popped.
        let ctx = BacktraceCtx::new();
        let result = eval_script(&ctx, r#"(eval-string "(1 2 3)")"#);
        assert_eq!(result.unwrap_err(), EvalError::UnvalidFunctionCall);
        assert_eq!(ctx.backtrace(), vec!["eval-string"]);
    }

    #[test]
    fn funcall_stays_on_the_stack_across_the_function_it_invokes() {
        // `funcall` also does its dispatch via a nested `eval` inside its
        // own primitive body, so it stays frozen on the stack across
        // whatever it calls -- even though the callee itself (`ee`, whose
        // whole body is one tail-position form) does not, matching
        // `a_non_tail_call_stays_on_a_frozen_stack` above.
        let ctx = BacktraceCtx::new();
        let result = eval_script(
            &ctx,
            r#"(defun ee (input) (eval-string input))
               (funcall 'ee "(1 2 3)")"#,
        );
        assert_eq!(result.unwrap_err(), EvalError::UnvalidFunctionCall);
        assert_eq!(ctx.backtrace(), vec!["eval-string", "funcall"]);
    }

    #[test]
    fn eval_string_safe_does_not_leak_frames_from_the_error_it_swallows() {
        // Regression test: `eval-string-safe` catches the error from its
        // own nested `eval` and returns a value instead of propagating
        // it -- but that nested evaluation may have pushed frames (e.g.
        // `eval-string` itself, called by the very expression under
        // test) that never got to pop, because *their* call failed. If
        // `eval-string-safe` didn't clean those up, they'd sit on the
        // stack forever, corrupting the backtrace of some later, wholly
        // unrelated, genuinely uncaught error.
        let ctx = BacktraceCtx::new();
        eval_script(&ctx, r#"(eval-string-safe "(eval-string \"(1 2 3)\")")"#).unwrap();
        assert_eq!(LispContext::call_frame_depth(&ctx), 0);
        assert_eq!(ctx.backtrace(), Vec::<String>::new());

        // A subsequent, genuinely uncaught error still gets an accurate
        // backtrace, not one with the swallowed failure's frames still
        // sitting underneath it.
        let result = eval_script(&ctx, r#"(defun f () (eval-string "(1 2 3)")) (f)"#);
        assert_eq!(result.unwrap_err(), EvalError::UnvalidFunctionCall);
        assert_eq!(ctx.backtrace(), vec!["eval-string"]);
    }
}
