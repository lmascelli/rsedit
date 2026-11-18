//! Buffer roles from the user's configuration, and the roles at work as a
//! person meets them: `define-buffer-role`, `set-minor-mode-policy`,
//! `minor-mode-allowed-p` and `buffer-role`, each checked whole before it
//! changes anything; the manual's log-view example run as written; roles
//! derived from the shipped ones; a revocation of a mode that is on; and,
//! by keys, M-x and its history and candidates with wrapping on everywhere,
//! split and floating, a reader of the user's written before roles,
//! incremental search, Vim, and the ordinary keys in a buffer being edited.
//!
//! Design: `doc/architettura-editor.typ, <ruoli-configurazione>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::buffer::{BufferRole, BufferTrait};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::managers::{Admission, PolicyOrigin};
    use crate::tests::shipped::man_directory;
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

    /// Returns what evaluating SRC was refused with, which it must be.
    fn refused(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match eval_str(src, env, ctx) {
            Ok(value) => panic!("{src} was not refused: {value:?}"),
            Err(why) => format!("{why:?}"),
        }
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

    /// As [`booted`], with every prompt the floating one when FLOATING.
    fn booted_with_prompt(floating: bool) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = booted();
        if floating {
            run(
                "(setq *minibuffer-read-function* 'floating-minibuffer-prompt)",
                &env,
                &ctx,
            );
        }
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

    fn meta_x(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        ctx.handle_key_event(key(KeyCode::Char('x'), false, true), env);
    }

    fn type_text(ctx: &Ctx, env: &Arc<Env<Ctx>>, text: &str) {
        for c in text.chars() {
            press(ctx, env, KeyCode::Char(c));
        }
    }

    /// Runs COMMAND with M-x, by keys.
    fn m_x(ctx: &Ctx, env: &Arc<Env<Ctx>>, command: &str) {
        meta_x(ctx, env);
        type_text(ctx, env, command);
        press(ctx, env, KeyCode::Enter);
        assert!(
            !ctx.has_buffer(PROMPT),
            "M-x {command} left its prompt open"
        );
    }

    fn on(minor: &str, buffer: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        run(&format!("(minor-mode-p '{minor} {buffer:?})"), env, ctx).is_truthy()
    }

    fn allowed(src_args: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        run(&format!("(minor-mode-allowed-p {src_args})"), env, ctx).is_truthy()
    }

    fn role_of(buffer: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text(&run(&format!("(buffer-role {buffer:?})"), env, ctx))
    }

    fn user_rule(ctx: &Ctx, role: &str, minor: &str) -> Option<Admission> {
        ctx.modes(|modes| {
            let role = modes.role_named(role).expect("a role");
            modes.minor_policy(role, minor, PolicyOrigin::User)
        })
    }

    fn number(exp: &LispExp<Ctx>) -> f64 {
        match exp {
            LispExp::Number(n) => *n,
            other => panic!("not a number: {other:?}"),
        }
    }

    /// The text of the open prompt: what was typed or recalled.
    fn prompt_text(ctx: &Ctx) -> String {
        ctx.with_buffer(PROMPT, |buf| buf.text.to_string())
            .expect("a prompt is open")
    }

    /// The candidate the list has selected: its region, which is what is
    /// drawn highlighted.
    fn selected(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text(&run(
            r#"(with-current-buffer "*Completions*"
                 (lambda () (substring (buffer-string) (region-beginning) (region-end))))"#,
            env,
            ctx,
        ))
    }

    /// Wrapping wherever it can go, the prompts and the lists of candidates
    /// included, as a user who wants it everywhere configures it.
    fn wrap_everywhere(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            r#"(set-minor-mode-policy 'visual-line-mode 'allow :role 'prompt)
               (set-minor-mode-policy 'visual-line-mode 'allow :role 'completion)
               (global-visual-line-mode 1)"#,
            env,
            ctx,
        );
    }

    // ----------------------------------------------------------------
    // define-buffer-role
    // ----------------------------------------------------------------

    #[test]
    fn a_defined_role_is_given_to_buffers_and_answers_through_its_lineage() {
        let (ctx, env) = booted();
        assert_eq!(
            text(&run(
                "(define-buffer-role 'log-view :parent 'view)",
                &env,
                &ctx
            )),
            "log-view"
        );
        run(
            r#"(setq roles-seen nil)
               (defun note-role () (setq roles-seen (cons (buffer-role) roles-seen)))
               (add-hook nil "buffer-created-hook" 'note-role)
               (buffer-create "*log*" :role 'log-view)"#,
            &env,
            &ctx,
        );
        // Given before the first hook, and read back by name.
        assert_eq!(text(&run("(car roles-seen)", &env, &ctx)), "log-view");
        assert_eq!(role_of("*log*", &env, &ctx), "log-view");
        let role = ctx.with_buffer("*log*", |buf| buf.role()).unwrap();
        assert_eq!(role.parent(), Some(BufferRole::View));
        assert!(role.derives_from(BufferRole::View));
        assert!(!role.open_by_default());
        for (ancestor, derives) in [
            ("log-view", true),
            ("view", true),
            ("document-view", false),
            ("editing", false),
        ] {
            assert_eq!(
                run(
                    &format!("(buffer-role-derived-p 'log-view '{ancestor})"),
                    &env,
                    &ctx
                )
                .is_truthy(),
                derives,
                "{ancestor}"
            );
        }
        // A buffer of the role asked for as its parent is handed back; one of
        // the parent asked for as the role is not.
        assert_eq!(
            text(&run(r#"(buffer-create "*log*" :role 'view)"#, &env, &ctx)),
            "*log*"
        );
        run(r#"(buffer-create "plain-view" :role 'view)"#, &env, &ctx);
        let why = refused(
            r#"(buffer-create "plain-view" :role 'log-view)"#,
            &env,
            &ctx,
        );
        assert!(why.contains("not log-view"), "{why}");
        // A root of its own admits only what is permitted for it.
        run(
            r#"(define-buffer-role 'kiosk)
               (buffer-create "*kiosk*" :role 'kiosk)"#,
            &env,
            &ctx,
        );
        let kiosk = ctx.with_buffer("*kiosk*", |buf| buf.role()).unwrap();
        assert_eq!(kiosk.parent(), None);
        assert!(!allowed("'hl-line-mode \"*kiosk*\"", &env, &ctx));
        assert!(allowed("'read-only-mode \"*kiosk*\"", &env, &ctx));
    }

    #[test]
    fn defining_a_role_again_the_same_way_changes_nothing() {
        let (ctx, env) = booted();
        run(
            r#"(define-buffer-role 'log-view :parent 'view)
               (buffer-create "*log*" :role 'log-view)
               (set-minor-mode-policy 'hl-line-mode 'allow :role 'log-view)"#,
            &env,
            &ctx,
        );
        let before = ctx.with_buffer("*log*", |buf| buf.role()).unwrap();
        for _ in 0..2 {
            assert_eq!(
                text(&run(
                    "(define-buffer-role 'log-view :parent 'view)",
                    &env,
                    &ctx
                )),
                "log-view"
            );
        }
        let after = ctx.modes(|modes| modes.role_named("log-view")).unwrap();
        assert_eq!(after, before, "the same role");
        assert_eq!(
            user_rule(&ctx, "log-view", "hl-line-mode"),
            Some(Admission::Permit),
            "and its rules kept"
        );
        // The shipped roles, as they are, are no redefinition either.
        for src in [
            "(define-buffer-role 'document-view :parent 'view)",
            "(define-buffer-role 'view)",
            "(define-buffer-role 'editing)",
        ] {
            run(src, &env, &ctx);
        }
        // Two editors that know the same name know it each their own way.
        let (other, other_env) = booted();
        run(
            "(define-buffer-role 'log-view :parent 'document-view)",
            &other_env,
            &other,
        );
        assert_eq!(
            other
                .modes(|modes| modes.role_named("log-view"))
                .unwrap()
                .parent(),
            Some(BufferRole::DocumentView)
        );
        assert_eq!(after.parent(), Some(BufferRole::View));
    }

    #[test]
    fn a_define_buffer_role_wrong_anywhere_defines_nothing() {
        let (ctx, env) = booted();
        run("(define-buffer-role 'log-view :parent 'view)", &env, &ctx);
        for (src, says) in [
            (
                "(define-buffer-role 'fresh :parent 'shelf)",
                "no buffer role",
            ),
            (
                "(define-buffer-role 'fresh :parent 'view :parent 'view)",
                "given twice",
            ),
            ("(define-buffer-role 'fresh :base 'view)", "unknown keyword"),
            ("(define-buffer-role 'fresh :parent)", "no value"),
            (
                "(define-buffer-role 'fresh 'view)",
                "WrongNumberOfArguments",
            ),
            ("(define-buffer-role \"fresh\" :parent 'view)", "Symbol"),
            ("(define-buffer-role :fresh)", "Symbol"),
            ("(define-buffer-role nil)", "cannot name"),
            ("(define-buffer-role)", "WrongNumberOfArguments"),
        ] {
            let why = refused(src, &env, &ctx);
            assert!(why.contains(says), "{src}: {why}");
            assert!(
                ctx.modes(|modes| modes.role_named("fresh")).is_none(),
                "{src} defined it"
            );
        }
        // Another parent for a role there is, shipped or defined, is refused,
        // and the role stays what it was.
        for (src, name, parent) in [
            (
                "(define-buffer-role 'log-view :parent 'editing)",
                "log-view",
                Some(BufferRole::View),
            ),
            (
                "(define-buffer-role 'log-view)",
                "log-view",
                Some(BufferRole::View),
            ),
            ("(define-buffer-role 'view :parent 'editing)", "view", None),
            (
                "(define-buffer-role 'document-view :parent 'prompt)",
                "document-view",
                Some(BufferRole::View),
            ),
            (
                "(define-buffer-role 'document-view)",
                "document-view",
                Some(BufferRole::View),
            ),
            (
                "(define-buffer-role 'editing :parent 'view)",
                "editing",
                None,
            ),
        ] {
            let why = refused(src, &env, &ctx);
            assert!(why.contains("exists already"), "{src}: {why}");
            assert_eq!(
                ctx.modes(|modes| modes.role_named(name)).unwrap().parent(),
                parent,
                "{src}"
            );
        }
    }

    // ----------------------------------------------------------------
    // set-minor-mode-policy and minor-mode-allowed-p
    // ----------------------------------------------------------------

    #[test]
    fn a_set_minor_mode_policy_wrong_anywhere_changes_nothing() {
        let (ctx, env) = booted();
        run("(global-visual-line-mode 1)", &env, &ctx);
        let help = run(
            r#"(buffer-create "*page*" :role 'document-view)"#,
            &env,
            &ctx,
        );
        let help = text(&help);
        run(
            &format!("(with-current-buffer {help:?} (lambda () (visual-line-mode 1)))"),
            &env,
            &ctx,
        );
        assert!(on("visual-line-mode", &help, &env, &ctx));
        for (src, says) in [
            (
                "(set-minor-mode-policy 'no-such-mode 'deny :role 'document-view)",
                "No such minor mode",
            ),
            (
                "(set-minor-mode-policy 'visual-line-mode 'maybe :role 'document-view)",
                "allow, deny or inherit",
            ),
            (
                "(set-minor-mode-policy 'visual-line-mode 'deny :role 'shelf)",
                "no buffer role",
            ),
            (
                "(set-minor-mode-policy 'visual-line-mode 'deny)",
                "with :role",
            ),
            (
                "(set-minor-mode-policy 'visual-line-mode 'deny :role nil)",
                "with :role",
            ),
            (
                "(set-minor-mode-policy 'visual-line-mode 'deny :role 'document-view :role 'document-view)",
                "given twice",
            ),
            (
                "(set-minor-mode-policy 'visual-line-mode 'deny :roles 'document-view)",
                "unknown keyword",
            ),
            (
                "(set-minor-mode-policy 'visual-line-mode 'deny 'document-view)",
                "WrongNumberOfArguments",
            ),
            (
                "(set-minor-mode-policy 'visual-line-mode :role 'document-view)",
                "WrongNumberOfArguments",
            ),
        ] {
            let why = refused(src, &env, &ctx);
            assert!(why.contains(says), "{src}: {why}");
            assert_eq!(
                user_rule(&ctx, "document-view", "visual-line-mode"),
                None,
                "{src}"
            );
            assert!(on("visual-line-mode", &help, &env, &ctx), "{src}");
        }
    }

    #[test]
    fn the_users_policy_wins_for_its_role_and_inherit_takes_it_back() {
        let (ctx, env) = booted();
        run(
            r#"(buffer-create "*page*" :role 'document-view)"#,
            &env,
            &ctx,
        );
        assert!(allowed("'visual-line-mode \"*page*\"", &env, &ctx));
        assert!(
            run(
                "(set-minor-mode-policy 'visual-line-mode 'deny :role 'document-view)",
                &env,
                &ctx
            )
            .is_truthy()
        );
        assert!(
            run(
                "(set-minor-mode-policy 'visual-line-mode 'deny :role 'document-view)",
                &env,
                &ctx
            )
            .is_nil(),
            "already so"
        );
        assert_eq!(
            user_rule(&ctx, "document-view", "visual-line-mode"),
            Some(Admission::Revoke)
        );
        assert!(!allowed("'visual-line-mode \"*page*\"", &env, &ctx));
        let why = text(&run(
            r#"(minor-mode-refusal 'visual-line-mode "*page*")"#,
            &env,
            &ctx,
        ));
        assert!(why.contains("the user configuration"), "{why}");
        // The distributed rule is still there, under the user's.
        ctx.modes(|modes| {
            assert_eq!(
                modes.minor_policy(
                    BufferRole::DocumentView,
                    "visual-line-mode",
                    PolicyOrigin::Distributed
                ),
                Some(Admission::Permit)
            )
        });
        // inherit takes the user's rule back: the distributed one decides.
        assert!(
            run(
                "(set-minor-mode-policy 'visual-line-mode 'inherit :role 'document-view)",
                &env,
                &ctx
            )
            .is_truthy()
        );
        assert_eq!(user_rule(&ctx, "document-view", "visual-line-mode"), None);
        assert!(allowed("'visual-line-mode \"*page*\"", &env, &ctx));
        // A string names the mode and the policy as well as a symbol.
        run(
            r#"(set-minor-mode-policy "hl-line-mode" "allow" :role 'view)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            user_rule(&ctx, "view", "hl-line-mode"),
            Some(Admission::Permit)
        );
    }

    #[test]
    fn minor_mode_allowed_p_asks_of_a_buffer_or_of_a_role() {
        let (ctx, env) = booted();
        run(r#"(buffer-create "a-view" :role 'view)"#, &env, &ctx);
        // The current buffer by default, *scratch*, which is edited.
        assert!(allowed("'hl-line-mode", &env, &ctx));
        assert!(allowed("'hl-line-mode nil", &env, &ctx));
        assert!(!allowed("'hl-line-mode \"a-view\"", &env, &ctx));
        assert!(allowed("'hl-line-mode :role 'editing", &env, &ctx));
        assert!(!allowed("'hl-line-mode :role 'view", &env, &ctx));
        assert!(allowed("'show-paren-mode :role 'prompt", &env, &ctx));
        assert!(allowed(
            "'visual-line-mode :role 'document-view",
            &env,
            &ctx
        ));
        assert!(!allowed("'visual-line-mode :role 'view", &env, &ctx));
        assert!(allowed("'read-only-mode :role 'completion", &env, &ctx));
        assert!(!allowed("'hl-line-mode \"no such buffer\"", &env, &ctx));
        // It says what minor-mode-refusal and minor-mode-on do.
        for buffer in ["*scratch*", "a-view"] {
            let refusal = run(
                &format!("(minor-mode-refusal 'hl-line-mode {buffer:?})"),
                &env,
                &ctx,
            );
            assert_eq!(
                allowed(&format!("'hl-line-mode {buffer:?}"), &env, &ctx),
                refusal.is_nil(),
                "{buffer}"
            );
        }
        for (src, says) in [
            (
                "(minor-mode-allowed-p 'hl-line-mode \"a-view\" :role 'view)",
                "not both",
            ),
            ("(minor-mode-allowed-p 'no-such-mode)", "No such minor mode"),
            (
                "(minor-mode-allowed-p 'hl-line-mode :role 'shelf)",
                "no buffer role",
            ),
            (
                "(minor-mode-allowed-p 'hl-line-mode :role 'view :role 'view)",
                "given twice",
            ),
        ] {
            let why = refused(src, &env, &ctx);
            assert!(why.contains(says), "{src}: {why}");
        }
    }

    /// The lines of the example in the `modes' page, from its `;; init.lisp'
    /// to the blank line after the command.
    fn log_view_example() -> String {
        let page = std::fs::read_to_string(man_directory().join("modes.txt")).expect("modes page");
        let start = page.find("        ;; init.lisp\n").expect("the example");
        let body = &page[start..];
        let end = body.find("\n\n    (minor-mode-allowed-p").expect("its end");
        body[..end].to_string()
    }

    #[test]
    fn the_log_view_example_of_the_manual_works_as_written() {
        let (ctx, env) = booted();
        let example = log_view_example();
        assert!(
            example.contains("define-buffer-role 'log-view"),
            "{example}"
        );
        run(&example, &env, &ctx);
        // Loading the configuration twice is no error.
        run(&example, &env, &ctx);
        m_x(&ctx, &env, "open-log");
        assert_eq!(ctx.get_current_buffer_name(), "*log*");
        assert_eq!(role_of("*log*", &env, &ctx), "log-view");
        assert!(on("hl-line-mode", "*log*", &env, &ctx));
        assert!(allowed("'hl-line-mode :role 'log-view", &env, &ctx));
        assert!(!allowed("'hl-line-mode :role 'view", &env, &ctx));
        // A view of the same major mode does not get it.
        run(
            r#"(buffer-create "other-log" :mode 'log-mode :role 'view)"#,
            &env,
            &ctx,
        );
        assert!(!on("hl-line-mode", "other-log", &env, &ctx));
        // A rule given to view later reaches log-view where it has none.
        run(
            "(set-minor-mode-policy 'visual-line-mode 'allow :role 'view)",
            &env,
            &ctx,
        );
        assert!(allowed("'visual-line-mode \"*log*\"", &env, &ctx));
        // And taking the permission back turns the mode off at once.
        run(
            "(set-minor-mode-policy 'hl-line-mode 'deny :role 'log-view)",
            &env,
            &ctx,
        );
        assert!(!on("hl-line-mode", "*log*", &env, &ctx));
    }

    // ----------------------------------------------------------------
    // Derived roles
    // ----------------------------------------------------------------

    #[test]
    fn the_nearest_role_decides_whoever_wrote_the_rule() {
        let (ctx, env) = booted();
        run(
            r#"(define-buffer-role 'manual-view :parent 'document-view)
               (buffer-create "*manual*" :role 'manual-view)
               (buffer-create "*page*" :role 'document-view)
               (global-visual-line-mode 1)"#,
            &env,
            &ctx,
        );
        // Two levels up, the distributed permit of document-view reaches it.
        assert!(on("visual-line-mode", "*manual*", &env, &ctx));
        // A rule of its own comes first, and stays its own.
        run(
            "(set-minor-mode-policy 'visual-line-mode 'deny :role 'manual-view)",
            &env,
            &ctx,
        );
        assert!(!on("visual-line-mode", "*manual*", &env, &ctx));
        assert!(on("visual-line-mode", "*page*", &env, &ctx));
        // The user's rule on view reaches both, where they have none.
        run(
            "(set-minor-mode-policy 'hl-line-mode 'allow :role 'view)",
            &env,
            &ctx,
        );
        assert!(allowed("'hl-line-mode \"*manual*\"", &env, &ctx));
        assert!(allowed("'hl-line-mode \"*page*\"", &env, &ctx));
        // A distributed rule nearer than the user's farther one wins: the
        // hierarchy first, the origin only within one role.
        run(
            "(minor-mode--distributed-policy 'hl-line-mode 'deny 'manual-view)",
            &env,
            &ctx,
        );
        assert!(!allowed("'hl-line-mode \"*manual*\"", &env, &ctx));
        assert!(allowed("'hl-line-mode \"*page*\"", &env, &ctx));
        let why = text(&run(
            r#"(minor-mode-refusal 'hl-line-mode "*manual*")"#,
            &env,
            &ctx,
        ));
        assert!(
            why.contains("the distributed configuration revokes it for the manual-view role"),
            "{why}"
        );
        // Global off still reaches every buffer it was on in.
        run("(global-visual-line-mode 0)", &env, &ctx);
        assert!(!on("visual-line-mode", "*page*", &env, &ctx));
    }

    #[test]
    fn a_role_derived_from_editing_is_open_and_one_from_prompt_reads_answers() {
        for floating in [false, true] {
            let (ctx, env) = booted_with_prompt(floating);
            run(
                r#"(define-buffer-role 'notes :parent 'editing)
                   (define-buffer-role 'quick-prompt :parent 'prompt)
                   (buffer-create "notes" :role 'notes)"#,
                &env,
                &ctx,
            );
            for minor in ["hl-line-mode", "electric-pair-mode", "visual-line-mode"] {
                assert!(
                    allowed(&format!("'{minor} \"notes\""), &env, &ctx),
                    "{minor}"
                );
            }
            run(
                r#"(setq answer nil)
                   (minibuffer-read "Q:" (lambda (text) (setq answer text)) :role 'quick-prompt)"#,
                &env,
                &ctx,
            );
            assert_eq!(
                role_of(PROMPT, &env, &ctx),
                "quick-prompt",
                "floating {floating}"
            );
            // What prompt admits, and nothing else.
            assert!(
                on("show-paren-mode", PROMPT, &env, &ctx),
                "floating {floating}"
            );
            assert!(!allowed("'hl-line-mode", &env, &ctx));
            type_text(&ctx, &env, "yes");
            press(&ctx, &env, KeyCode::Enter);
            assert_eq!(text(&run("answer", &env, &ctx)), "yes");
            // A role that is no prompt is refused before a prompt opens.
            let why = refused(r#"(minibuffer-read "Q:" 'ignore :role 'notes)"#, &env, &ctx);
            assert!(why.contains("prompt or one derived from it"), "{why}");
            assert!(!ctx.has_buffer(PROMPT));
        }
    }

    // ----------------------------------------------------------------
    // A revocation of a mode that is on
    // ----------------------------------------------------------------

    #[test]
    fn a_deny_turns_off_a_mode_that_is_on_with_its_hook_and_an_allow_turns_nothing_on() {
        let (ctx, env) = booted();
        run(
            r#"(define-buffer-role 'log-view :parent 'view)
               (set-minor-mode-policy 'hl-line-mode 'allow :role 'log-view)
               (buffer-create "*log*" :role 'log-view)
               (buffer-create "notes")
               (setq hl-runs 0)
               (defun count-hl (&rest ignored) (setq hl-runs (+ hl-runs 1)))
               (add-hook 'hl-line-mode "minor-mode-hook" 'count-hl)
               (with-current-buffer "*log*" (lambda () (hl-line-mode 1) (show-paren-mode 1)))
               (with-current-buffer "notes" (lambda () (hl-line-mode 1)))"#,
            &env,
            &ctx,
        );
        assert!(on("hl-line-mode", "*log*", &env, &ctx));
        let before = number(&run("hl-runs", &env, &ctx));
        // A deny on the parent leaves the role with its own permit alone.
        run(
            "(set-minor-mode-policy 'hl-line-mode 'deny :role 'view)",
            &env,
            &ctx,
        );
        assert!(on("hl-line-mode", "*log*", &env, &ctx));
        assert_eq!(number(&run("hl-runs", &env, &ctx)), before);
        // Its own taken back, the parent's deny holds: off, by the hook.
        run(
            "(set-minor-mode-policy 'hl-line-mode 'inherit :role 'log-view)",
            &env,
            &ctx,
        );
        assert!(!on("hl-line-mode", "*log*", &env, &ctx));
        assert_eq!(number(&run("hl-runs", &env, &ctx)), before + 1.0);
        assert!(on("hl-line-mode", "notes", &env, &ctx), "editing untouched");
        // An allow counts from the next request, and turns nothing on.
        run(
            "(set-minor-mode-policy 'hl-line-mode 'allow :role 'log-view)",
            &env,
            &ctx,
        );
        assert!(!on("hl-line-mode", "*log*", &env, &ctx));
        run(
            r#"(with-current-buffer "*log*" (lambda () (hl-line-mode 1)))"#,
            &env,
            &ctx,
        );
        assert!(on("hl-line-mode", "*log*", &env, &ctx));
        // A deny on editing reaches every editing buffer, by the same hook.
        let before = number(&run("hl-runs", &env, &ctx));
        run(
            "(set-minor-mode-policy 'hl-line-mode 'deny :role 'editing)",
            &env,
            &ctx,
        );
        assert!(!on("hl-line-mode", "notes", &env, &ctx));
        assert!(on("hl-line-mode", "*log*", &env, &ctx));
        assert_eq!(number(&run("hl-runs", &env, &ctx)), before + 1.0);
    }

    // ----------------------------------------------------------------
    // M-x by keys, with wrapping on everywhere
    // ----------------------------------------------------------------

    #[test]
    fn m_x_with_wrapping_everywhere_recalls_with_arrows_and_c_p_c_n_split_or_floating() {
        for floating in [false, true] {
            let (ctx, env) = booted_with_prompt(floating);
            wrap_everywhere(&env, &ctx);
            m_x(&ctx, &env, "beginning-of-buffer");
            m_x(&ctx, &env, "end-of-buffer");

            meta_x(&ctx, &env);
            assert_eq!(role_of(PROMPT, &env, &ctx), "prompt", "floating {floating}");
            assert!(
                on("visual-line-mode", PROMPT, &env, &ctx),
                "floating {floating}"
            );
            assert!(
                on("show-paren-mode", PROMPT, &env, &ctx),
                "floating {floating}"
            );
            for (back, forward, is_ctrl) in [
                (KeyCode::Up, KeyCode::Down, false),
                (KeyCode::Char('p'), KeyCode::Char('n'), true),
            ] {
                let go = |code: &KeyCode| {
                    ctx.handle_key_event(key(code.clone(), is_ctrl, false), &env);
                    prompt_text(&ctx)
                };
                assert_eq!(go(&back), "end-of-buffer", "floating {floating}, {back:?}");
                assert_eq!(
                    go(&back),
                    "beginning-of-buffer",
                    "floating {floating}, {back:?}"
                );
                assert_eq!(
                    go(&forward),
                    "end-of-buffer",
                    "floating {floating}, {forward:?}"
                );
                assert_eq!(go(&forward), "", "floating {floating}, {forward:?}");
            }
            // What is recalled runs.
            press(&ctx, &env, KeyCode::Up);
            press(&ctx, &env, KeyCode::Up);
            press(&ctx, &env, KeyCode::Enter);
            assert!(!ctx.has_buffer(PROMPT));
        }
    }

    #[test]
    fn with_wrapping_everywhere_the_open_list_chooses_and_closing_it_gives_the_history_back() {
        for floating in [false, true] {
            for close in [KeyCode::Esc, KeyCode::Char('g')] {
                let (ctx, env) = booted_with_prompt(floating);
                wrap_everywhere(&env, &ctx);
                m_x(&ctx, &env, "end-of-buffer");
                meta_x(&ctx, &env);
                type_text(&ctx, &env, "buffer");
                press(&ctx, &env, KeyCode::Tab);
                assert_eq!(role_of(COMPLETIONS, &env, &ctx), "completion");
                assert!(on("visual-line-mode", COMPLETIONS, &env, &ctx));
                let case = format!("floating {floating}, closed by {close:?}");

                let first = selected(&env, &ctx);
                for (next, previous, is_ctrl) in [
                    (KeyCode::Down, KeyCode::Up, false),
                    (KeyCode::Char('n'), KeyCode::Char('p'), true),
                ] {
                    ctx.handle_key_event(key(next.clone(), is_ctrl, false), &env);
                    let second = selected(&env, &ctx);
                    assert_ne!(second, first, "{case}: {next:?} moves the selection");
                    assert_eq!(prompt_text(&ctx), "buffer", "{case}: and recalls nothing");
                    ctx.handle_key_event(key(previous.clone(), is_ctrl, false), &env);
                    assert_eq!(selected(&env, &ctx), first, "{case}: {previous:?}");
                }

                ctx.handle_key_event(key(close.clone(), close == KeyCode::Char('g'), false), &env);
                assert!(
                    !ctx.has_buffer(COMPLETIONS) || ctx.window_count() == 1,
                    "{case}"
                );
                assert!(ctx.has_buffer(PROMPT), "{case}: the prompt stays");
                press(&ctx, &env, KeyCode::Up);
                assert_eq!(prompt_text(&ctx), "end-of-buffer", "{case}: Up recalls");
                ctrl(&ctx, &env, 'n');
                assert_eq!(prompt_text(&ctx), "buffer", "{case}: C-n comes back");
                ctrl(&ctx, &env, 'p');
                assert_eq!(prompt_text(&ctx), "end-of-buffer", "{case}: C-p recalls");
            }
        }
    }

    // ----------------------------------------------------------------
    // A reader written before roles, and its history
    // ----------------------------------------------------------------

    #[test]
    fn a_reader_written_before_roles_keeps_its_history_and_the_keys_recall_it() {
        let (ctx, env) = booted();
        run(
            r#"(setq reader-calls nil)
               (defun my-reader (prompt on-confirm on-change on-cancel &optional mode key)
                 (setq reader-calls (cons (list prompt mode key) reader-calls))
                 (default-minibuffer-prompt prompt on-confirm on-change on-cancel mode key))
               (setq *minibuffer-read-function* 'my-reader)"#,
            &env,
            &ctx,
        );
        wrap_everywhere(&env, &ctx);
        m_x(&ctx, &env, "beginning-of-buffer");
        m_x(&ctx, &env, "end-of-buffer");
        assert_eq!(
            number(&run("(length reader-calls)", &env, &ctx)),
            2.0,
            "M-x went through the reader"
        );
        meta_x(&ctx, &env);
        assert_eq!(role_of(PROMPT, &env, &ctx), "prompt");
        assert!(on("visual-line-mode", PROMPT, &env, &ctx));
        press(&ctx, &env, KeyCode::Up);
        assert_eq!(prompt_text(&ctx), "end-of-buffer");
        ctrl(&ctx, &env, 'p');
        assert_eq!(prompt_text(&ctx), "beginning-of-buffer");
        press(&ctx, &env, KeyCode::Down);
        assert_eq!(prompt_text(&ctx), "end-of-buffer");
        press(&ctx, &env, KeyCode::Esc);

        // A history of its own, by KEY, through the reader.
        for answer in ["first", "second"] {
            run(
                r#"(minibuffer-read "Q:" 'ignore :key 'my-questions)"#,
                &env,
                &ctx,
            );
            type_text(&ctx, &env, answer);
            press(&ctx, &env, KeyCode::Enter);
        }
        assert_eq!(
            text(&run("(prin1-to-string (car reader-calls))", &env, &ctx)),
            r#"("Q:" nil my-questions)"#
        );
        assert_eq!(
            text(&run(
                "(prin1-to-string (minibuffer-history 'my-questions))",
                &env,
                &ctx
            )),
            r#"("second" "first")"#
        );
        run(
            r#"(minibuffer-read "Q:" 'ignore :key 'my-questions)"#,
            &env,
            &ctx,
        );
        ctrl(&ctx, &env, 'p');
        assert_eq!(prompt_text(&ctx), "second");
        press(&ctx, &env, KeyCode::Up);
        assert_eq!(prompt_text(&ctx), "first");
        press(&ctx, &env, KeyCode::Esc);
    }

    // ----------------------------------------------------------------
    // Incremental search
    // ----------------------------------------------------------------

    #[test]
    fn an_incremental_search_in_a_prompt_that_wraps_still_searches_and_has_no_vim() {
        for floating in [false, true] {
            let (ctx, env) = booted_with_prompt(floating);
            run(
                r#"(insert "alpha beta gamma beta")
                   (goto-char 0)
                   (global-vim-mode t)"#,
                &env,
                &ctx,
            );
            wrap_everywhere(&env, &ctx);
            // Vim's keys are in *scratch*, so C-s is the editor's.
            ctrl(&ctx, &env, 's');
            let case = format!("floating {floating}");
            assert_eq!(role_of(PROMPT, &env, &ctx), "prompt", "{case}");
            assert_eq!(
                ctx.with_buffer(PROMPT, |buf| buf.current_mode.clone())
                    .unwrap(),
                "isearch-mode",
                "{case}"
            );
            assert!(on("visual-line-mode", PROMPT, &env, &ctx), "{case}");
            assert!(
                run(r#"(buffer-get 'vim-state "*Minibuffer*")"#, &env, &ctx).is_nil(),
                "{case}"
            );
            type_text(&ctx, &env, "beta");
            assert_eq!(prompt_text(&ctx), "beta", "{case}: typed, not Vim commands");
            let point = || {
                ctx.with_buffer("*scratch*", |buf| buf.text.cursor_pos())
                    .unwrap()
            };
            assert_eq!(point(), (0, 10), "{case}");
            ctrl(&ctx, &env, 's');
            assert_eq!(point(), (0, 21), "{case}: the next one");
            press(&ctx, &env, KeyCode::Enter);
            assert!(!ctx.has_buffer(PROMPT), "{case}");
            assert_eq!(point(), (0, 21), "{case}");
        }
    }

    // ----------------------------------------------------------------
    // Vim
    // ----------------------------------------------------------------

    fn vim_state(buffer: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text(&run(
            &format!("(buffer-get 'vim-state {buffer:?})"),
            env,
            ctx,
        ))
    }

    #[test]
    fn vim_comes_on_by_itself_only_where_something_is_edited() {
        let (ctx, env) = booted();
        run(
            r#"(define-buffer-role 'notes :parent 'editing)
               (define-buffer-role 'log-view :parent 'view)
               (global-vim-mode t)
               (buffer-create "notes" :role 'notes)
               (buffer-create "plain" )
               (buffer-create "listing" :role 'view)
               (buffer-create "log" :role 'log-view)
               (buffer-create "page" :role 'document-view)"#,
            &env,
            &ctx,
        );
        for (buffer, state) in [
            ("*scratch*", "normal"),
            ("notes", "normal"),
            ("plain", "normal"),
            ("listing", "nil"),
            ("log", "nil"),
            ("page", "nil"),
        ] {
            assert_eq!(vim_state(buffer, &env, &ctx), state, "{buffer}");
        }
        // The user's filters narrow the role, and never widen it.
        run(
            r#"(setq vim-mode-predicate (lambda () (not (string= (current-buffer) "plain"))))
               (setq vim-mode-excluded-modes nil)
               (vim-mode-refresh)"#,
            &env,
            &ctx,
        );
        assert_eq!(vim_state("plain", &env, &ctx), "nil");
        assert_eq!(vim_state("notes", &env, &ctx), "normal");
        assert_eq!(vim_state("listing", &env, &ctx), "nil");
        // By hand, a view can have it.
        run(
            r#"(with-current-buffer "listing" (lambda () (vim-mode 1)))"#,
            &env,
            &ctx,
        );
        assert_eq!(vim_state("listing", &env, &ctx), "normal");
    }

    #[test]
    fn vim_never_puts_its_keymap_in_a_prompt_or_a_list_of_candidates() {
        for floating in [false, true] {
            let (ctx, env) = booted_with_prompt(floating);
            run(
                r#"(define-buffer-role 'quick-prompt :parent 'prompt)
                   (setq vim-mode-excluded-modes nil vim-mode-predicate nil)
                   (global-vim-mode t)"#,
                &env,
                &ctx,
            );
            let case = format!("floating {floating}");
            meta_x(&ctx, &env);
            assert_eq!(vim_state(PROMPT, &env, &ctx), "nil", "{case}");
            // By hand neither, and it says so.
            run("(vim-mode 1)", &env, &ctx);
            assert_eq!(vim_state(PROMPT, &env, &ctx), "nil", "{case}");
            let echo = ctx.snapshot(&env, 80, 24).echo_message;
            assert!(echo.contains("Vim does not go in"), "{case}: {echo}");
            assert!(
                !text(&run("(buffer-get 'local-keymap)", &env, &ctx)).starts_with("vim-"),
                "{case}"
            );
            type_text(&ctx, &env, "buffer");
            assert_eq!(
                prompt_text(&ctx),
                "buffer",
                "{case}: typed, not Vim commands"
            );
            press(&ctx, &env, KeyCode::Tab);
            assert_eq!(vim_state(COMPLETIONS, &env, &ctx), "nil", "{case}");
            run(
                r#"(with-current-buffer "*Completions*" (lambda () (vim-mode 1)))"#,
                &env,
                &ctx,
            );
            assert_eq!(vim_state(COMPLETIONS, &env, &ctx), "nil", "{case}");
            press(&ctx, &env, KeyCode::Esc);
            press(&ctx, &env, KeyCode::Esc);
            assert!(!ctx.has_buffer(PROMPT), "{case}");
            // A prompt of a role derived from prompt, the same.
            run(
                r#"(minibuffer-read "Q:" 'ignore :role 'quick-prompt)"#,
                &env,
                &ctx,
            );
            run("(vim-mode 1)", &env, &ctx);
            assert_eq!(vim_state(PROMPT, &env, &ctx), "nil", "{case}");
            press(&ctx, &env, KeyCode::Esc);
            // And *scratch*, where something is edited, has had it all along.
            assert_eq!(vim_state("*scratch*", &env, &ctx), "normal", "{case}");
        }
    }

    // ----------------------------------------------------------------
    // Moving in a buffer being edited
    // ----------------------------------------------------------------

    fn point(ctx: &Ctx) -> (usize, usize) {
        ctx.with_current_buffer(|buf| buf.text.cursor_pos())
    }

    /// A long first line and a short second, in *scratch*, with point at the
    /// start and a 20-column window drawn once, so that the first line wraps.
    fn long_line(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            r#"(insert "alpha beta gamma delta epsilon zeta eta theta\nnext")
               (goto-char 2)"#,
            env,
            ctx,
        );
        let _ = ctx.snapshot(env, 20, 24);
    }

    #[test]
    fn in_an_editing_buffer_the_ordinary_keys_move_by_rows_where_it_wraps_and_by_lines_where_not() {
        let (ctx, env) = booted();
        run("(global-visual-line-mode 1)", &env, &ctx);
        assert!(on("visual-line-mode", "*scratch*", &env, &ctx));
        long_line(&env, &ctx);
        for (code, is_ctrl) in [(KeyCode::Down, false), (KeyCode::Char('n'), true)] {
            run("(goto-char 2)", &env, &ctx);
            ctx.handle_key_event(key(code.clone(), is_ctrl, false), &env);
            let (line, column) = point(&ctx);
            assert_eq!(line, 0, "{code:?} keeps to the wrapped line");
            assert!(column > 2, "{code:?}: {column}");
        }
        // C-e and End go to the end of the row, not of the line.
        for (code, is_ctrl) in [(KeyCode::End, false), (KeyCode::Char('e'), true)] {
            run("(goto-char 2)", &env, &ctx);
            ctx.handle_key_event(key(code.clone(), is_ctrl, false), &env);
            let (line, column) = point(&ctx);
            assert_eq!(line, 0, "{code:?}");
            assert!(column < 45, "{code:?}: {column}");
        }
        // The logical commands, from Lisp, go by lines.
        run("(goto-char 2) (next-line)", &env, &ctx);
        assert_eq!(point(&ctx), (1, 2));

        // The user keeps wrapping out of what is edited: off at once, and
        // the same keys move by lines.
        run(
            "(set-minor-mode-policy 'visual-line-mode 'deny :role 'editing)",
            &env,
            &ctx,
        );
        assert!(!on("visual-line-mode", "*scratch*", &env, &ctx));
        let _ = ctx.snapshot(&env, 20, 24);
        run("(goto-char 2)", &env, &ctx);
        press(&ctx, &env, KeyCode::Down);
        assert_eq!(point(&ctx), (1, 2));
        run("(goto-char 2)", &env, &ctx);
        ctrl(&ctx, &env, 'e');
        assert_eq!(point(&ctx), (0, 45));
    }

    #[test]
    fn vim_moves_by_lines_in_an_editing_buffer_that_wraps() {
        let (ctx, env) = booted();
        run(
            "(global-visual-line-mode 1) (global-vim-mode t)",
            &env,
            &ctx,
        );
        long_line(&env, &ctx);
        assert_eq!(vim_state("*scratch*", &env, &ctx), "normal");
        press(&ctx, &env, KeyCode::Char('j'));
        assert_eq!(point(&ctx).0, 1, "j goes to the next line");
        press(&ctx, &env, KeyCode::Char('k'));
        assert_eq!(point(&ctx).0, 0, "k back");
    }
}
