//! What the editor can be asked about itself, and why the answers are the
//! editor's own rather than a second opinion.
//!
//! # What is worth testing here
//!
//! Every one of these commands is believed. A `describe-key` that reports the
//! global binding while the mode's is the one that runs does not look wrong --
//! it looks authoritative, and the person reading it goes and rebinds the
//! wrong key. So the tests below are mostly about *precedence*: that a mode
//! hides the global map, that a transient map hides both, that a sequence
//! appears once and under the winner.
//!
//! The capture that `describe-key` is built on has a second failure of its
//! own: it must read the whole sequence -- `C-x C-f`, not `C-x` -- and it must
//! not run, type, or insert anything while doing it. An unbound printable key
//! self-inserts on the normal path, and a capture that let that through would
//! answer the question and edit the buffer.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::commands::ArgSpec;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn try_run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        try_run(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// A list of strings, as a Vec.
    fn strings(exp: &LispExp<Ctx>) -> Vec<String> {
        exp.iter()
            .map(|item| match item {
                LispExp::String(text) => text.to_string(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    /// The (KEYS TARGET SOURCE) triple, flattened for comparison.
    fn triple(exp: &LispExp<Ctx>) -> (String, String, String) {
        let parts: Vec<LispExp<Ctx>> = exp.iter().collect();
        assert_eq!(parts.len(), 3, "a binding should be a triple: {exp:?}");
        let text = |item: &LispExp<Ctx>| match item {
            LispExp::String(text) => text.to_string(),
            other => format!("{other:?}"),
        };
        (text(&parts[0]), text(&parts[1]), text(&parts[2]))
    }

    fn press(ch: char, ctrl: bool, ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers {
                    ctrl,
                    ..KeyModifiers::default()
                },
            },
            env,
        );
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|buf| buf.text.to_string())
    }

    // ----------------------------------------------------------------
    // What a key does
    // ----------------------------------------------------------------

    #[test]
    fn a_global_binding_is_reported_with_where_it_came_from() {
        let (ctx, env) = editor();
        run(r#"(define-key nil "C-x C-f" 'find-file)"#, &env, &ctx);
        let (keys, target, source) = triple(&run(r#"(key-binding "C-x C-f")"#, &env, &ctx));
        assert_eq!(keys, "C-x C-f");
        assert!(target.contains("find-file"), "target was {target}");
        assert_eq!(source, "global");
    }

    #[test]
    fn a_mode_binding_hides_the_global_one_and_says_so() {
        // The failure this is here for: reporting the global binding while
        // the mode's is what runs. It reads as authoritative and sends
        // somebody off to rebind a key that was never the problem.
        let (ctx, env) = editor();
        run(
            r#"(define-key nil "C-c x" 'global-answer)
               (make-mode 'help-test-mode)
               (define-key 'help-test-mode "C-c x" 'mode-answer)
               (buffer-create "*probe*" 'help-test-mode)
               (switch-to-buffer "*probe*")"#,
            &env,
            &ctx,
        );
        let (_, target, source) = triple(&run(r#"(key-binding "C-c x")"#, &env, &ctx));
        assert!(target.contains("mode-answer"), "target was {target}");
        assert_eq!(source, "mode");
    }

    #[test]
    fn a_transient_map_hides_both() {
        let (ctx, env) = editor();
        run(
            r#"(define-key nil "y" 'global-answer)
               (set-transient-keymap '(("y" . transient-answer)) "asking" 'refuse)"#,
            &env,
            &ctx,
        );
        let (_, target, source) = triple(&run(r#"(key-binding "y")"#, &env, &ctx));
        assert!(target.contains("transient-answer"), "target was {target}");
        assert_eq!(source, "transient");
    }

    #[test]
    fn a_prefix_runs_nothing_but_leads_somewhere() {
        // The two questions together are how a prefix is told apart from a
        // key bound to nothing -- neither answers it alone.
        let (ctx, env) = editor();
        run(r#"(define-key nil "C-x C-f" 'find-file)"#, &env, &ctx);
        assert_eq!(run(r#"(key-binding "C-x")"#, &env, &ctx), LispExp::nil());
        let onwards = run(r#"(keys-with-prefix "C-x")"#, &env, &ctx);
        assert!(
            strings(&onwards).is_empty() || !onwards.is_nil(),
            "C-x should lead somewhere"
        );
        let found: Vec<(String, String, String)> = onwards.iter().map(|b| triple(&b)).collect();
        assert!(
            found
                .iter()
                .any(|(keys, target, _)| keys == "C-x C-f" && target.contains("find-file")),
            "C-x C-f should be among {found:?}"
        );
    }

    #[test]
    fn a_key_bound_to_nothing_leads_nowhere_either() {
        let (ctx, env) = editor();
        assert_eq!(run(r#"(key-binding "C-M-y")"#, &env, &ctx), LispExp::nil());
        assert_eq!(
            run(r#"(keys-with-prefix "C-M-y")"#, &env, &ctx),
            LispExp::nil()
        );
    }

    #[test]
    fn a_sequence_that_cannot_be_read_is_refused_rather_than_guessed() {
        let (ctx, env) = editor();
        assert_eq!(run(r#"(key-binding "C-x C-")"#, &env, &ctx), LispExp::nil());
    }

    // ----------------------------------------------------------------
    // The whole table
    // ----------------------------------------------------------------

    #[test]
    fn a_rebound_key_is_listed_once_under_the_map_that_wins() {
        let (ctx, env) = editor();
        run(
            r#"(define-key nil "C-c z" 'global-answer)
               (make-mode 'help-table-mode)
               (define-key 'help-table-mode "C-c z" 'mode-answer)
               (buffer-create "*probe*" 'help-table-mode)
               (switch-to-buffer "*probe*")"#,
            &env,
            &ctx,
        );
        let listed: Vec<(String, String, String)> = run("(keymap-bindings)", &env, &ctx)
            .iter()
            .map(|b| triple(&b))
            .filter(|(keys, _, _)| keys == "C-c z")
            .collect();
        assert_eq!(listed.len(), 1, "listed twice: {listed:?}");
        assert_eq!(listed[0].2, "mode");
    }

    #[test]
    fn the_table_and_the_empty_prefix_are_the_same_question() {
        let (ctx, env) = editor();
        let all = run("(keymap-bindings)", &env, &ctx).iter().count();
        let everything = run(r#"(keys-with-prefix "")"#, &env, &ctx).iter().count();
        assert_eq!(all, everything);
        assert!(all > 0, "the editor should come up with bindings");
    }

    // ----------------------------------------------------------------
    // Where a command is bound
    // ----------------------------------------------------------------

    #[test]
    fn where_is_finds_a_command_however_its_binding_was_written() {
        // `define-key` stores a one-element form; the keymaps built in Rust
        // store a bare symbol. Both mean "run this command", so both have to
        // be found -- a `where-is` that knew only one shape would answer
        // "nowhere" for half the editor.
        let (ctx, env) = editor();
        run(
            r#"(define-key nil "C-c 1" 'somewhere)
               (define-key nil "C-c 2" '(somewhere))"#,
            &env,
            &ctx,
        );
        let mut found = strings(&run("(where-is 'somewhere)", &env, &ctx));
        found.sort();
        assert_eq!(found, vec!["C-c 1".to_string(), "C-c 2".to_string()]);
    }

    #[test]
    fn where_is_says_nothing_about_a_command_with_no_binding() {
        let (ctx, env) = editor();
        assert_eq!(
            run("(where-is 'not-bound-anywhere)", &env, &ctx),
            LispExp::nil()
        );
    }

    #[test]
    fn where_is_leaves_out_a_binding_that_supplies_its_own_arguments() {
        // Every printable key is bound to `(self-insert "x")` for its own x.
        // Listing those as ways to run `self-insert` would bury the answer
        // under the alphabet, and none of them is the command a reader could
        // reach with `M-x`.
        let (ctx, env) = editor();
        run(r#"(define-key nil "C-c 3" '(self-insert "q"))"#, &env, &ctx);
        assert_eq!(run("(where-is 'self-insert)", &env, &ctx), LispExp::nil());
    }

    // ----------------------------------------------------------------
    // What a command asks for
    // ----------------------------------------------------------------

    #[test]
    fn a_commands_specs_come_back_as_they_were_written() {
        let (ctx, env) = editor();
        run(
            r#"(register-command 'greet '("sGreet whom: " "nHow many times: "))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            strings(&run(r#"(command-specs "greet")"#, &env, &ctx)),
            vec!["sGreet whom: ".to_string(), "nHow many times: ".to_string()]
        );
    }

    #[test]
    fn a_spec_written_back_out_reads_back_in() {
        // What makes `command-specs` worth showing somebody: the codes can be
        // copied into a `defcommand' of their own.
        for code in [
            "sPrompt: ",
            "nHow many: ",
            "bBuffer: ",
            "fFile: ",
            "p",
            "P",
            "r",
        ] {
            let spec = ArgSpec::parse(code).expect("the test's own codes must parse");
            assert_eq!(spec.code(), code);
            assert_eq!(ArgSpec::parse(&spec.code()), Ok(spec));
        }
    }

    #[test]
    fn a_command_taking_nothing_and_a_name_that_is_no_command_both_read_as_nil() {
        let (ctx, env) = editor();
        run("(register-command 'plain nil)", &env, &ctx);
        assert_eq!(
            run(r#"(command-specs "plain")"#, &env, &ctx),
            LispExp::nil()
        );
        assert_eq!(
            run(r#"(command-specs "no-such")"#, &env, &ctx),
            LispExp::nil()
        );
        // Which is why `commandp` is the question that tells them apart.
        assert!(!run("(commandp 'plain)", &env, &ctx).is_nil());
        assert!(run("(commandp 'no-such)", &env, &ctx).is_nil());
    }

    // ----------------------------------------------------------------
    // Reading the next key sequence
    // ----------------------------------------------------------------

    /// Arm a capture that records its three arguments into variables.
    fn arm(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            r#"(setq captured-keys nil captured-target nil captured-source nil)
               (read-key-sequence
                 (lambda (keys target source)
                   (setq captured-keys keys
                         captured-target target
                         captured-source source)))"#,
            env,
            ctx,
        );
    }

    fn captured(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> (String, String, String) {
        let one = |name: &str| match run(name, env, ctx) {
            LispExp::String(text) => text.to_string(),
            other => format!("{other:?}"),
        };
        (
            one("captured-keys"),
            one("captured-target"),
            one("captured-source"),
        )
    }

    #[test]
    fn the_next_key_is_reported_instead_of_run() {
        let (ctx, env) = editor();
        run(r#"(define-key nil "C-c 9" 'somewhere)"#, &env, &ctx);
        arm(&env, &ctx);
        press('c', true, &ctx, &env);
        press('9', false, &ctx, &env);
        let (keys, target, source) = captured(&env, &ctx);
        assert_eq!(keys, "C-c 9");
        assert!(target.contains("somewhere"), "target was {target}");
        assert_eq!(source, "global");
    }

    #[test]
    fn a_prefix_is_read_to_the_end() {
        // The whole reason capture sits on the resolution path: `C-x` is not
        // an answer, it is half a question, and something reading one key at
        // a time would report the prefix and stop.
        let (ctx, env) = editor();
        run(r#"(define-key nil "C-x C-f" 'find-file)"#, &env, &ctx);
        arm(&env, &ctx);
        press('x', true, &ctx, &env);
        assert_eq!(
            run("captured-keys", &env, &ctx),
            LispExp::nil(),
            "nothing should be reported half-way"
        );
        press('f', true, &ctx, &env);
        let (keys, target, _) = captured(&env, &ctx);
        assert_eq!(keys, "C-x C-f");
        assert!(target.contains("find-file"), "target was {target}");
    }

    #[test]
    fn a_self_inserting_key_is_reported_and_not_typed() {
        // The one thing `C-h k` must never do. On the normal path a printable
        // key runs `(self-insert "q")`; a capture that let that through would
        // answer the question *and* edit the buffer.
        let (ctx, env) = editor();
        let before = text(&ctx);
        arm(&env, &ctx);
        press('q', false, &ctx, &env);
        let (keys, target, source) = captured(&env, &ctx);
        assert_eq!(keys, "q");
        assert!(target.contains("self-insert"), "target was {target}");
        assert_eq!(source, "global");
        assert_eq!(text(&ctx), before, "the capture typed into the buffer");
    }

    #[test]
    fn a_key_bound_to_nothing_is_reported_as_bound_to_nothing() {
        let (ctx, env) = editor();
        arm(&env, &ctx);
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('y'),
                modifiers: KeyModifiers {
                    ctrl: true,
                    alt: true,
                    ..KeyModifiers::default()
                },
            },
            &env,
        );
        let (keys, target, source) = captured(&env, &ctx);
        assert_eq!(keys, "C-M-y");
        assert_eq!(target, "nil");
        assert_eq!(source, "nil");
    }

    #[test]
    fn the_prefix_argument_key_is_reported_as_what_it_is() {
        // `C-u` never reaches a keymap -- the argument reader takes it first
        // -- so a capture that only asked the keymaps would report the one
        // key with an answer as having none.
        let (ctx, env) = editor();
        arm(&env, &ctx);
        press('u', true, &ctx, &env);
        let (keys, _, source) = captured(&env, &ctx);
        assert_eq!(keys, "C-u");
        assert_eq!(source, "prefix-argument");
    }

    #[test]
    fn capture_is_over_after_one_sequence() {
        // A watcher that left the next keystroke captured too would take the
        // editor away from whoever is typing, which is a state with no way
        // out that does not look like a hang.
        let (ctx, env) = editor();
        arm(&env, &ctx);
        press('a', false, &ctx, &env);
        assert_eq!(
            run("*key-capture-function*", &env, &ctx),
            LispExp::nil(),
            "the capture should disarm itself"
        );
        let before = text(&ctx);
        press('b', false, &ctx, &env);
        assert_ne!(text(&ctx), before, "typing should work again");
    }

    #[test]
    fn a_capture_can_be_called_off_without_pressing_anything() {
        let (ctx, env) = editor();
        arm(&env, &ctx);
        run("(setq *key-capture-function* nil)", &env, &ctx);
        let before = text(&ctx);
        press('z', false, &ctx, &env);
        assert_ne!(text(&ctx), before, "typing should work again");
        assert_eq!(run("captured-keys", &env, &ctx), LispExp::nil());
    }

    #[test]
    fn the_reported_binding_is_the_one_that_would_have_run() {
        // The claim the whole mechanism exists to make. The mode's binding
        // wins on the real path, so it has to be what capture reports.
        let (ctx, env) = editor();
        run(
            r#"(define-key nil "C-c 8" 'global-answer)
               (make-mode 'help-capture-mode)
               (define-key 'help-capture-mode "C-c 8" 'mode-answer)
               (buffer-create "*probe*" 'help-capture-mode)
               (switch-to-buffer "*probe*")"#,
            &env,
            &ctx,
        );
        arm(&env, &ctx);
        press('c', true, &ctx, &env);
        press('8', false, &ctx, &env);
        let (_, target, source) = captured(&env, &ctx);
        assert!(target.contains("mode-answer"), "target was {target}");
        assert_eq!(source, "mode");
    }
}
