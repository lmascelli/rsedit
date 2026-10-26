//! Hooks that belong to every mode rather than to one, for what is not about
//! a kind of buffer, as the completion strip redrawing after a command: a
//! hook added to each mode would miss a mode made afterwards.
//!
//! Design: `doc/architettura-editor.typ, <modi>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use risp::{Env, EvalError, LispExp, Parser, eval};
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

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, c: char) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    #[test]
    fn a_hook_registered_with_nil_runs_in_the_current_mode() {
        let (ctx, env) = editor();
        run(
            r#"(setq *ran* 0)
               (defun count-it () (setq *ran* (+ *ran* 1)))
               (add-hook nil "post-command-hook" 'count-it)"#,
            &env,
            &ctx,
        );
        press(&ctx, &env, 'a');
        assert_eq!(run("*ran*", &env, &ctx), LispExp::number(1.0));
    }

    #[test]
    fn a_global_hook_runs_in_a_mode_defined_after_it_was_registered() {
        // The reason this exists at all. A hook registered in each mode
        // separately is correct until the next mode is defined, and then it is
        // quietly missing from that one.
        let (ctx, env) = editor();
        run(
            r#"(setq *ran* 0)
               (defun count-it () (setq *ran* (+ *ran* 1)))
               (add-hook nil "post-command-hook" 'count-it)
               (make-mode 'later-mode)
               (buffer-create "later" 'later-mode)
               (switch-to-buffer "later")"#,
            &env,
            &ctx,
        );
        press(&ctx, &env, 'a');
        assert_eq!(run("*ran*", &env, &ctx), LispExp::number(1.0));
    }

    #[test]
    fn a_modes_own_hook_runs_before_the_global_one() {
        // The specific statement about this buffer acts first; anything
        // general then reacts to the result.
        let (ctx, env) = editor();
        run(
            r#"(setq *order* nil)
               (defun mine () (setq *order* (append *order* (list "mode"))))
               (defun everyones () (setq *order* (append *order* (list "global"))))
               (add-hook 'fundamental-mode "post-command-hook" 'mine)
               (add-hook nil "post-command-hook" 'everyones)"#,
            &env,
            &ctx,
        );
        press(&ctx, &env, 'a');
        let order: Vec<String> = run("*order*", &env, &ctx)
            .iter()
            .map(|item| match item {
                LispExp::String(s) => s.to_string(),
                other => panic!("expected a string, got {other:?}"),
            })
            .collect();
        assert_eq!(order, vec!["mode", "global"]);
    }

    #[test]
    fn a_mode_hook_still_only_runs_in_its_own_mode() {
        let (ctx, env) = editor();
        run(
            r#"(setq *ran* 0)
               (defun count-it () (setq *ran* (+ *ran* 1)))
               (make-mode 'other-mode)
               (add-hook 'other-mode "post-command-hook" 'count-it)"#,
            &env,
            &ctx,
        );
        press(&ctx, &env, 'a');
        assert_eq!(run("*ran*", &env, &ctx), LispExp::number(0.0));
    }

    #[test]
    fn add_hook_for_an_unknown_mode_still_answers_nil() {
        // nil means "everywhere", so it must not be mistaken for the name of a
        // mode -- and a genuinely unknown mode must still be reported.
        let (ctx, env) = editor();
        assert!(
            run(
                r#"(add-hook 'no-such-mode "post-command-hook" 'ignore)"#,
                &env,
                &ctx
            )
            .is_nil()
        );
    }
    #[test]
    fn lifecycle_hooks_run_in_hidden_buffer_context_once_and_in_order() {
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'code-mode)
               (setq events nil)
               (defun created ()
                 (buffer-put 'created-in (current-buffer))
                 (setq events (append events '(created))))
               (defun local-mode () (setq events (append events '(local))))
               (defun any-mode () (setq events (append events '(global))))
               (defun ready () (setq events (append events '(ready))))
               (add-hook nil "buffer-created-hook" 'created)
               (add-hook 'code-mode "major-mode-hook" 'local-mode)
               (add-hook nil "major-mode-hook" 'any-mode)
               (add-hook nil "after-change-major-mode-hook" 'ready)
               (buffer-create "hidden" 'code-mode)
               (buffer-create "hidden" 'fundamental-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("events", &env, &ctx),
            run("'(created local global ready)", &env, &ctx)
        );
        assert_eq!(
            run("(buffer-get 'created-in \"hidden\")", &env, &ctx),
            LispExp::string("hidden".into())
        );
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        assert_eq!(ctx.focused_window_buffer().as_deref(), Some("*scratch*"));
        run(
            r#"(switch-to-buffer "hidden") (switch-to-buffer "*scratch*")"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(length events)", &env, &ctx), LispExp::number(4.0));
        run(
            r#"(with-current-buffer "hidden" (lambda ()
                 (set-major-mode 'fundamental-mode)
                 (set-major-mode 'fundamental-mode)))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("events", &env, &ctx),
            run("'(created local global ready global ready)", &env, &ctx)
        );
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
    }

    #[test]
    fn hook_registration_is_idempotent_and_removal_is_scoped() {
        let (ctx, env) = editor();
        run(
            r#"(setq ran 0)
               (defun count-hook () (setq ran (+ ran 1)))
               (add-hook nil "buffer-created-hook" 'count-hook)
               (add-hook nil "buffer-created-hook" 'count-hook)
               (add-hook 'fundamental-mode "buffer-created-hook" 'count-hook)
               (add-hook 'fundamental-mode "buffer-created-hook" 'count-hook)
               (buffer-create "first")"#,
            &env,
            &ctx,
        );
        assert_eq!(run("ran", &env, &ctx), LispExp::number(2.0));
        run(
            r#"(remove-hook 'fundamental-mode "buffer-created-hook" 'count-hook)
               (buffer-create "second")"#,
            &env,
            &ctx,
        );
        assert_eq!(run("ran", &env, &ctx), LispExp::number(3.0));
        run(
            r#"(remove-hook nil "buffer-created-hook" 'count-hook)
               (buffer-create "third")"#,
            &env,
            &ctx,
        );
        assert_eq!(run("ran", &env, &ctx), LispExp::number(3.0));
        assert!(
            run(
                r#"(remove-hook 'absent "buffer-created-hook" 'count-hook)"#,
                &env,
                &ctx
            )
            .is_nil()
        );
    }

    #[test]
    fn failed_creation_hook_does_not_block_later_hooks_or_strand_current_buffer() {
        let (ctx, env) = editor();
        run(
            r#"(defun broken-hook () (+ 1 "bad"))
               (defun working-hook () (buffer-put 'hook-ran t))
               (add-hook nil "buffer-created-hook" 'broken-hook)
               (add-hook nil "buffer-created-hook" 'working-hook)
               (buffer-create "hidden")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(buffer-get 'hook-ran \"hidden\")", &env, &ctx),
            LispExp::t()
        );
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("Hook buffer-created-hook"))
        );
    }

    #[test]
    fn file_hooks_see_text_path_and_major_mode_and_do_not_repeat_on_reopen() {
        let (ctx, env) = editor();
        let path =
            std::env::temp_dir().join(format!("rsedit-lifecycle-{}.hooktest", std::process::id()));
        std::fs::write(&path, "loaded text").unwrap();
        run(
            r#"(make-mode 'file-mode)
               (add-auto-mode "[.]hooktest$" 'file-mode)
               (setq visits 0)
               (defun file-hook ()
                 (setq visits (+ visits 1))
                 (buffer-put 'file-ready (list (major-mode) (buffer-file-name) (buffer-string))))
               (add-hook nil "find-file-hook" 'file-hook)"#,
            &env,
            &ctx,
        );
        run(
            &format!("(find-file {:?})", path.to_str().unwrap()),
            &env,
            &ctx,
        );
        assert_eq!(
            run("(car (buffer-get 'file-ready))", &env, &ctx),
            LispExp::symbol("file-mode".into())
        );
        assert_eq!(
            run("(nth 2 (buffer-get 'file-ready))", &env, &ctx),
            LispExp::string("loaded text".into())
        );
        assert_eq!(
            run("(nth 1 (buffer-get 'file-ready))", &env, &ctx),
            LispExp::string(path.to_str().unwrap().into())
        );
        run(
            &format!("(find-file {:?})", path.to_str().unwrap()),
            &env,
            &ctx,
        );
        assert_eq!(run("visits", &env, &ctx), LispExp::number(1.0));
        std::fs::remove_file(&path).unwrap();
        let new_path = path.with_extension("new.hooktest");
        run(
            &format!("(find-file {:?})", new_path.to_str().unwrap()),
            &env,
            &ctx,
        );
        assert_eq!(run("visits", &env, &ctx), LispExp::number(2.0));
        assert_eq!(
            run("(nth 2 (buffer-get 'file-ready))", &env, &ctx),
            LispExp::string("".into())
        );
    }

    #[test]
    fn write_file_runs_mode_hooks_after_rename_only_on_a_real_mode_change() {
        let (ctx, env) = editor();
        let path = std::env::temp_dir().join(format!(
            "rsedit-lifecycle-save-{}.hooktest",
            std::process::id()
        ));
        run(
            r#"(make-mode 'file-mode)
               (add-auto-mode "[.]hooktest$" 'file-mode)
               (setq mode-changes 0)
               (defun mode-hook ()
                 (setq mode-changes (+ mode-changes 1))
                 (buffer-put 'mode-name-seen (current-buffer)))
               (add-hook nil "after-change-major-mode-hook" 'mode-hook)"#,
            &env,
            &ctx,
        );
        run(
            &format!(
                "(write-file {:?}) (write-file {:?})",
                path.to_str().unwrap(),
                path.to_str().unwrap()
            ),
            &env,
            &ctx,
        );
        assert_eq!(run("mode-changes", &env, &ctx), LispExp::number(1.0));
        assert_eq!(
            run("(buffer-get 'mode-name-seen)", &env, &ctx),
            LispExp::string(path.file_name().unwrap().to_str().unwrap().into())
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn closing_the_last_buffer_initializes_its_replacement() {
        let (ctx, env) = editor();
        run(
            r#"(defun scratch-hook () (buffer-put 'initialized t))
               (add-hook nil "buffer-created-hook" 'scratch-hook)
               (close-buffer "*scratch*")"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(buffer-get 'initialized)", &env, &ctx), LispExp::t());
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
    }

    /// What `add-hook` refuses, it names: a lambda where the function's name
    /// goes is reported as the function, and a symbol where the hook's name
    /// goes as the hook.
    #[test]
    fn add_hook_names_the_argument_it_refuses() {
        let (ctx, env) = editor();
        for mode in ["nil", "'fundamental-mode"] {
            let refused = eval_str(
                &format!(r#"(add-hook {mode} "post-command-hook" (lambda () nil))"#),
                &env,
                &ctx,
            );
            match refused {
                Err(EvalError::WrongArgumentType { expected, got }) => {
                    assert!(expected.contains("function"), "{mode}: {expected}");
                    assert!(matches!(got, LispExp::Lambda(_)), "{mode}: got {got:?}");
                }
                other => panic!("{mode}: a lambda should be refused, got {other:?}"),
            }
        }
        match eval_str("(add-hook nil 'post-command-hook 'ignore)", &env, &ctx) {
            Err(EvalError::WrongArgumentType { expected, got }) => {
                assert!(expected.contains("hook"), "{expected}");
                assert_eq!(got, LispExp::symbol("post-command-hook".into()));
            }
            other => panic!("a hook named by a symbol should be refused, got {other:?}"),
        }
        match eval_str(r#"(remove-hook nil "post-command-hook" "f")"#, &env, &ctx) {
            Err(EvalError::WrongArgumentType { expected, .. }) => {
                assert!(expected.contains("function"), "{expected}");
            }
            other => panic!("remove-hook should say so too, got {other:?}"),
        }
    }
}
