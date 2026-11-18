//! Buffer roles and the minor modes each admits: the rules read nearest role
//! first and the user's before the distributed ones, the one check in
//! `minor_mode_on` that a command, a rule and a global mode all go through,
//! the commands that say why, the automatic requests that say nothing, a
//! change of major mode, and what a permit and a revocation do to the buffers
//! already open.
//!
//! Design: `doc/architettura-editor.typ, <ruoli>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::buffer::{Buffer, BufferRole, READ_ONLY_MODE};
    use crate::editor::{EditorState, MinorModeOn, create_global_env};
    use crate::managers::{Admission, DecidedBy, MinorPolicies, PolicyOrigin};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    use Admission::{Permit, Revoke};
    use BufferRole::{Completion, DocumentView, Editing, Prompt, View};
    use PolicyOrigin::{Distributed, User};

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn text(exp: &LispExp<Ctx>) -> String {
        match exp {
            LispExp::String(text) => text.to_string(),
            LispExp::Symbol(name) => name.to_string(),
            other => format!("{other:?}"),
        }
    }

    /// An editor with the commands module, the minor modes `one-mode`, shown
    /// as One, and `two-mode`, the major mode `prose-mode`, and a buffer of
    /// each role but editing, named after it.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
        ] {
            let ast = Parser::new(&format!("(progn {source})")).next().unwrap();
            eval(&ast, env.clone(), &ctx).expect("loading the shipped lisp");
        }
        run(
            r#"(make-minor-mode 'one-mode "One")
               (make-minor-mode 'two-mode)
               (make-mode 'prose-mode)"#,
            &env,
            &ctx,
        );
        for role in [Prompt, Completion, View, DocumentView] {
            made(&ctx, &env, role.name(), role);
        }
        (ctx, env)
    }

    /// Makes the buffer NAME, of ROLE, with its hooks run.
    fn made(ctx: &Ctx, env: &Arc<Env<Ctx>>, name: &str, role: BufferRole) {
        assert_eq!(
            ctx.new_buffer_as(name, None, None, role, env).as_deref(),
            Some(name)
        );
    }

    fn on(minor: &str, buffer: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        run(&format!("(minor-mode-p '{minor} {buffer:?})"), env, ctx).is_truthy()
    }

    /// Runs SRC with BUFFER current.
    fn in_buffer(buffer: &str, src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        run(
            &format!("(with-current-buffer {buffer:?} (lambda (&rest ignored) {src}))"),
            env,
            ctx,
        )
    }

    // ----------------------------------------------------------------
    // The roles and the rules
    // ----------------------------------------------------------------

    #[test]
    fn a_role_is_given_before_the_first_hook_and_kept_through_a_new_major_mode() {
        let (ctx, env) = editor();
        run(
            r#"(setq roles-seen nil)
               (defun note-role () (setq roles-seen (cons (buffer-role) roles-seen)))
               (add-hook nil "buffer-created-hook" 'note-role)"#,
            &env,
            &ctx,
        );
        made(&ctx, &env, "shown", View);
        assert_eq!(text(&run("(car roles-seen)", &env, &ctx)), "view");
        in_buffer("shown", "(set-major-mode 'prose-mode)", &env, &ctx);
        assert_eq!(text(&run(r#"(buffer-role "shown")"#, &env, &ctx)), "view");
        assert_eq!(text(&run("(buffer-role)", &env, &ctx)), "editing");
        assert_eq!(
            text(&run(r#"(buffer-role "document-view")"#, &env, &ctx)),
            "document-view"
        );
        assert!(run(r#"(buffer-role "no such buffer")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn editing_is_open_and_every_other_role_admits_only_what_is_permitted() {
        let policies = MinorPolicies::default();
        assert!(policies.decide(Editing, "one-mode").admitted);
        for role in [Prompt, Completion, View, DocumentView] {
            let decision = policies.decide(role, "one-mode");
            assert!(!decision.admitted, "{role:?}");
            // A derived role answers as the role it derives from.
            let root = if role == DocumentView { View } else { role };
            assert_eq!(decision.by, DecidedBy::Default { role: root });
        }
        // The core's own minor mode is no feature, and goes everywhere.
        for role in BufferRole::ALL {
            assert!(policies.decide(role, READ_ONLY_MODE).admitted, "{role:?}");
        }
    }

    #[test]
    fn the_nearest_role_with_a_rule_decides() {
        let mut policies = MinorPolicies::default();
        policies.set(View, "one-mode", Distributed, Permit);
        assert!(policies.decide(View, "one-mode").admitted);
        assert_eq!(
            policies.decide(DocumentView, "one-mode").by,
            DecidedBy::Rule {
                role: View,
                origin: Distributed
            },
            "inherited"
        );
        assert!(policies.decide(DocumentView, "one-mode").admitted);
        // Its own rule, even the distributed one, comes before the user's
        // for the role it derives from.
        policies.set(View, "one-mode", User, Permit);
        policies.set(DocumentView, "one-mode", Distributed, Revoke);
        assert!(!policies.decide(DocumentView, "one-mode").admitted);
        assert!(policies.decide(View, "one-mode").admitted);
        // A rule says nothing of another minor mode, or of a role that is not
        // its own or derived from it.
        assert!(!policies.decide(View, "two-mode").admitted);
        assert!(!policies.decide(Prompt, "one-mode").admitted);
        policies.set(Editing, "two-mode", Distributed, Revoke);
        assert!(!policies.decide(Editing, "two-mode").admitted);
        assert!(policies.decide(Editing, "one-mode").admitted);
    }

    #[test]
    fn for_the_same_role_the_users_rule_wins_whichever_came_first() {
        let mut policies = MinorPolicies::default();
        policies.set(View, "one-mode", User, Permit);
        policies.set(View, "one-mode", Distributed, Revoke);
        assert_eq!(
            policies.decide(View, "one-mode").by,
            DecidedBy::Rule {
                role: View,
                origin: User
            }
        );
        assert!(policies.decide(View, "one-mode").admitted);
        policies.set(Editing, "one-mode", Distributed, Permit);
        policies.set(Editing, "one-mode", User, Revoke);
        assert!(!policies.decide(Editing, "one-mode").admitted);
        // Taken back, the user's leaves the distributed one in force, and
        // with neither the role's default.
        assert!(policies.clear(View, "one-mode", User));
        assert!(!policies.decide(View, "one-mode").admitted);
        assert_eq!(policies.rule(View, "one-mode", Distributed), Some(Revoke));
        assert!(policies.clear(View, "one-mode", Distributed));
        assert!(
            !policies.clear(View, "one-mode", Distributed),
            "gone already"
        );
        assert_eq!(
            policies.decide(View, "one-mode").by,
            DecidedBy::Default { role: View }
        );
        // Saying again what was said changes nothing.
        assert!(policies.set(View, "one-mode", User, Permit));
        assert!(!policies.set(View, "one-mode", User, Permit));
    }

    // ----------------------------------------------------------------
    // The one check
    // ----------------------------------------------------------------

    #[test]
    fn minor_mode_on_tells_a_refusal_from_a_mode_on_already() {
        let (ctx, env) = editor();
        let editing = ctx.get_current_buffer_name();
        assert_eq!(
            ctx.minor_mode_on(&editing, "one-mode", &env),
            MinorModeOn::TurnedOn
        );
        assert_eq!(
            ctx.minor_mode_on(&editing, "one-mode", &env),
            MinorModeOn::AlreadyOn
        );
        let MinorModeOn::Refused(refusal) = ctx.minor_mode_on("view", "one-mode", &env) else {
            panic!("a view admits only what is permitted");
        };
        assert_eq!((refusal.role, refusal.buffer.as_str()), (View, "view"));
        assert_eq!(
            ctx.minor_mode_on("view", "no-mode", &env),
            MinorModeOn::Missing
        );
        assert_eq!(
            ctx.minor_mode_on("nowhere", "one-mode", &env),
            MinorModeOn::Missing
        );
        assert!(ctx.minor_modes_of("view").is_empty());
        // The core's own mode is admitted everywhere.
        assert_eq!(
            ctx.minor_mode_on("prompt", READ_ONLY_MODE, &env),
            MinorModeOn::TurnedOn
        );
    }

    #[test]
    fn the_lisp_minor_mode_on_still_answers_t_or_nil_and_minor_mode_refusal_says_why() {
        let (ctx, env) = editor();
        run(
            r#"(setq hook-ran 0)
               (defun count-hook () (setq hook-ran (+ hook-ran 1)))
               (add-hook 'one-mode "minor-mode-hook" 'count-hook)"#,
            &env,
            &ctx,
        );
        assert!(run(r#"(minor-mode-on 'one-mode "view")"#, &env, &ctx).is_nil());
        assert!(run(r#"(minor-mode-on 'one-mode "completion")"#, &env, &ctx).is_nil());
        assert!(run("(minor-mode-on 'one-mode)", &env, &ctx).is_truthy());
        assert!(run("(minor-mode-on 'one-mode)", &env, &ctx).is_nil());
        assert_eq!(text(&run("hook-ran", &env, &ctx)), "1");
        assert!(run("(minor-mode-refusal 'one-mode)", &env, &ctx).is_nil());
        let why = text(&run(r#"(minor-mode-refusal 'one-mode "view")"#, &env, &ctx));
        assert!(
            why.contains("one-mode") && why.contains("view") && why.contains("permitted"),
            "{why}"
        );
        // Positional, as every other minor mode primitive.
        assert!(
            Parser::new(r#"(minor-mode-refusal 'no-mode "view")"#)
                .next()
                .map(|ast| eval(&ast, env.clone(), &ctx).is_err())
                .unwrap()
        );
    }

    #[test]
    fn a_refusal_names_the_rule_and_whose_it_is() {
        let (ctx, env) = editor();
        ctx.set_minor_policy(View, "one-mode", User, Some(Revoke), &env);
        let why = text(&run(r#"(minor-mode-refusal 'one-mode "view")"#, &env, &ctx));
        assert!(
            why.contains("user configuration") && why.contains("view role"),
            "{why}"
        );
        let why = text(&run(
            r#"(minor-mode-refusal 'one-mode "document-view")"#,
            &env,
            &ctx,
        ));
        assert!(why.contains("derives from"), "{why}");
    }

    #[test]
    fn a_command_says_why_it_is_refused_and_skips_its_body() {
        let (ctx, env) = editor();
        run(
            r#"(setq body-ran 0)
               (define-minor-mode shout-mode "Shout" "Shout in this buffer."
                 (setq body-ran (+ body-ran 1)))"#,
            &env,
            &ctx,
        );
        in_buffer("view", "(shout-mode)", &env, &ctx);
        assert!(!on("shout-mode", "view", &env, &ctx));
        assert_eq!(text(&run("body-ran", &env, &ctx)), "0");
        let echo = ctx.get_echo_message();
        assert!(
            echo.contains("shout-mode is not admitted in view"),
            "{echo}"
        );
        // Off is never refused, and runs the body as before.
        in_buffer("view", "(shout-mode -1)", &env, &ctx);
        assert_eq!(text(&run("body-ran", &env, &ctx)), "1");
        // Where the role admits it, as before.
        run("(shout-mode 1)", &env, &ctx);
        assert!(run("(minor-mode-p 'shout-mode)", &env, &ctx).is_truthy());
        assert_eq!(ctx.get_echo_message(), "shout-mode on");
        assert_eq!(text(&run("body-ran", &env, &ctx)), "2");
    }

    // ----------------------------------------------------------------
    // The automatic requests
    // ----------------------------------------------------------------

    #[test]
    fn a_rule_passes_over_a_role_that_does_not_admit_it_without_a_word() {
        let (ctx, env) = editor();
        run("(minor-mode-enable 'one-mode)", &env, &ctx);
        ctx.set_echo_message("");
        made(&ctx, &env, "new view", View);
        made(&ctx, &env, "new prompt", Prompt);
        made(&ctx, &env, "new editing", Editing);
        assert!(!on("one-mode", "new view", &env, &ctx));
        assert!(!on("one-mode", "new prompt", &env, &ctx));
        assert!(on("one-mode", "new editing", &env, &ctx));
        // And again at a new major mode.
        in_buffer("view", "(set-major-mode 'prose-mode)", &env, &ctx);
        in_buffer("new editing", "(set-major-mode 'prose-mode)", &env, &ctx);
        assert!(!on("one-mode", "view", &env, &ctx));
        assert!(on("one-mode", "new editing", &env, &ctx));
        assert_eq!(ctx.get_echo_message(), "");
    }

    /// The editor, with `shout-mode`, a global mode for it that asks TURN-ON
    /// and counts where it was asked, and one that asks no TURN-ON.
    fn editor_with_global_modes() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor();
        run(
            r#"(setq turn-on-asked nil)
               (define-minor-mode shout-mode "Shout" "Shout in this buffer.")
               (defun shout-anywhere ()
                 (setq turn-on-asked (cons (current-buffer) turn-on-asked))
                 (shout-mode 1))
               (define-globalized-minor-mode global-shout-mode shout-mode
                 shout-anywhere "Shout in every buffer.")
               (define-globalized-minor-mode global-plain-mode two-mode nil
                 "Two in every buffer.")"#,
            &env,
            &ctx,
        );
        (ctx, env)
    }

    fn asked_in(buffer: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        run(&format!("(member {buffer:?} turn-on-asked)"), env, ctx).is_truthy()
    }

    #[test]
    fn a_global_mode_does_not_call_turn_on_where_the_role_refuses_in_the_buffers_open() {
        let (ctx, env) = editor_with_global_modes();
        run("(global-shout-mode 1) (global-plain-mode 1)", &env, &ctx);
        assert_eq!(ctx.get_echo_message(), "global-plain-mode on");
        for role in [Prompt, Completion, View, DocumentView] {
            assert!(!asked_in(role.name(), &env, &ctx), "{role:?}");
            assert!(!on("shout-mode", role.name(), &env, &ctx), "{role:?}");
            assert!(!on("two-mode", role.name(), &env, &ctx), "{role:?}");
        }
        assert!(asked_in("*scratch*", &env, &ctx));
        assert!(on("shout-mode", "*scratch*", &env, &ctx));
        assert!(on("two-mode", "*scratch*", &env, &ctx));
    }

    #[test]
    fn a_global_mode_does_not_call_turn_on_where_the_role_refuses_in_the_buffers_to_come() {
        let (ctx, env) = editor_with_global_modes();
        run("(global-shout-mode 1) (global-plain-mode 1)", &env, &ctx);
        ctx.set_echo_message("");
        run("(setq turn-on-asked nil)", &env, &ctx);
        made(&ctx, &env, "later view", View);
        made(&ctx, &env, "later file", Editing);
        assert!(!asked_in("later view", &env, &ctx));
        assert!(!on("shout-mode", "later view", &env, &ctx));
        assert!(!on("two-mode", "later view", &env, &ctx));
        assert!(asked_in("later file", &env, &ctx));
        assert!(on("shout-mode", "later file", &env, &ctx));
        // A new major mode asks again, and the role answers the same.
        run("(setq turn-on-asked nil)", &env, &ctx);
        in_buffer("later view", "(set-major-mode 'prose-mode)", &env, &ctx);
        in_buffer("later file", "(set-major-mode 'prose-mode)", &env, &ctx);
        assert!(!asked_in("later view", &env, &ctx));
        assert!(!on("shout-mode", "later view", &env, &ctx));
        assert!(asked_in("later file", &env, &ctx));
        assert!(on("shout-mode", "later file", &env, &ctx));
        assert!(on("two-mode", "later file", &env, &ctx));
        // TURN-ON's own command spoke where it ran, and nowhere else.
        assert_eq!(ctx.get_echo_message(), "shout-mode on");
    }

    #[test]
    fn a_global_mode_turned_off_still_turns_its_mode_off_everywhere() {
        let (ctx, env) = editor_with_global_modes();
        ctx.set_minor_policy(View, "shout-mode", User, Some(Permit), &env);
        run("(global-shout-mode 1)", &env, &ctx);
        assert!(on("shout-mode", "view", &env, &ctx));
        assert!(on("shout-mode", "document-view", &env, &ctx), "inherited");
        run("(global-shout-mode -1)", &env, &ctx);
        for name in ["view", "document-view", "*scratch*"] {
            assert!(!on("shout-mode", name, &env, &ctx), "{name}");
        }
    }

    // ----------------------------------------------------------------
    // Permits and revocations
    // ----------------------------------------------------------------

    #[test]
    fn a_permit_admits_the_mode_from_the_next_request_on() {
        let (ctx, env) = editor();
        run("(minor-mode-enable 'one-mode)", &env, &ctx);
        assert!(!on("one-mode", "view", &env, &ctx));
        assert!(ctx.set_minor_policy(View, "one-mode", Distributed, Some(Permit), &env));
        // Nothing is turned on by the permit itself.
        assert!(!on("one-mode", "view", &env, &ctx));
        assert!(!on("one-mode", "document-view", &env, &ctx));
        // The next request is admitted: a new major mode, or a call.
        in_buffer("view", "(set-major-mode 'prose-mode)", &env, &ctx);
        assert!(on("one-mode", "view", &env, &ctx));
        assert!(run(r#"(minor-mode-on 'one-mode "document-view")"#, &env, &ctx).is_truthy());
        assert!(!run(r#"(minor-mode-on 'one-mode "prompt")"#, &env, &ctx).is_truthy());
    }

    #[test]
    fn a_revocation_turns_off_what_it_no_longer_admits_running_the_hook_without_a_lock() {
        let (ctx, env) = editor();
        // The hook reads the buffer it runs in, and the registry: under a lock
        // of either, it would never return.
        run(
            r#"(setq left nil)
               (defun note-leaving ()
                 (if (not (minor-mode-p 'one-mode))
                     (setq left (cons (list (current-buffer) (buffer-role)
                                            (minor-mode-refusal 'one-mode)
                                            (minor-modes))
                                      left))))
               (add-hook 'one-mode "minor-mode-hook" 'note-leaving)"#,
            &env,
            &ctx,
        );
        ctx.set_minor_policy(View, "one-mode", Distributed, Some(Permit), &env);
        for name in ["view", "document-view", "*scratch*"] {
            assert!(run(&format!("(minor-mode-on 'one-mode {name:?})"), &env, &ctx).is_truthy());
        }
        // The user's permit for the derived role keeps it there.
        ctx.set_minor_policy(DocumentView, "one-mode", User, Some(Permit), &env);
        assert!(ctx.set_minor_policy(View, "one-mode", Distributed, None, &env));
        assert!(!on("one-mode", "view", &env, &ctx));
        assert!(on("one-mode", "document-view", &env, &ctx));
        assert!(on("one-mode", "*scratch*", &env, &ctx));
        let left = run("left", &env, &ctx);
        assert_eq!(left.iter().count(), 1, "{left:?}");
        let entry: Vec<_> = left.iter().next().unwrap().iter().collect();
        assert_eq!(text(&entry[0]), "view");
        assert_eq!(text(&entry[1]), "view");
        assert!(text(&entry[2]).contains("not admitted"));
        assert!(entry[3].is_nil());
        assert_eq!(
            ctx.modes(|modes| modes.minor_policy(View, "one-mode", Distributed)),
            None
        );
        // Taken back, the user's permit leaves the derived role closed too.
        ctx.set_minor_policy(DocumentView, "one-mode", User, None, &env);
        assert!(!on("one-mode", "document-view", &env, &ctx));
        assert_eq!(run("left", &env, &ctx).iter().count(), 2);
        // A revocation for editing reaches editing alone.
        ctx.set_minor_policy(Editing, "one-mode", User, Some(Revoke), &env);
        assert!(!on("one-mode", "*scratch*", &env, &ctx));
        // What changes nothing turns nothing off.
        assert!(!ctx.set_minor_policy(Editing, "one-mode", User, Some(Revoke), &env));
    }

    #[test]
    fn a_buffer_keeps_its_role_through_new_text() {
        let mut buf = Buffer::<GapBuffer>::new("b").with_role(DocumentView);
        buf.adopt_text("again");
        assert_eq!(buf.role(), DocumentView);
        assert_eq!(Buffer::<GapBuffer>::new("plain").role(), Editing);
    }
}
