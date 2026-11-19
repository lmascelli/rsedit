//! The minor modes the editor ships, in the buffer roles: the three rules the
//! modules give -- `show-paren-mode` in prompts, `visual-line-mode` in texts
//! to read, `vim-mode` in views -- the modes that have none and so go where
//! editing goes, the
//! global modes reaching only the buffers that admit them, and the ordinary
//! keys, which move by visual rows where a buffer wraps and give way to a
//! prompt's history and a list's choosing.
//!
//! Design: `doc/architettura-editor.typ, <ruoli>` and `<righe-visive>`.
#[cfg(test)]
mod tests {
    use crate::buffer::BufferRole;
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::managers::{Admission, PolicyOrigin};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const PROMPT: &str = "*Minibuffer*";
    const COMPLETIONS: &str = "*Completions*";

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn text(exp: &LispExp<Ctx>) -> String {
        match exp {
            LispExp::String(text) => text.to_string(),
            LispExp::Symbol(name) => name.to_string(),
            other => format!("{other:?}"),
        }
    }

    /// The editor with every module the default init.lisp loads.
    fn booted() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        crate::tests::shipped::load_default_modules(&env, &ctx);
        (ctx, env)
    }

    fn key(code: KeyCode, ctrl: bool, alt: bool) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers {
                ctrl,
                alt,
                ..Default::default()
            },
        }
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode) {
        ctx.handle_key_event(key(code, false, false), env);
    }

    fn ctrl(ctx: &Ctx, env: &Arc<Env<Ctx>>, c: char) {
        ctx.handle_key_event(key(KeyCode::Char(c), true, false), env);
    }

    fn type_text(ctx: &Ctx, env: &Arc<Env<Ctx>>, text: &str) {
        for c in text.chars() {
            press(ctx, env, KeyCode::Char(c));
        }
    }

    fn on(minor: &str, buffer: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        run(&format!("(minor-mode-p '{minor} {buffer:?})"), env, ctx).is_truthy()
    }

    fn admitted(minor: &str, buffer: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        run(
            &format!("(minor-mode-refusal '{minor} {buffer:?})"),
            env,
            ctx,
        )
        .is_nil()
    }

    /// The command KEYS runs in the current buffer, by name.
    fn bound(keys: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        // A bare symbol from Rust, a one-element form from `define-key'.
        text(&run(
            &format!(
                r#"(let ((target (nth 1 (key-binding "{keys}"))))
                     (if (listp target) (car target) target))"#
            ),
            env,
            ctx,
        ))
    }

    fn number(exp: &LispExp<Ctx>) -> f64 {
        match exp {
            LispExp::Number(n) => *n,
            other => panic!("not a number: {other:?}"),
        }
    }

    /// A buffer of each role, named after it, made with `buffer-create`.
    fn one_of_each_role(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Vec<(BufferRole, String)> {
        BufferRole::ALL
            .into_iter()
            .map(|role| {
                let name = format!("*{}*", role.name());
                run(
                    &format!("(buffer-create {name:?} :role '{})", role.name()),
                    env,
                    ctx,
                );
                (role, name)
            })
            .collect()
    }

    /// Opens the help for `buffer-create`, by keys, and returns its buffer.
    fn help_page(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        ctx.handle_key_event(key(KeyCode::Char('x'), false, true), env);
        type_text(ctx, env, "describe-function");
        press(ctx, env, KeyCode::Enter);
        type_text(ctx, env, "buffer-create");
        press(ctx, env, KeyCode::Enter);
        let help = text(&run("help-buffer-name", env, ctx));
        assert_eq!(
            text(&run(&format!("(buffer-role {help:?})"), env, ctx)),
            "document-view"
        );
        help
    }

    // ----------------------------------------------------------------
    // What each role admits of what the editor ships
    // ----------------------------------------------------------------

    /// The modes that have no rule of their own: what an editing buffer
    /// admits, and nothing else does.
    const EDITING_ONLY: [&str; 13] = [
        "electric-indent-mode",
        "electric-pair-mode",
        "whitespace-mode",
        "whitespace-cleanup-mode",
        "auto-revert-mode",
        "findings-mode",
        "hl-line-mode",
        "display-line-numbers-mode",
        "auto-fill-mode",
        "auto-fill-column",
        "overwrite-mode",
        "subword-mode",
        "lsp-mode",
    ];

    #[test]
    fn the_ordinary_modes_go_where_editing_goes_and_nowhere_else() {
        let (ctx, env) = booted();
        for (role, buffer) in one_of_each_role(&env, &ctx) {
            for minor in EDITING_ONLY {
                assert_eq!(
                    admitted(minor, &buffer, &env, &ctx),
                    role == BufferRole::Editing,
                    "{minor} in {}",
                    role.name()
                );
            }
        }
    }

    #[test]
    fn show_paren_goes_in_prompts_and_wrapping_in_texts_to_read() {
        let (ctx, env) = booted();
        for (role, buffer) in one_of_each_role(&env, &ctx) {
            use BufferRole::*;
            assert_eq!(
                admitted("show-paren-mode", &buffer, &env, &ctx),
                matches!(role, Editing | Prompt),
                "show-paren-mode in {}",
                role.name()
            );
            assert_eq!(
                admitted("visual-line-mode", &buffer, &env, &ctx),
                matches!(role, Editing | DocumentView),
                "visual-line-mode in {}",
                role.name()
            );
        }
        // Rules of the distributed configuration, each in the role it names.
        ctx.modes(|modes| {
            assert_eq!(
                modes.minor_policy(
                    BufferRole::Prompt,
                    "show-paren-mode",
                    PolicyOrigin::Distributed
                ),
                Some(Admission::Permit)
            );
            assert_eq!(
                modes.minor_policy(
                    BufferRole::DocumentView,
                    "visual-line-mode",
                    PolicyOrigin::Distributed
                ),
                Some(Admission::Permit)
            );
            assert_eq!(
                modes.minor_policy(
                    BufferRole::View,
                    "visual-line-mode",
                    PolicyOrigin::Distributed
                ),
                None
            );
        });
    }

    #[test]
    fn vim_goes_in_views_and_never_in_a_prompt_or_a_list_of_candidates() {
        let (ctx, env) = booted();
        for (role, buffer) in one_of_each_role(&env, &ctx) {
            use BufferRole::*;
            assert_eq!(
                admitted("vim-mode", &buffer, &env, &ctx),
                matches!(role, Editing | View | DocumentView),
                "vim-mode in {}",
                role.name()
            );
        }
        ctx.modes(|modes| {
            assert_eq!(
                modes.minor_policy(BufferRole::View, "vim-mode", PolicyOrigin::Distributed),
                Some(Admission::Permit)
            );
            // A text to read has it from view, which it derives from.
            assert_eq!(
                modes.minor_policy(
                    BufferRole::DocumentView,
                    "vim-mode",
                    PolicyOrigin::Distributed
                ),
                None
            );
        });
    }

    #[test]
    fn show_paren_is_on_at_the_prompt_split_or_floating_and_not_in_its_candidates() {
        for floating in [false, true] {
            let (ctx, env) = booted();
            if floating {
                run(
                    "(setq *minibuffer-read-function* 'floating-minibuffer-prompt)",
                    &env,
                    &ctx,
                );
            }
            ctx.handle_key_event(key(KeyCode::Char('x'), false, true), &env);
            assert!(
                on("show-paren-mode", PROMPT, &env, &ctx),
                "floating {floating}"
            );
            type_text(&ctx, &env, "buffer-li");
            press(&ctx, &env, KeyCode::Tab);
            assert!(
                !on("show-paren-mode", COMPLETIONS, &env, &ctx),
                "floating {floating}"
            );
            assert!(on("show-paren-mode", "*scratch*", &env, &ctx));
        }
    }

    #[test]
    fn global_wrapping_reaches_what_is_edited_and_read_and_global_off_takes_it_back() {
        let (ctx, env) = booted();
        let help = help_page(&env, &ctx);
        run("(global-visual-line-mode 1)", &env, &ctx);
        // A buffer edited in the major mode of a list of candidates: no
        // longer kept out by its major mode, only roles decide.
        run(
            r#"(buffer-create "edited-list" :mode 'completion-mode)"#,
            &env,
            &ctx,
        );
        run(r#"(buffer-create "a-view" :role 'view)"#, &env, &ctx);
        assert!(on("visual-line-mode", "*scratch*", &env, &ctx));
        assert!(on("visual-line-mode", &help, &env, &ctx));
        assert!(on("visual-line-mode", "edited-list", &env, &ctx));
        assert!(!on("visual-line-mode", "a-view", &env, &ctx));
        m_x_tab(&ctx, &env);
        assert!(!on("visual-line-mode", PROMPT, &env, &ctx));
        assert!(!on("visual-line-mode", COMPLETIONS, &env, &ctx));
        press(&ctx, &env, KeyCode::Esc);
        press(&ctx, &env, KeyCode::Esc);

        run("(global-visual-line-mode 0)", &env, &ctx);
        for buffer in ["*scratch*", help.as_str(), "edited-list"] {
            assert!(!on("visual-line-mode", buffer, &env, &ctx), "{buffer}");
        }
    }

    /// Opens M-x, types the start of a command and asks for its candidates.
    fn m_x_tab(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        ctx.handle_key_event(key(KeyCode::Char('x'), false, true), env);
        type_text(ctx, env, "buffer-li");
        press(ctx, env, KeyCode::Tab);
        assert_eq!(
            text(&run("(buffer-role \"*Completions*\")", env, ctx)),
            "completion"
        );
    }

    // ----------------------------------------------------------------
    // The rules from Lisp, and over them the user's
    // ----------------------------------------------------------------

    #[test]
    fn a_distributed_rule_is_checked_whole_and_says_whether_it_changed_anything() {
        let (ctx, env) = booted();
        for (src, says) in [
            (
                "(minor-mode--distributed-policy 'no-such-mode 'allow 'view)",
                "No such minor mode",
            ),
            (
                "(minor-mode--distributed-policy 'hl-line-mode 'maybe 'view)",
                "allow, deny or inherit",
            ),
            (
                "(minor-mode--distributed-policy 'hl-line-mode 'allow 'shelf)",
                "no buffer role",
            ),
            (
                "(minor-mode--distributed-policy 'hl-line-mode 'allow nil)",
                "buffer role",
            ),
            (
                "(minor-mode--distributed-policy 'hl-line-mode 'allow)",
                "WrongNumberOfArguments",
            ),
        ] {
            let why = match eval_str(src, &env, &ctx) {
                Ok(value) => panic!("{src} should have failed, and gave {value:?}"),
                Err(why) => format!("{why:?}"),
            };
            assert!(why.contains(says), "{src}: {why}");
        }
        assert_eq!(
            ctx.modes(|modes| modes.minor_policy(
                BufferRole::View,
                "hl-line-mode",
                PolicyOrigin::Distributed
            )),
            None,
            "nothing is recorded by a call refused"
        );
        let set = "(minor-mode--distributed-policy 'hl-line-mode 'allow 'view)";
        assert!(run(set, &env, &ctx).is_truthy());
        assert!(
            run(set, &env, &ctx).is_nil(),
            "the same rule twice changes nothing"
        );
        assert!(
            run(
                "(minor-mode--distributed-policy 'hl-line-mode 'inherit 'view)",
                &env,
                &ctx
            )
            .is_truthy()
        );
        assert!(
            run(
                "(minor-mode--distributed-policy 'hl-line-mode 'inherit 'view)",
                &env,
                &ctx
            )
            .is_nil()
        );
    }

    #[test]
    fn the_users_rule_wins_over_the_distributed_one_and_a_revoke_turns_wrapping_off() {
        let (ctx, env) = booted();
        let help = help_page(&env, &ctx);
        run(
            r#"(setq wrap-hook-runs 0)
               (defun count-wrap-hook (&rest ignored)
                 (setq wrap-hook-runs (+ wrap-hook-runs 1)))
               (add-hook 'visual-line-mode "minor-mode-hook" 'count-wrap-hook)
               (global-visual-line-mode 1)"#,
            &env,
            &ctx,
        );
        assert!(on("visual-line-mode", &help, &env, &ctx));
        let before = run("wrap-hook-runs", &env, &ctx);

        // The user keeps wrapping out of the texts to read: off at once,
        // with its hook, as for any other way off.
        ctx.set_minor_policy(
            BufferRole::DocumentView,
            "visual-line-mode",
            PolicyOrigin::User,
            Some(Admission::Revoke),
            &env,
        );
        assert!(!on("visual-line-mode", &help, &env, &ctx));
        assert!(on("visual-line-mode", "*scratch*", &env, &ctx));
        let after = run("wrap-hook-runs", &env, &ctx);
        assert_eq!(number(&after), number(&before) + 1.0, "the hook ran once");
        let why = text(&run(
            &format!("(minor-mode-refusal 'visual-line-mode {help:?})"),
            &env,
            &ctx,
        ));
        assert!(why.contains("the user configuration"), "{why}");

        // The user's rule gone, the distributed permit holds again: from the
        // next time the mode is asked for, not before.
        ctx.set_minor_policy(
            BufferRole::DocumentView,
            "visual-line-mode",
            PolicyOrigin::User,
            None,
            &env,
        );
        assert!(!on("visual-line-mode", &help, &env, &ctx));
        run(
            &format!(
                "(with-current-buffer {help:?} (lambda (&rest ignored) (visual-line-mode 1)))"
            ),
            &env,
            &ctx,
        );
        assert!(on("visual-line-mode", &help, &env, &ctx));

        // And the distributed rule dropped, document-view asks view, which
        // has no rule: wrapping goes from the page.
        run(
            "(minor-mode--distributed-policy 'visual-line-mode 'inherit 'document-view)",
            &env,
            &ctx,
        );
        assert!(!on("visual-line-mode", &help, &env, &ctx));
    }

    // ----------------------------------------------------------------
    // The ordinary keys
    // ----------------------------------------------------------------

    /// The prompt's text after the label: what was typed or recalled.
    fn prompt_text(ctx: &Ctx) -> String {
        ctx.with_buffer(PROMPT, |buf| buf.text.to_string())
            .expect("a prompt is open")
    }

    #[test]
    fn at_a_prompt_that_wraps_the_history_keys_still_recall() {
        for floating in [false, true] {
            let (ctx, env) = booted();
            if floating {
                run(
                    "(setq *minibuffer-read-function* 'floating-minibuffer-prompt)",
                    &env,
                    &ctx,
                );
            }
            // Wrapping wherever it can be, the prompts included.
            ctx.set_minor_policy(
                BufferRole::Prompt,
                "visual-line-mode",
                PolicyOrigin::User,
                Some(Admission::Permit),
                &env,
            );
            run("(global-visual-line-mode 1)", &env, &ctx);
            ctx.handle_key_event(key(KeyCode::Char('x'), false, true), &env);
            type_text(&ctx, &env, "buffer-list");
            press(&ctx, &env, KeyCode::Enter);

            ctx.handle_key_event(key(KeyCode::Char('x'), false, true), &env);
            assert!(
                on("visual-line-mode", PROMPT, &env, &ctx),
                "floating {floating}"
            );
            for (recall, back) in [
                (KeyCode::Up, KeyCode::Down),
                (KeyCode::Char('p'), KeyCode::Char('n')),
            ] {
                let is_ctrl = matches!(recall, KeyCode::Char(_));
                ctx.handle_key_event(key(recall.clone(), is_ctrl, false), &env);
                assert!(
                    prompt_text(&ctx).contains("buffer-list"),
                    "floating {floating}, {recall:?}: {:?}",
                    prompt_text(&ctx)
                );
                ctx.handle_key_event(key(back.clone(), is_ctrl, false), &env);
                assert!(
                    !prompt_text(&ctx).contains("buffer-list"),
                    "floating {floating}, {back:?}: {:?}",
                    prompt_text(&ctx)
                );
            }
            // The keys of the line's ends are the ordinary ones.
            assert_eq!(bound("C-a", &env, &ctx), "move-beginning-of-line");
            assert_eq!(bound("<end>", &env, &ctx), "move-end-of-line");
        }
    }

    #[test]
    fn in_a_list_of_candidates_that_wraps_the_arrows_still_choose() {
        let (ctx, env) = booted();
        ctx.set_minor_policy(
            BufferRole::Completion,
            "visual-line-mode",
            PolicyOrigin::User,
            Some(Admission::Permit),
            &env,
        );
        run("(global-visual-line-mode 1)", &env, &ctx);
        m_x_tab(&ctx, &env);
        assert!(on("visual-line-mode", COMPLETIONS, &env, &ctx));
        for (keys, command) in [
            ("<down>", "completion-next"),
            ("<up>", "completion-previous"),
            ("C-n", "completion-next"),
            ("C-p", "completion-previous"),
        ] {
            assert_eq!(bound(keys, &env, &ctx), command, "{keys}");
        }
        // The list closed with C-g, the arrows go back to the history.
        ctrl(&ctx, &env, 'g');
        assert_eq!(bound("<up>", &env, &ctx), "history-previous");
        assert_eq!(bound("C-p", &env, &ctx), "history-previous");
    }
}
