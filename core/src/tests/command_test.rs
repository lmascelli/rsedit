//! Commands: the registry, `call-interactively`, and M-x.
//!
//! These drive the real flow -- key events into `handle_key_event`, input typed
//! into the minibuffer, Return to confirm -- rather than calling the primitives
//! directly, because the interesting behaviour is precisely what happens
//! *across* keystrokes. `minibuffer-read` returns immediately and its callback
//! fires later, so a command that takes arguments only finishes running several
//! events after the key that started it.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(
        source: &str,
        env: &Arc<Env<Ctx>>,
        ctx: &Ctx,
    ) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {source})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    /// A booted editor with no `.lisp` file loaded at all.
    ///
    /// Everything the command system needs is in Rust, so this is the harness
    /// most tests here use -- if a command test needs a `.lisp` file to pass,
    /// that is the bug.
    fn bare() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("create_global_env failed");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    /// A booted editor with the standard Lisp layers loaded, in the order
    /// `init.lisp` uses them. Only needed by tests that use `defcommand`,
    /// which is sugar defined in `commands.lisp`.
    fn setup() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("create_global_env failed");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for (name, source) in [
            ("debug.lisp", include_str!("../../lisp/debug.lisp")),
            (
                "minibuffer.lisp",
                include_str!("../../lisp/minibuffer.lisp"),
            ),
            ("commands.lisp", include_str!("../../lisp/commands.lisp")),
        ] {
            eval_str(source, &env, &ctx).unwrap_or_else(|e| panic!("loading {name} failed: {e:?}"));
        }
        (ctx, env)
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::default(),
        }
    }

    fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::default(),
        }
    }

    fn meta(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers {
                alt: true,
                ..Default::default()
            },
        }
    }

    /// Type SOURCE into the open minibuffer and press Return.
    fn type_and_confirm(ctx: &Ctx, env: &Arc<Env<Ctx>>, text: &str) {
        for c in text.chars() {
            ctx.handle_key_event(key(c), env);
        }
        ctx.handle_key_event(plain(KeyCode::Enter), env);
    }

    fn current_buffer(ctx: &Ctx) -> String {
        ctx.get_current_buffer_name()
    }

    // ---------------------------------------------------------------
    // The registry
    // ---------------------------------------------------------------

    /// Built-in commands are registered from Rust, next to the primitive they
    /// name, so they cannot be missed by a `.lisp` file failing to load.
    #[test]
    fn built_in_commands_are_registered_from_rust() {
        let (ctx, env) = setup();
        for name in ["find-file", "save-buffer", "next-line", "quit"] {
            assert_eq!(
                eval_str(&format!("(commandp '{name})"), &env, &ctx).expect("commandp"),
                LispExp::t(),
                "{name} should be a command"
            );
        }
        // An ordinary function is not a command just by existing.
        assert!(
            eval_str("(commandp 'car)", &env, &ctx)
                .expect("commandp")
                .is_nil()
        );
    }

    /// `find-file` takes a path, so it declares one file argument. This is the
    /// binding that was impossible before: a keystroke has no path to give.
    #[test]
    fn a_command_reports_the_arguments_the_editor_will_collect() {
        let (ctx, env) = setup();
        let specs = eval_str("(command-args 'find-file)", &env, &ctx).expect("command-args");
        assert_eq!(
            format!("{specs:?}"),
            "((file \"Find file: \"))",
            "find-file should declare one file argument"
        );
        assert!(
            eval_str("(command-args 'save-buffer)", &env, &ctx)
                .expect("command-args")
                .is_nil(),
            "save-buffer takes no arguments"
        );
    }

    /// `defcommand` defines an ordinary function *and* registers it. Calling it
    /// from Lisp is a plain call -- no prompting, no ceremony.
    #[test]
    fn defcommand_defines_a_normal_function_and_registers_it() {
        let (ctx, env) = setup();
        eval_str(
            r#"(defcommand greet (who) ("sGreet whom: ") (concat "hello " who))"#,
            &env,
            &ctx,
        )
        .expect("defcommand");

        assert_eq!(
            eval_str("(greet \"world\")", &env, &ctx).expect("direct call"),
            LispExp::string("hello world".into()),
            "a command called directly from Lisp is just a function call"
        );
        assert_eq!(
            eval_str("(commandp 'greet)", &env, &ctx).expect("commandp"),
            LispExp::t()
        );
    }

    /// Specs and parameters cannot drift apart: the mismatch is an error when
    /// the command is defined, not a `WrongNumberOfArguments` the first time
    /// somebody runs it.
    #[test]
    fn a_spec_list_that_disagrees_with_the_parameters_is_rejected_at_definition() {
        let (ctx, env) = setup();
        let err = eval_str(
            r#"(defcommand two-args (a b) ("sOne: ") (list a b))"#,
            &env,
            &ctx,
        )
        .expect_err("registering 1 spec for 2 parameters must fail");
        assert!(
            format!("{err:?}").contains("argument spec"),
            "unhelpful error for a spec/parameter mismatch: {err:?}"
        );
    }

    /// A bad spec code is caught when the command is defined, too.
    #[test]
    fn an_unknown_argument_code_is_rejected() {
        let (ctx, env) = setup();
        let err = eval_str(r#"(register-command 'nope '("zBad: "))"#, &env, &ctx)
            .expect_err("an unknown spec code must fail");
        assert!(
            format!("{err:?}").contains('z'),
            "the error should name the offending code: {err:?}"
        );
    }

    // ---------------------------------------------------------------
    // Key dispatch
    // ---------------------------------------------------------------

    /// The gap this whole feature exists to close. A key bound to a command
    /// that needs an argument now prompts for it, which is why `find-file`
    /// could not be bound to a key before.
    #[test]
    fn a_key_bound_to_a_command_with_an_argument_prompts_for_it() {
        let (ctx, env) = setup();
        eval_str(
            r#"(progn
                 (setq probe nil)
                 (defcommand probe-cmd (text) ("sSay: ") (setq probe text))
                 (define-key nil "C-t" 'probe-cmd))"#,
            &env,
            &ctx,
        )
        .expect("setup");

        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('t'),
                modifiers: KeyModifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
            &env,
        );
        assert_eq!(
            current_buffer(&ctx),
            "*Minibuffer*",
            "pressing the key should have opened a prompt for the argument"
        );

        type_and_confirm(&ctx, &env, "hi");
        assert_eq!(
            env.get_variable("probe").expect("probe must be bound"),
            LispExp::string("hi".into()),
            "the command should have run with the text read from the minibuffer"
        );
    }

    /// Cancelling a prompt must abort the command rather than run it with a
    /// half-collected argument list.
    #[test]
    fn cancelling_a_prompt_abandons_the_command() {
        let (ctx, env) = setup();
        eval_str(
            r#"(progn
                 (setq probe 'untouched)
                 (defcommand probe-cmd (text) ("sSay: ") (setq probe text))
                 (define-key nil "C-t" 'probe-cmd))"#,
            &env,
            &ctx,
        )
        .expect("setup");

        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('t'),
                modifiers: KeyModifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
            &env,
        );
        ctx.handle_key_event(plain(KeyCode::Esc), &env);

        assert_eq!(
            env.get_variable("probe").expect("probe must be bound"),
            LispExp::symbol("untouched".into()),
            "escaping the prompt must not run the command"
        );
    }

    /// Typing is bound as `(self-insert "a")` -- arguments already supplied --
    /// so it keeps its old path and is not routed through `call-interactively`.
    #[test]
    fn ordinary_typing_still_reaches_the_buffer() {
        let (ctx, env) = setup();
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        let before = scratch.read().unwrap().text.len();
        for c in "abc".chars() {
            ctx.handle_key_event(key(c), &env);
        }
        assert_eq!(scratch.read().unwrap().text.len() - before, 3);
    }

    /// Regression test for a bug this routing fixed on the way past.
    ///
    /// `install_minibuffer` binds Enter, Escape and Tab to **bare symbols**,
    /// but evaluating a symbol looks up a *variable* -- so every one of those
    /// keys failed with `UnboundVariable("minibuffer-confirm")` in the real
    /// editor, making the minibuffer unusable. The suite never noticed because
    /// its tests confirm by evaluating `(minibuffer-confirm)` directly rather
    /// than by pressing the key.
    #[test]
    fn pressing_return_in_the_minibuffer_confirms_it() {
        let (ctx, env) = setup();
        eval_str(
            r#"(progn (setq got nil)
                      (minibuffer-read "P" (lambda (i) (setq got i)) nil nil))"#,
            &env,
            &ctx,
        )
        .expect("opening the prompt");
        assert_eq!(current_buffer(&ctx), "*Minibuffer*");

        type_and_confirm(&ctx, &env, "typed");
        assert_eq!(
            env.get_variable("got").expect("got must be bound"),
            LispExp::string("typed".into()),
            "Return in the minibuffer must run minibuffer-confirm"
        );
        assert_ne!(
            current_buffer(&ctx),
            "*Minibuffer*",
            "confirming should have closed the minibuffer"
        );
    }

    /// The same for Escape, which was broken in exactly the same way.
    #[test]
    fn pressing_escape_in_the_minibuffer_cancels_it() {
        let (ctx, env) = setup();
        eval_str(
            r#"(progn (setq cancelled nil)
                      (minibuffer-read "P" nil nil (lambda () (setq cancelled t))))"#,
            &env,
            &ctx,
        )
        .expect("opening the prompt");

        ctx.handle_key_event(plain(KeyCode::Esc), &env);
        assert_eq!(
            env.get_variable("cancelled")
                .expect("cancelled must be bound"),
            LispExp::t(),
            "Escape in the minibuffer must run minibuffer-cancel"
        );
    }

    // ---------------------------------------------------------------
    // M-x
    // ---------------------------------------------------------------

    #[test]
    fn m_x_runs_a_command_by_name() {
        let (ctx, env) = setup();
        eval_str(
            r#"(progn (setq probe nil)
                      (defcommand probe-cmd () nil (setq probe 'ran)))"#,
            &env,
            &ctx,
        )
        .expect("setup");

        ctx.handle_key_event(meta('x'), &env);
        assert_eq!(current_buffer(&ctx), "*Minibuffer*", "M-x should prompt");

        type_and_confirm(&ctx, &env, "probe-cmd");
        assert_eq!(
            env.get_variable("probe").expect("probe must be bound"),
            LispExp::symbol("ran".into())
        );
    }

    /// A command reached through M-x collects its arguments the same way one
    /// reached by a key does -- a second prompt opens after the first closes.
    #[test]
    fn m_x_prompts_for_a_commands_arguments_too() {
        let (ctx, env) = setup();
        eval_str(
            r#"(progn (setq probe nil)
                      (defcommand probe-cmd (text) ("sSay: ") (setq probe text)))"#,
            &env,
            &ctx,
        )
        .expect("setup");

        ctx.handle_key_event(meta('x'), &env);
        type_and_confirm(&ctx, &env, "probe-cmd");
        assert_eq!(
            current_buffer(&ctx),
            "*Minibuffer*",
            "a second prompt should have opened for the command's argument"
        );

        type_and_confirm(&ctx, &env, "chained");
        assert_eq!(
            env.get_variable("probe").expect("probe must be bound"),
            LispExp::string("chained".into())
        );
    }

    /// Two arguments means two prompts in sequence, which is the whole reason
    /// argument collection is a continuation chain rather than a loop.
    #[test]
    fn a_two_argument_command_prompts_twice_and_keeps_the_order() {
        let (ctx, env) = setup();
        eval_str(
            r#"(progn (setq probe nil)
                      (defcommand probe-cmd (from to) ("sFrom: " "sTo: ")
                        (setq probe (concat from "->" to))))"#,
            &env,
            &ctx,
        )
        .expect("setup");

        ctx.handle_key_event(meta('x'), &env);
        type_and_confirm(&ctx, &env, "probe-cmd");
        type_and_confirm(&ctx, &env, "a");
        assert_eq!(
            current_buffer(&ctx),
            "*Minibuffer*",
            "the second argument should still be being prompted for"
        );
        type_and_confirm(&ctx, &env, "b");

        assert_eq!(
            env.get_variable("probe").expect("probe must be bound"),
            LispExp::string("a->b".into()),
            "arguments must reach the command in the order they were prompted"
        );
    }

    /// The command system is mandatory functionality, so it must work with no
    /// `.lisp` file loaded at all -- registry, M-x, its keybinding, argument
    /// collection and completion are all in Rust. `commands.lisp` adds only
    /// `defcommand` sugar.
    ///
    /// This is the test that keeps that true: it boots bare, registers a
    /// command with `register-command` alone, and drives the whole flow from
    /// the M-x keystroke to the command running with a prompted argument.
    #[test]
    fn the_command_system_works_with_no_lisp_files_loaded() {
        let (ctx, env) = bare();
        eval_str(
            r#"(progn (setq probe nil)
                      (defun probe-cmd (text) (setq probe text))
                      (register-command 'probe-cmd '("sSay: ")))"#,
            &env,
            &ctx,
        )
        .expect("registering without commands.lisp");

        ctx.handle_key_event(meta('x'), &env);
        assert_eq!(
            current_buffer(&ctx),
            "*Minibuffer*",
            "M-x must be bound and functional with no .lisp file loaded"
        );

        type_and_confirm(&ctx, &env, "probe-cmd");
        assert_eq!(
            current_buffer(&ctx),
            "*Minibuffer*",
            "the command's argument must still be prompted for"
        );

        type_and_confirm(&ctx, &env, "bare");
        assert_eq!(
            env.get_variable("probe").expect("probe must be bound"),
            LispExp::string("bare".into()),
            "the command must have run with the argument collected in Rust"
        );
    }

    /// Buffer-name arguments complete over the live buffers, with no Lisp
    /// involved.
    #[test]
    fn a_buffer_argument_completes_over_live_buffers() {
        let (ctx, env) = bare();
        eval_str(
            r#"(progn (setq probe nil)
                      (defun probe-cmd (b) (setq probe b))
                      (register-command 'probe-cmd '("bBuffer: ")))"#,
            &env,
            &ctx,
        )
        .expect("setup");

        ctx.handle_key_event(meta('x'), &env);
        type_and_confirm(&ctx, &env, "probe-cmd");
        // Tab completes the buffer name from its first character.
        ctx.handle_key_event(key('*'), &env);
        ctx.handle_key_event(plain(KeyCode::Tab), &env);
        ctx.handle_key_event(plain(KeyCode::Enter), &env);

        assert_eq!(
            env.get_variable("probe").expect("probe must be bound"),
            LispExp::string("*scratch*".into()),
            "Tab should have completed the only matching buffer name"
        );
    }

    /// A numeric argument reaches the command as a number, not the string the
    /// user typed.
    #[test]
    fn a_numeric_argument_is_converted_before_the_command_sees_it() {
        let (ctx, env) = bare();
        eval_str(
            r#"(progn (setq probe nil)
                      (defun probe-cmd (n) (setq probe (+ n 1)))
                      (register-command 'probe-cmd '("nHow many: ")))"#,
            &env,
            &ctx,
        )
        .expect("setup");

        ctx.handle_key_event(meta('x'), &env);
        type_and_confirm(&ctx, &env, "probe-cmd");
        type_and_confirm(&ctx, &env, "41");

        assert_eq!(
            env.get_variable("probe").expect("probe must be bound"),
            LispExp::number(42.0),
            "a number spec must hand the command a number"
        );
    }

    /// A prompt closed by neither confirm nor cancel abandons its command; the
    /// next one must still collect its own argument and run normally.
    ///
    /// Note what this does *not* cover. `call-interactively` also clears the
    /// pending stack when it starts with no minibuffer open, and that guard is
    /// defensive hygiene rather than a fix for a reachable bug: because the
    /// stack is a stack, a new command is pushed above any orphan and pops
    /// itself off, so an orphan leaks memory but cannot steal input. Removing
    /// the guard does not fail this test, and it is not claimed to.
    #[test]
    fn an_abandoned_prompt_does_not_disturb_the_next_command() {
        let (ctx, env) = bare();
        eval_str(
            r#"(progn (setq first-arg nil) (setq second-arg nil)
                      (defun first-cmd (t1) (setq first-arg t1))
                      (defun second-cmd (t2) (setq second-arg t2))
                      (register-command 'first-cmd '("sOne: "))
                      (register-command 'second-cmd '("sTwo: ")))"#,
            &env,
            &ctx,
        )
        .expect("setup");

        // Start the first command, then close its prompt behind its back --
        // neither Return nor Escape, so no callback fires and its pending
        // entry is orphaned.
        eval_str("(call-interactively 'first-cmd)", &env, &ctx).expect("first prompt");
        assert_eq!(current_buffer(&ctx), "*Minibuffer*");
        eval_str("(close-buffer \"*Minibuffer*\")", &env, &ctx).expect("closing behind its back");

        // The second command must collect its own argument, not inherit the
        // abandoned one.
        eval_str("(call-interactively 'second-cmd)", &env, &ctx).expect("second prompt");
        type_and_confirm(&ctx, &env, "mine");

        assert_eq!(
            env.get_variable("second-arg").expect("second-arg bound"),
            LispExp::string("mine".into()),
            "the second command should have received its own input"
        );
        assert!(
            env.get_variable("first-arg")
                .expect("first-arg bound")
                .is_nil(),
            "the abandoned command must never run"
        );
    }

    #[test]
    fn m_x_reports_an_unknown_command_instead_of_failing() {
        let (ctx, env) = bare();
        ctx.handle_key_event(meta('x'), &env);
        type_and_confirm(&ctx, &env, "no-such-command");
        assert!(
            ctx.get_echo_message().contains("No such command"),
            "expected a message about the unknown command, got {:?}",
            ctx.get_echo_message()
        );
    }

    #[test]
    fn m_x_completion_offers_only_matching_commands() {
        let (ctx, env) = bare();
        let matches = eval_str("(command-completions \"save-\")", &env, &ctx).expect("completion");
        let rendered = format!("{matches:?}");
        assert!(
            rendered.contains("save-buffer"),
            "save-buffer should complete from \"save-\": {rendered}"
        );
        assert!(
            !rendered.contains("next-line"),
            "completion should not offer non-matching commands: {rendered}"
        );
    }
}
