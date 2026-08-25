//! End-to-end tests for `core/lisp/minibuffer.lisp`, run against a real
//! `EditorState` + `install_primitives` stack (not just `setup_base_env`)
//! since the minibuffer relies on `make-floating-window`, `close-buffer`,
//! `switch-to-buffer`, and the `after-close-hook`/major-mode machinery,
//! none of which exist in the bare Lisp-core test environment.
//!
//! `minibuffer.lisp` is loaded here by evaluating its contents directly
//! (`include_str!`) rather than through `eval-file`, since `eval-file`
//! resolves relative to `lisp-path`, which points at the *installed*
//! binary's directory and won't find the source tree under `cargo test`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    fn eval_str(
        source: &str,
        env: &Arc<Env<EditorState<GapBuffer>>>,
        ctx: &EditorState<GapBuffer>,
    ) -> Result<LispExp<EditorState<GapBuffer>>, EvalError> {
        let wrapped = format!("(progn {})", source);
        let mut parser = Parser::new(&wrapped);
        let ast = parser.next().expect("failed to parse test script");
        eval(&ast, env.clone(), ctx)
    }

    /// A fresh editor with `debug.lisp` and `minibuffer.lisp` loaded (in
    /// that order, matching the default init.lisp -- `minibuffer.lisp`'s
    /// `eval-expression-confirm` calls `message`, from `debug.lisp`) and
    /// `frame-width`/`frame-height` set (both needed by
    /// `default-minibuffer-prompt`, and otherwise only set by the real
    /// resize event from a UI frontend).
    fn setup() -> (EditorState<GapBuffer>, Arc<Env<EditorState<GapBuffer>>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("create_global_env failed");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        eval_str(include_str!("../../lisp/debug.lisp"), &env, &ctx)
            .unwrap_or_else(|e| panic!("loading debug.lisp failed: {e:?}"));
        eval_str(include_str!("../../lisp/minibuffer.lisp"), &env, &ctx)
            .unwrap_or_else(|e| panic!("loading minibuffer.lisp failed: {e:?}"));
        (ctx, env)
    }

    #[test]
    fn confirm_flow_captures_input_and_restores_the_previous_buffer() {
        let (ctx, env) = setup();

        eval_str(
            r#"
            (setq *test-result* nil)
            (minibuffer-read "Test"
                (lambda (input) (setq *test-result* input))
                nil nil)
            "#,
            &env,
            &ctx,
        )
        .unwrap();
        assert_eq!(ctx.get_current_buffer_name(), "*Minibuffer*");

        eval_str(r#"(self-insert "h") (self-insert "i")"#, &env, &ctx).unwrap();
        eval_str("(minibuffer-confirm)", &env, &ctx).unwrap();

        assert_eq!(
            eval_str("*test-result*", &env, &ctx).unwrap(),
            LispExp::string("hi".into())
        );
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        assert!(ctx.get_buffer("*Minibuffer*").is_none());
    }

    #[test]
    fn cancel_flow_calls_on_cancel_without_touching_on_confirm() {
        let (ctx, env) = setup();

        eval_str(
            r#"
            (setq *confirmed* nil)
            (setq *cancelled* nil)
            (minibuffer-read "Test"
                (lambda (input) (setq *confirmed* t))
                nil
                (lambda () (setq *cancelled* t)))
            "#,
            &env,
            &ctx,
        )
        .unwrap();
        eval_str("(minibuffer-cancel)", &env, &ctx).unwrap();

        assert_eq!(eval_str("*cancelled*", &env, &ctx).unwrap(), LispExp::t());
        assert_eq!(eval_str("*confirmed*", &env, &ctx).unwrap(), LispExp::nil());
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
    }

    #[test]
    fn tab_cycles_through_completion_candidates_and_wraps_around() {
        let (ctx, env) = setup();

        eval_str(
            r#"(minibuffer-read "Test" nil (lambda (input) (list "alpha" "beta" "gamma")) nil)"#,
            &env,
            &ctx,
        )
        .unwrap();

        for expected in ["alpha", "beta", "gamma", "alpha"] {
            eval_str("(minibuffer-complete)", &env, &ctx).unwrap();
            assert_eq!(
                eval_str("(buffer-string)", &env, &ctx).unwrap(),
                LispExp::string(expected.into())
            );
        }
    }

    #[test]
    fn typing_after_a_completion_starts_a_fresh_completion_request() {
        let (ctx, env) = setup();

        eval_str(
            r#"(minibuffer-read "Test" nil (lambda (input) (list "alpha" "beta")) nil)"#,
            &env,
            &ctx,
        )
        .unwrap();

        eval_str("(minibuffer-complete)", &env, &ctx).unwrap(); // -> "alpha"
        // Typing invalidates the mid-cycle state (buffer no longer equals
        // the last-shown candidate), so the next Tab must ask for fresh
        // candidates from scratch rather than advancing the old cycle.
        eval_str(r#"(self-insert "!")"#, &env, &ctx).unwrap(); // -> "alpha!"
        eval_str("(minibuffer-complete)", &env, &ctx).unwrap();
        assert_eq!(
            eval_str("(buffer-string)", &env, &ctx).unwrap(),
            LispExp::string("alpha".into())
        );
    }

    #[test]
    fn closing_does_not_leak_completion_state_into_the_next_prompt() {
        let (ctx, env) = setup();

        eval_str(
            r#"(minibuffer-read "First" nil (lambda (input) (list "x" "y")) nil)"#,
            &env,
            &ctx,
        )
        .unwrap();
        eval_str("(minibuffer-complete)", &env, &ctx).unwrap();
        eval_str("(minibuffer-cancel)", &env, &ctx).unwrap();

        // Reopen with no on-change callback this time; Tab must be a
        // harmless no-op rather than acting on the previous prompt's
        // stale candidate list.
        eval_str(r#"(minibuffer-read "Second" nil nil nil)"#, &env, &ctx).unwrap();
        eval_str("(minibuffer-complete)", &env, &ctx).unwrap();
        assert_eq!(
            eval_str("(buffer-string)", &env, &ctx).unwrap(),
            LispExp::string("".into())
        );
    }

    #[test]
    fn meta_colon_opens_an_eval_minibuffer_that_evaluates_its_input() {
        let (ctx, env) = setup();

        eval_str("(setq probe nil)", &env, &ctx).unwrap();

        // Drive the actual M-: key binding rather than calling
        // `eval-expression-prompt` directly, so the test also proves the
        // global keymap entry is wired up.
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(':'),
                modifiers: KeyModifiers {
                    alt: true,
                    ..Default::default()
                },
            },
            &env,
        );
        assert_eq!(ctx.get_current_buffer_name(), "*Minibuffer*");

        for c in "(setq probe 42)".chars() {
            eval_str(&format!(r#"(self-insert "{c}")"#), &env, &ctx).unwrap();
        }
        eval_str("(minibuffer-confirm)", &env, &ctx).unwrap();

        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        assert!(ctx.get_buffer("*Minibuffer*").is_none());
        assert_eq!(
            eval_str("probe", &env, &ctx).unwrap(),
            LispExp::number(42.0)
        );
        // The result is shown in the echo area, not just logged where it's
        // easy to miss -- see the debug-system work this test accompanies.
        assert_eq!(ctx.get_echo_message(), "(setq probe 42) => 42");
    }

    #[test]
    fn eval_expression_of_an_invalid_form_does_not_abort_the_confirm_flow() {
        // Regression test: typing something that isn't a valid function
        // call (e.g. `(1 2 3)`, whose head `1` isn't callable) used to make
        // `eval-string`'s error propagate all the way out of
        // `minibuffer-confirm`, so `(minibuffer-confirm)` itself returned
        // an Err logged as an opaque "Eval Error: (minibuffer-confirm)
        // UnvalidFunctionCall" -- no mention of what was actually typed.
        // `eval-expression-confirm` now goes through `eval-string-safe`,
        // so a bad expression is reported instead of blowing up the whole
        // confirm flow.
        let (ctx, env) = setup();

        eval_str("(eval-expression-prompt)", &env, &ctx).unwrap();
        for c in "(1 2 3)".chars() {
            eval_str(&format!(r#"(self-insert "{c}")"#), &env, &ctx).unwrap();
        }

        // Must not error -- the whole point of the fix.
        eval_str("(minibuffer-confirm)", &env, &ctx).unwrap();

        // The minibuffer still closes and hands focus back cleanly, exactly
        // as it does for a valid expression, and the echo area explains
        // what happened instead of staying silent.
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        assert!(ctx.get_buffer("*Minibuffer*").is_none());
        assert_eq!(ctx.get_echo_message(), "(1 2 3) !! UnvalidFunctionCall");
    }
}
