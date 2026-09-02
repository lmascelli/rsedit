//! End-to-end tests for `core/lisp/debug.lisp` and the `backtrace`/
//! `all-logs` primitives it builds on.
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
    ) -> Result<LispExp<EditorState<GapBuffer>>, EvalError<EditorState<GapBuffer>>> {
        let wrapped = format!("(progn {})", source);
        let mut parser = Parser::new(&wrapped);
        let ast = parser.next().expect("failed to parse test script");
        eval(&ast, env.clone(), ctx)
    }

    /// A fresh editor with `debug.lisp` loaded and `frame-width`/
    /// `frame-height` set (needed by `backtrace-show`'s window sizing,
    /// and otherwise only set by a real resize event from a UI frontend).
    fn setup() -> (EditorState<GapBuffer>, Arc<Env<EditorState<GapBuffer>>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("create_global_env failed");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        eval_str(include_str!("../../lisp/debug.lisp"), &env, &ctx)
            .unwrap_or_else(|e| panic!("loading debug.lisp failed: {e:?}"));
        (ctx, env)
    }

    /// A key bound (globally) to a call to an undefined function, so
    /// dispatching it through `handle_key_event` always produces a real,
    /// uncaught `UndefinedFunction` error -- exercising the same path a
    /// mistyped keybinding or a buggy command would.
    fn bind_a_failing_key(
        env: &Arc<Env<EditorState<GapBuffer>>>,
        ctx: &EditorState<GapBuffer>,
    ) -> KeyEvent {
        eval_str(
            r#"(define-key nil "C-t" '(this-function-does-not-exist))"#,
            env,
            ctx,
        )
        .unwrap();
        KeyEvent {
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        }
    }

    #[test]
    fn message_shows_in_the_echo_area_and_stays_in_the_log() {
        let (ctx, env) = setup();
        eval_str(r#"(message "hello %s, you are %d" "world" 3)"#, &env, &ctx).unwrap();

        assert_eq!(ctx.get_echo_message(), "hello world, you are 3");
        assert!(
            ctx.get_logs()
                .contains(&"hello world, you are 3".to_string())
        );
    }

    #[test]
    fn message_returns_the_formatted_string() {
        let (ctx, env) = setup();
        assert_eq!(
            eval_str(r#"(message "%s+%s" "a" "b")"#, &env, &ctx).unwrap(),
            LispExp::string("a+b".into())
        );
    }

    #[test]
    fn switch_to_messages_creates_and_populates_a_buffer_from_the_log() {
        let (ctx, env) = setup();
        eval_str(r#"(log "first entry") (log "second entry")"#, &env, &ctx).unwrap();

        eval_str("(switch-to-messages)", &env, &ctx).unwrap();
        assert_eq!(ctx.get_current_buffer_name(), "*Messages*");

        let content = match eval_str("(buffer-string)", &env, &ctx).unwrap() {
            LispExp::String(s) => (*s).clone(),
            other => panic!("buffer-string didn't return a string: {other:?}"),
        };
        assert!(content.contains("first entry"));
        assert!(content.contains("second entry"));
    }

    #[test]
    fn switch_to_messages_picks_up_anything_logged_since_the_last_call() {
        let (ctx, env) = setup();
        eval_str(r#"(log "one")"#, &env, &ctx).unwrap();
        eval_str("(switch-to-messages)", &env, &ctx).unwrap();
        eval_str(r#"(log "two")"#, &env, &ctx).unwrap();
        eval_str("(switch-to-messages)", &env, &ctx).unwrap();

        let content = match eval_str("(buffer-string)", &env, &ctx).unwrap() {
            LispExp::String(s) => (*s).clone(),
            other => panic!("buffer-string didn't return a string: {other:?}"),
        };
        assert!(content.contains("one"));
        assert!(content.contains("two"));
    }

    #[test]
    fn backtrace_primitive_reports_the_last_uncaught_error_without_including_itself() {
        let (ctx, env) = setup();

        let result = eval_str(r#"(eval-string "(1 2 3)")"#, &env, &ctx);
        assert!(result.is_err());

        assert_eq!(
            eval_str("(backtrace)", &env, &ctx).unwrap(),
            LispExp::proper_list(vec![LispExp::string("eval-string".into())])
        );
    }

    #[test]
    fn a_key_triggered_error_is_echoed_via_report_error() {
        let (ctx, env) = setup();
        let key = bind_a_failing_key(&env, &ctx);

        ctx.handle_key_event(key, &env);

        let echo = ctx.get_echo_message();
        assert!(echo.starts_with("Eval Error:"), "got {echo:?}");
        assert!(echo.contains("UndefinedFunction"), "got {echo:?}");
    }

    #[test]
    fn debug_on_error_off_does_not_open_a_backtrace_window() {
        let (ctx, env) = setup();
        let key = bind_a_failing_key(&env, &ctx);

        ctx.handle_key_event(key, &env);

        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        assert!(ctx.get_buffer("*Backtrace*").is_none());
    }

    #[test]
    fn debug_on_error_on_opens_a_backtrace_window_with_the_error_and_stack() {
        let (ctx, env) = setup();
        eval_str("(setq debug-on-error t)", &env, &ctx).unwrap();
        let key = bind_a_failing_key(&env, &ctx);

        ctx.handle_key_event(key, &env);

        assert_eq!(ctx.get_current_buffer_name(), "*Backtrace*");
        let content = match eval_str("(buffer-string)", &env, &ctx).unwrap() {
            LispExp::String(s) => (*s).clone(),
            other => panic!("buffer-string didn't return a string: {other:?}"),
        };
        assert!(content.contains("UndefinedFunction"));

        // Dismissing hands focus back to where it was before the popup.
        eval_str("(backtrace-dismiss)", &env, &ctx).unwrap();
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        assert!(
            ctx.floating_windows
                .read()
                .expect("read floating_windows")
                .is_empty()
        );
    }

    #[test]
    fn a_second_error_while_debug_on_error_is_on_replaces_rather_than_stacks_the_window() {
        let (ctx, env) = setup();
        eval_str("(setq debug-on-error t)", &env, &ctx).unwrap();
        let key = bind_a_failing_key(&env, &ctx);

        ctx.handle_key_event(key.clone(), &env);
        ctx.handle_key_event(key, &env);

        assert_eq!(
            ctx.floating_windows
                .read()
                .expect("read floating_windows")
                .len(),
            1,
            "a second error should replace the *Backtrace* window, not stack another one"
        );
    }
}
