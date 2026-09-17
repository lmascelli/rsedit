//! Hooks that belong to every mode rather than to one.
//!
//! A mode's own hooks are right for anything that is about *this kind of
//! buffer*. Some things are not: the completion strip has to redraw after a
//! command that changed what was typed, and what the buffer's mode happens to
//! be has nothing to do with it. Registering such a hook in each mode
//! separately works until somebody defines a mode afterwards.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
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
}
