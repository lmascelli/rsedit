//! The prompt `minibuffer-read` opens, confirmed, cancelled and completed in,
//! and `M-:`, which `minibuffer.lisp` defines over it. The module is read
//! from the source tree, since `eval-file` would look for it where the
//! installed binary is.
//!
//! Design: `doc/architettura-editor.typ, <minibuffer>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    fn eval_str(
        source: &str,
        env: &Arc<Env<EditorState<GapBuffer>>>,
        ctx: &EditorState<GapBuffer>,
    ) -> Result<LispExp<EditorState<GapBuffer>>, EvalError<EditorState<GapBuffer>>> {
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
        eval_str(include_str!("../../lisp/base/debug.lisp"), &env, &ctx)
            .unwrap_or_else(|e| panic!("loading debug.lisp failed: {e:?}"));
        eval_str(include_str!("../../lisp/base/minibuffer.lisp"), &env, &ctx)
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
        assert!(!ctx.has_buffer("*Minibuffer*"));
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
        assert!(!ctx.has_buffer("*Minibuffer*"));
        assert_eq!(
            eval_str("probe", &env, &ctx).unwrap(),
            LispExp::number(42.0)
        );
        // The result is shown in the echo area, not just logged where it's
        // easy to miss -- see the debug-system work this test accompanies.
        assert_eq!(ctx.get_echo_message(), "(setq probe 42) => 42");
    }

    /// Type SOURCE at M-: and press Return.
    fn evaluate_at_meta_colon(
        source: &str,
        env: &Arc<Env<EditorState<GapBuffer>>>,
        ctx: &EditorState<GapBuffer>,
    ) {
        eval_str("(eval-expression-prompt)", env, ctx).unwrap();
        for c in source.chars() {
            eval_str(&format!(r#"(self-insert "{c}")"#), env, ctx).unwrap();
        }
        eval_str("(minibuffer-confirm)", env, ctx).unwrap();
    }

    #[test]
    fn what_meta_colon_defines_outlives_the_expression() {
        // At the top level, not inside `eval-expression-confirm`, where a
        // defun would be that function's own and go when it returned.
        let (ctx, env) = setup();
        eval_str("(defun probe-old () 1)", &env, &ctx).unwrap();
        evaluate_at_meta_colon("(defun probe-new () 7)", &env, &ctx);
        evaluate_at_meta_colon("(defvar probe-var 5)", &env, &ctx);
        evaluate_at_meta_colon("(setq probe-fresh 6)", &env, &ctx);
        evaluate_at_meta_colon("(defun probe-old () 2)", &env, &ctx);
        assert_eq!(
            eval_str("(probe-new)", &env, &ctx).unwrap(),
            LispExp::number(7.0)
        );
        assert_eq!(
            eval_str("probe-var", &env, &ctx).unwrap(),
            LispExp::number(5.0)
        );
        assert_eq!(
            eval_str("probe-fresh", &env, &ctx).unwrap(),
            LispExp::number(6.0)
        );
        assert_eq!(
            eval_str("(probe-old)", &env, &ctx).unwrap(),
            LispExp::number(2.0)
        );
    }

    #[test]
    fn a_command_defined_at_meta_colon_runs_from_m_x() {
        // Defined as well as registered, so that M-x runs it rather than
        // failing with UndefinedFunction.
        let (ctx, env) = setup();
        eval_str(include_str!("../../lisp/base/commands.lisp"), &env, &ctx)
            .unwrap_or_else(|e| panic!("loading commands.lisp failed: {e:?}"));
        eval_str("(setq probe-ran nil)", &env, &ctx).unwrap();
        evaluate_at_meta_colon(
            "(defcommand probe-cmd () nil (setq probe-ran t))",
            &env,
            &ctx,
        );
        eval_str(r#"(execute-extended-command "probe-cmd")"#, &env, &ctx).unwrap();
        assert!(eval_str("probe-ran", &env, &ctx).unwrap().is_truthy());
    }

    #[test]
    fn the_command_m_colon_runs_is_one_m_x_reaches_too() {
        let (ctx, env) = setup();
        assert!(
            eval_str("(commandp 'eval-expression-prompt)", &env, &ctx)
                .unwrap()
                .is_truthy()
        );
        eval_str(
            r#"(execute-extended-command "eval-expression-prompt")"#,
            &env,
            &ctx,
        )
        .unwrap();
        assert_eq!(ctx.get_current_buffer_name(), "*Minibuffer*");
    }

    #[test]
    fn eval_expression_of_an_invalid_form_does_not_abort_the_confirm_flow() {
        // An expression that does not evaluate, as `(1 2 3)`, whose head is
        // no function, is reported with what was typed: the failure is caught
        // by `eval-string-top-level`, and does not end `minibuffer-confirm`.
        let (ctx, env) = setup();

        eval_str("(eval-expression-prompt)", &env, &ctx).unwrap();
        for c in "(1 2 3)".chars() {
            eval_str(&format!(r#"(self-insert "{c}")"#), &env, &ctx).unwrap();
        }

        // Must not error.
        eval_str("(minibuffer-confirm)", &env, &ctx).unwrap();

        // The minibuffer still closes and hands focus back cleanly, exactly
        // as it does for a valid expression, and the echo area explains
        // what happened instead of staying silent.
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        assert!(!ctx.has_buffer("*Minibuffer*"));
        assert_eq!(ctx.get_echo_message(), "(1 2 3) !! UnvalidFunctionCall");
    }
}
