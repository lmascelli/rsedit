//! Minor modes: turned on and off in one buffer, their keys between the local
//! keymap and the major mode's, their hooks only where they are on, gone
//! when the major mode changes, brought back by the rules for the new one,
//! shown in the status line, and the command `define-minor-mode` makes.
//!
//! Design: `doc/architettura-editor.typ, <modi-minori>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent};
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

    /// An editor with the modes `one-mode`, shown as One, and `two-mode`,
    /// shown as nothing.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        run(
            r#"(make-minor-mode 'one-mode "One")
               (make-minor-mode 'two-mode)"#,
            &env,
            &ctx,
        );
        (ctx, env)
    }

    fn with_commands_module(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
        ] {
            eval_str(source, env, ctx).expect("loading the shipped lisp");
        }
    }

    /// Returns SRC's value as `format` writes it.
    fn shown(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match run(&format!("(format \"%s\" {src})"), env, ctx) {
            LispExp::String(text) => text.to_string(),
            other => panic!("format gave {other:?}"),
        }
    }

    /// What KEYS run, and the source `key-binding` says they come from.
    fn binding(keys: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> (String, String) {
        (
            shown(&format!("(nth 1 (key-binding {keys:?}))"), env, ctx),
            shown(&format!("(nth 2 (key-binding {keys:?}))"), env, ctx),
        )
    }

    /// The minor modes on in the current buffer.
    fn names(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Vec<String> {
        run("(minor-modes)", env, ctx)
            .iter()
            .map(|item| match item {
                LispExp::Symbol(name) => name.to_string(),
                other => panic!("not a symbol: {other:?}"),
            })
            .collect()
    }

    // ----------------------------------------------------------------
    // On and off
    // ----------------------------------------------------------------

    #[test]
    fn a_minor_mode_is_on_in_one_buffer_until_it_is_turned_off() {
        let (ctx, env) = editor();
        assert!(run("(minor-mode-p 'one-mode)", &env, &ctx).is_nil());
        assert!(run("(minor-mode-on 'one-mode)", &env, &ctx).is_truthy());
        assert!(
            run("(minor-mode-on 'one-mode)", &env, &ctx).is_nil(),
            "on already"
        );
        assert!(run("(minor-mode-p 'one-mode)", &env, &ctx).is_truthy());
        run(r#"(buffer-create "other")"#, &env, &ctx);
        assert!(run(r#"(minor-mode-p 'one-mode "other")"#, &env, &ctx).is_nil());
        assert!(run("(minor-mode-off 'one-mode)", &env, &ctx).is_truthy());
        assert!(
            run("(minor-mode-off 'one-mode)", &env, &ctx).is_nil(),
            "off already"
        );
        assert!(run("(minor-mode-p 'one-mode)", &env, &ctx).is_nil());
    }

    #[test]
    fn minor_modes_lists_them_in_the_order_they_came_on() {
        let (ctx, env) = editor();
        run(
            "(minor-mode-on 'two-mode) (minor-mode-on 'one-mode)",
            &env,
            &ctx,
        );
        assert_eq!(names(&env, &ctx), vec!["two-mode", "one-mode"]);
    }

    #[test]
    fn only_a_minor_mode_is_turned_on_and_only_a_major_one_is_a_major_mode() {
        let (ctx, env) = editor();
        assert!(eval_str("(minor-mode-on 'fundamental-mode)", &env, &ctx).is_err());
        assert!(eval_str("(minor-mode-on 'no-such-mode)", &env, &ctx).is_err());
        assert!(eval_str("(set-major-mode 'one-mode)", &env, &ctx).is_err());
    }

    // ----------------------------------------------------------------
    // Keys
    // ----------------------------------------------------------------

    #[test]
    fn a_minor_modes_keys_come_before_the_major_modes_and_the_global_ones() {
        let (ctx, env) = editor();
        run(
            r#"(define-key nil "C-c z" 'global-thing)
               (define-key 'fundamental-mode "C-c z" 'major-thing)
               (define-key 'one-mode "C-c z" 'minor-thing)"#,
            &env,
            &ctx,
        );
        assert_eq!(binding("C-c z", &env, &ctx).1, "mode");
        run("(minor-mode-on 'one-mode)", &env, &ctx);
        assert_eq!(
            binding("C-c z", &env, &ctx),
            ("minor-thing".into(), "minor".into())
        );
        run("(minor-mode-off 'one-mode)", &env, &ctx);
        assert_eq!(binding("C-c z", &env, &ctx).0, "major-thing");
    }

    #[test]
    fn the_minor_mode_turned_on_last_wins_and_the_local_keymap_wins_over_all() {
        let (ctx, env) = editor();
        run(
            r#"(define-key 'one-mode "C-c z" 'one-thing)
               (define-key 'two-mode "C-c z" 'two-thing)
               (minor-mode-on 'one-mode)
               (minor-mode-on 'two-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(binding("C-c z", &env, &ctx).0, "two-thing");
        run(
            r#"(make-mode 'my-local-map)
               (define-key 'my-local-map "C-c z" 'local-thing)
               (set-local-keymap 'my-local-map)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            binding("C-c z", &env, &ctx),
            ("local-thing".into(), "local".into())
        );
    }

    // ----------------------------------------------------------------
    // Hooks
    // ----------------------------------------------------------------

    #[test]
    fn a_minor_modes_hooks_run_only_where_it_is_on() {
        let (ctx, env) = editor();
        run(
            r#"(defvar typed 0)
               (defun count-typed () (setq typed (+ typed 1)))
               (add-hook 'one-mode "post-self-insert-hook" 'count-typed)"#,
            &env,
            &ctx,
        );
        ctx.handle_key_event(KeyEvent::new(KeyCode::Char('a')), &env);
        assert_eq!(run("typed", &env, &ctx), LispExp::number(0.0));
        run("(minor-mode-on 'one-mode)", &env, &ctx);
        ctx.handle_key_event(KeyEvent::new(KeyCode::Char('b')), &env);
        assert_eq!(run("typed", &env, &ctx), LispExp::number(1.0));
    }

    #[test]
    fn the_minor_mode_hook_runs_on_the_way_on_and_off_with_the_buffer_current() {
        let (ctx, env) = editor();
        run(
            r#"(defvar seen nil)
               (defun note () (setq seen (cons (list (current-buffer) (minor-mode-p 'one-mode)) seen)))
               (add-hook 'one-mode "minor-mode-hook" 'note)
               (buffer-create "other")
               (minor-mode-on 'one-mode "other")
               (minor-mode-off 'one-mode "other")"#,
            &env,
            &ctx,
        );
        assert_eq!(shown("seen", &env, &ctx), r#"(("other" nil) ("other" t))"#);
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
    }

    #[test]
    fn a_closed_buffer_runs_the_after_close_hook_of_its_minor_modes() {
        let (ctx, env) = editor();
        run(
            r#"(defvar closed nil)
               (defun note-closed () (setq closed t))
               (add-hook 'one-mode "after-close-hook" 'note-closed)
               (buffer-create "other")
               (minor-mode-on 'one-mode "other")
               (close-buffer "other")"#,
            &env,
            &ctx,
        );
        assert!(run("closed", &env, &ctx).is_truthy());
    }

    // ----------------------------------------------------------------
    // The major mode
    // ----------------------------------------------------------------

    #[test]
    fn a_new_major_mode_turns_the_minor_modes_off_and_its_rules_on() {
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'prose-mode)
               (minor-mode-enable 'two-mode 'prose-mode)
               (minor-mode-on 'one-mode)
               (set-major-mode 'prose-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(names(&env, &ctx), vec!["two-mode"]);
        run("(set-major-mode 'fundamental-mode)", &env, &ctx);
        assert!(run("(minor-modes)", &env, &ctx).is_nil());
    }

    #[test]
    fn a_rule_for_every_major_mode_reaches_every_new_buffer() {
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'prose-mode)
               (minor-mode-enable 'one-mode)
               (buffer-create "a" 'prose-mode)
               (buffer-create "b")"#,
            &env,
            &ctx,
        );
        assert!(run(r#"(minor-mode-p 'one-mode "a")"#, &env, &ctx).is_truthy());
        assert!(run(r#"(minor-mode-p 'one-mode "b")"#, &env, &ctx).is_truthy());
        assert!(
            run("(minor-mode-p 'one-mode)", &env, &ctx).is_nil(),
            "a buffer made before is left as it is"
        );
    }

    #[test]
    fn a_rule_taken_back_no_longer_reaches_a_new_buffer() {
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'prose-mode)
               (minor-mode-enable 'one-mode)
               (minor-mode-enable 'one-mode 'prose-mode)
               (minor-mode-enable 'two-mode 'prose-mode)
               (buffer-create "before")"#,
            &env,
            &ctx,
        );
        assert!(run("(minor-mode-disable 'one-mode)", &env, &ctx).is_truthy());
        assert!(
            run("(minor-mode-disable 'one-mode)", &env, &ctx).is_nil(),
            "nothing asks for it any more"
        );
        run(
            r#"(buffer-create "plain") (buffer-create "prose" 'prose-mode)"#,
            &env,
            &ctx,
        );
        assert!(run(r#"(minor-mode-p 'one-mode "plain")"#, &env, &ctx).is_nil());
        assert!(
            run(r#"(minor-mode-p 'one-mode "prose")"#, &env, &ctx).is_truthy(),
            "the rule for one major mode is another rule"
        );
        assert!(run(r#"(minor-mode-p 'two-mode "prose")"#, &env, &ctx).is_truthy());
        assert!(
            run(r#"(minor-mode-p 'one-mode "before")"#, &env, &ctx).is_truthy(),
            "a buffer where it is on is left as it is"
        );
    }

    #[test]
    fn minor_mode_disable_refuses_what_is_no_minor_mode() {
        let (ctx, env) = editor();
        assert!(eval_str("(minor-mode-disable 'fundamental-mode)", &env, &ctx).is_err());
    }

    // ----------------------------------------------------------------
    // A minor mode in every buffer
    // ----------------------------------------------------------------

    /// An editor with `shout-mode`, and `global-shout-mode`, which turns it on
    /// in every buffer.
    fn editor_with_a_global_mode() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor();
        with_commands_module(&env, &ctx);
        run(
            r#"(define-minor-mode shout-mode "Shout" "Shout in this buffer.")
               (define-globalized-minor-mode global-shout-mode shout-mode nil
                 "Shout in every buffer.")
               (make-mode 'prose-mode)
               (buffer-create "open")"#,
            &env,
            &ctx,
        );
        (ctx, env)
    }

    fn shouting(buffer: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        run(&format!("(minor-mode-p 'shout-mode {buffer:?})"), env, ctx).is_truthy()
    }

    #[test]
    fn a_global_mode_turns_its_mode_on_in_the_buffers_open_and_in_those_to_come() {
        let (ctx, env) = editor_with_a_global_mode();
        assert!(!shouting("open", &env, &ctx), "off until it is turned on");
        assert!(run("(global-shout-mode)", &env, &ctx).is_truthy());
        assert!(run("global-shout-mode", &env, &ctx).is_truthy());
        run(
            r#"(buffer-create "later") (buffer-create "prose" 'prose-mode)"#,
            &env,
            &ctx,
        );
        for buffer in ["*scratch*", "open", "later", "prose"] {
            assert!(shouting(buffer, &env, &ctx), "{buffer}");
        }
        assert!(run("(commandp 'global-shout-mode)", &env, &ctx).is_truthy());
    }

    #[test]
    fn a_global_mode_turned_off_turns_its_mode_off_in_every_buffer() {
        let (ctx, env) = editor_with_a_global_mode();
        run(r#"(minor-mode-on 'shout-mode "open")"#, &env, &ctx);
        run("(global-shout-mode 1) (global-shout-mode 0)", &env, &ctx);
        run(r#"(buffer-create "later")"#, &env, &ctx);
        for buffer in ["*scratch*", "open", "later"] {
            assert!(!shouting(buffer, &env, &ctx), "{buffer}");
        }
    }

    #[test]
    fn a_global_mode_reads_its_argument_as_a_minor_modes_command_does() {
        let (ctx, env) = editor_with_a_global_mode();
        for (arg, on) in [
            ("1", true),
            ("1", true),
            ("0", false),
            ("-1", false),
            ("'-", false),
        ] {
            run(&format!("(global-shout-mode {arg})"), &env, &ctx);
            assert_eq!(shouting("open", &env, &ctx), on, "after {arg}");
        }
    }

    #[test]
    fn a_buffer_that_turned_the_mode_off_has_it_back_with_its_next_major_mode() {
        let (ctx, env) = editor_with_a_global_mode();
        run(
            "(global-shout-mode 1) (minor-mode-off 'shout-mode)",
            &env,
            &ctx,
        );
        assert!(!shouting("*scratch*", &env, &ctx));
        run("(set-major-mode 'prose-mode)", &env, &ctx);
        assert!(shouting("*scratch*", &env, &ctx));
    }

    #[test]
    fn turn_on_chooses_the_buffers_a_global_mode_reaches() {
        let (ctx, env) = editor_with_a_global_mode();
        run(
            r#"(defun shout-in-prose ()
                 (if (eq (major-mode) 'prose-mode) (minor-mode-on 'shout-mode)))
               (define-globalized-minor-mode global-prose-shout-mode shout-mode
                 shout-in-prose "Shout in every prose buffer.")
               (buffer-create "prose" 'prose-mode)
               (global-prose-shout-mode 1)
               (buffer-create "more" 'prose-mode)
               (buffer-create "plain")"#,
            &env,
            &ctx,
        );
        assert!(shouting("prose", &env, &ctx));
        assert!(shouting("more", &env, &ctx));
        assert!(!shouting("open", &env, &ctx));
        assert!(!shouting("plain", &env, &ctx));
    }

    #[test]
    fn defining_a_global_mode_again_keeps_it_as_it_was() {
        let (ctx, env) = editor_with_a_global_mode();
        run(
            r#"(global-shout-mode 1)
               (define-globalized-minor-mode global-shout-mode shout-mode nil
                 "Shout in every buffer.")
               (buffer-create "later")"#,
            &env,
            &ctx,
        );
        assert!(run("global-shout-mode", &env, &ctx).is_truthy());
        assert!(shouting("later", &env, &ctx));
    }

    // ----------------------------------------------------------------
    // The status line
    // ----------------------------------------------------------------

    #[test]
    fn the_status_line_shows_the_lighters_after_the_major_mode() {
        let (ctx, env) = editor();
        run(
            "(minor-mode-on 'one-mode) (minor-mode-on 'two-mode)",
            &env,
            &ctx,
        );
        let mode_line = ctx.snapshot(&env, 80, 24).views[0]
            .mode_line
            .clone()
            .expect("a status line");
        assert!(
            mode_line.contains("fundamental-mode One "),
            "an empty lighter shows nothing: {mode_line:?}"
        );
    }

    // ----------------------------------------------------------------
    // define-minor-mode
    // ----------------------------------------------------------------

    #[test]
    fn define_minor_mode_makes_a_command_that_turns_it_on_and_off() {
        let (ctx, env) = editor();
        with_commands_module(&env, &ctx);
        run(
            r#"(defvar bodies 0)
               (define-minor-mode shout-mode "Shout"
                 "Shout what is typed."
                 (setq bodies (+ bodies 1)))"#,
            &env,
            &ctx,
        );
        assert!(run("(commandp 'shout-mode)", &env, &ctx).is_truthy());
        run("(shout-mode)", &env, &ctx);
        assert!(run("(minor-mode-p 'shout-mode)", &env, &ctx).is_truthy());
        assert_eq!(ctx.get_echo_message(), "shout-mode on");
        run("(shout-mode)", &env, &ctx);
        assert!(
            run("(minor-mode-p 'shout-mode)", &env, &ctx).is_nil(),
            "toggled"
        );
        run("(shout-mode 1) (shout-mode 1)", &env, &ctx);
        assert!(
            run("(minor-mode-p 'shout-mode)", &env, &ctx).is_truthy(),
            "1 is on"
        );
        run("(shout-mode -1)", &env, &ctx);
        assert!(
            run("(minor-mode-p 'shout-mode)", &env, &ctx).is_nil(),
            "-1 is off"
        );
        run("(shout-mode '(4))", &env, &ctx);
        assert!(
            run("(minor-mode-p 'shout-mode)", &env, &ctx).is_truthy(),
            "C-u is on"
        );
        assert_eq!(run("bodies", &env, &ctx), LispExp::number(6.0));
    }
}
