//! Key sequences: a key that completes a binding, one that starts a longer
//! one, and one that does neither; and how a sequence is written and read.
//!
//! Design: `doc/architettura-editor.typ, <keymap>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers, describe_keys};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            b.text.cursor_move(0, 0);
            b.is_modified = false;
        });
        (ctx, env)
    }

    fn text_of(ctx: &Ctx) -> String {
        ctx.with_buffer("*scratch*", |b| b.text.to_string())
            .expect("*scratch*")
    }

    fn point_1d(ctx: &Ctx) -> usize {
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos_1d())
            .expect("*scratch*")
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode, modifiers: KeyModifiers) {
        ctx.handle_key_event(KeyEvent { code, modifiers }, env);
    }

    fn ctrl() -> KeyModifiers {
        KeyModifiers {
            ctrl: true,
            ..Default::default()
        }
    }

    fn plain() -> KeyModifiers {
        KeyModifiers::default()
    }

    // ---------------- the three-way decision ----------------

    #[test]
    fn a_sequence_runs_only_once_it_is_complete() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(
            "(defun mark-it () (set-mark)) (define-key nil \"C-x m\" 'mark-it)",
            &env,
            &ctx,
        )
        .expect("bind a two-key sequence");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        assert!(
            ctx.with_buffer("*scratch*", |b| b.mark.is_none())
                .expect("*scratch*"),
            "the first key of a sequence must not run anything"
        );

        press(&ctx, &env, KeyCode::Char('m'), plain());
        assert!(
            ctx.with_buffer("*scratch*", |b| b.mark.is_some())
                .expect("*scratch*"),
            "the second key completes it"
        );
    }

    /// The first key of a sequence must not reach the buffer as text, which is
    /// what would happen if it fell through to the self-insert binding.
    #[test]
    fn a_prefix_key_inserts_nothing() {
        let (ctx, env) = editor_with("");
        eval_str(
            "(define-key nil \"C-x m\" 'beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("bind");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        assert_eq!(text_of(&ctx), "", "nothing typed, nothing inserted");
    }

    #[test]
    fn an_unknown_continuation_is_reported_and_the_sequence_dropped() {
        let (ctx, env) = editor_with("");
        eval_str(
            "(define-key nil \"C-x m\" 'beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("bind");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        press(&ctx, &env, KeyCode::Char('z'), ctrl());
        assert!(
            ctx.get_echo_message().contains("is undefined"),
            "the user should be told: {:?}",
            ctx.get_echo_message()
        );

        // And the sequence is gone, so the next key starts afresh rather than
        // being read as a continuation of the abandoned one.
        press(&ctx, &env, KeyCode::Char('a'), plain());
        assert_eq!(text_of(&ctx), "a");
    }

    /// A key that begins a sequence must not stop being a binding in its own
    /// right elsewhere -- `C-x` as a prefix leaves plain `x` alone.
    #[test]
    fn binding_a_prefix_does_not_disturb_other_bindings() {
        let (ctx, env) = editor_with("");
        eval_str(
            "(define-key nil \"C-x m\" 'beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("bind");

        press(&ctx, &env, KeyCode::Char('x'), plain());
        assert_eq!(text_of(&ctx), "x", "plain x still self-inserts");
    }

    /// A half-typed sequence has to be visible, or the editor looks hung.
    ///
    /// It travels in the frame rather than the echo area because it is state,
    /// not a message: a message expires, and a sequence that is still pending
    /// must not stop being shown while the editor is still waiting on it.
    #[test]
    fn a_pending_sequence_is_carried_in_the_frame() {
        let (ctx, env) = editor_with("");
        eval_str(
            "(define-key nil \"C-x m\" 'beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("bind");
        assert_eq!(ctx.snapshot(&env, 80, 24).pending_input, "");

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        assert_eq!(
            ctx.snapshot(&env, 80, 24).pending_input,
            "C-x-",
            "spelt as the binding that completes it is written"
        );

        press(&ctx, &env, KeyCode::Char('m'), plain());
        assert_eq!(
            ctx.snapshot(&env, 80, 24).pending_input,
            "",
            "nothing is pending once the sequence completes"
        );
    }

    // ---------------- the machinery underneath ----------------

    /// A prefix key is not a command. Letting one through the command
    /// machinery would end the undo group being typed into -- silently, and
    /// nowhere near where it looks like a key-handling bug.
    ///
    /// The sequence is *abandoned* on purpose: a sequence that completes runs
    /// a command, and that command ends the group legitimately, which would
    /// hide the difference this is looking for.
    #[test]
    fn an_abandoned_sequence_does_not_break_the_undo_group_being_typed() {
        let (ctx, env) = editor_with("");
        eval_str(
            "(define-key nil \"C-x m\" 'beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("bind");

        for c in "abc".chars() {
            press(&ctx, &env, KeyCode::Char(c), plain());
        }
        press(&ctx, &env, KeyCode::Char('x'), ctrl()); // starts a sequence
        press(&ctx, &env, KeyCode::Char('z'), ctrl()); // abandons it
        for c in "def".chars() {
            press(&ctx, &env, KeyCode::Char(c), plain());
        }
        assert_eq!(text_of(&ctx), "abcdef", "neither key reached the buffer");

        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(
            text_of(&ctx),
            "",
            "the six characters were one run of typing: keys that ran no \
             command must not have split them into two undo steps"
        );
    }

    /// The same rule for the kill ring: keys that run no command must not end
    /// a run of kills and start a second ring entry.
    #[test]
    fn an_abandoned_sequence_does_not_break_a_run_of_kills() {
        let (ctx, env) = editor_with("one\ntwo\nthree\n");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");

        press(&ctx, &env, KeyCode::Char('k'), ctrl());
        press(&ctx, &env, KeyCode::Char('x'), ctrl()); // starts a sequence
        press(&ctx, &env, KeyCode::Char('z'), ctrl()); // abandons it
        press(&ctx, &env, KeyCode::Char('k'), ctrl());

        assert_eq!(
            text_of(&ctx),
            "two\nthree\n",
            "both kills should have happened"
        );
        assert_eq!(
            ctx.kill_ring_len(),
            1,
            "neither key was a command, so the run of kills continues"
        );
    }

    // ---------------- parsing and describing ----------------

    #[test]
    fn a_sequence_that_does_not_wholly_parse_binds_nothing() {
        let (ctx, env) = editor_with("");
        // `C-` is not a key. Binding `C-x` from this would define a prefix
        // that swallows every sequence starting with it. It is said, as an
        // error, and nothing is bound.
        let refused = eval_str(
            "(define-key nil \"C-x C-\" 'beginning-of-buffer)",
            &env,
            &ctx,
        );
        assert!(
            matches!(&refused, Err(EvalError::RuntimeMessage(why)) if why.contains("C-x C-")),
            "{refused:?}"
        );

        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        assert!(
            ctx.get_echo_message().contains("is undefined"),
            "C-x should not have become a prefix: {:?}",
            ctx.get_echo_message()
        );
    }

    /// What the echo area shows has to be spelt the way the binding that
    /// completes it was written, or the hint does not help anyone.
    #[test]
    fn keys_are_described_the_way_bindings_are_written() {
        // Spelt out rather than round-tripped through the parser, so this
        // checks the printer rather than checking the two against each other.
        assert_eq!(
            describe_keys(&[KeyEvent {
                code: KeyCode::Char('x'),
                modifiers: ctrl()
            }]),
            "C-x"
        );
        assert_eq!(
            describe_keys(&[
                KeyEvent {
                    code: KeyCode::Char('x'),
                    modifiers: ctrl()
                },
                KeyEvent {
                    code: KeyCode::Char('f'),
                    modifiers: ctrl()
                },
            ]),
            "C-x C-f"
        );
        assert_eq!(
            describe_keys(&[KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: ctrl()
            }]),
            "C-<space>"
        );
    }

    // ---------------- taking a binding away ----------------

    /// nil takes a binding away; once nothing is left under a prefix, the
    /// prefix goes too and the key is free again.
    #[test]
    fn define_key_with_nil_takes_a_binding_and_its_prefix_away() {
        let (ctx, env) = editor_with("");
        eval_str(
            r#"(define-key nil "C-c 1" 'end-of-line)
               (define-key nil "C-c 2" 'beginning-of-line)"#,
            &env,
            &ctx,
        )
        .expect("binding");
        let asked = |src: &str| eval_str(src, &env, &ctx).expect(src);

        assert!(asked(r#"(define-key nil "C-c 1" nil)"#).is_truthy());
        assert!(asked(r#"(key-binding "C-c 1")"#).is_nil());
        assert_eq!(
            asked(r#"(length (keys-with-prefix "C-c"))"#),
            LispExp::number(1.0),
            "C-c 2 is still bound"
        );

        asked(r#"(define-key nil "C-c 2" nil)"#);
        assert!(asked(r#"(keys-with-prefix "C-c")"#).is_nil());
        press(&ctx, &env, KeyCode::Char('c'), ctrl());
        assert_eq!(
            ctx.get_echo_message(),
            "C-c is undefined",
            "C-c leads nowhere now, so it is not waited on"
        );
    }

    /// Taken away from a mode, a key reaches the global keymap again.
    #[test]
    fn define_key_with_nil_in_a_mode_lets_the_global_binding_through() {
        let (ctx, env) = editor_with("");
        eval_str(
            r#"(make-mode 'probe-mode)
               (define-key 'probe-mode "<down>" 'end-of-line)"#,
            &env,
            &ctx,
        )
        .expect("binding");
        ctx.with_current_buffer_mut(|buf| buf.current_mode = "probe-mode".to_string());
        let source = || eval_str(r#"(nth 2 (key-binding "<down>"))"#, &env, &ctx).expect("asking");

        assert_eq!(source(), LispExp::string("mode".into()));
        eval_str(r#"(define-key 'probe-mode "<down>" nil)"#, &env, &ctx).expect("unbinding");
        assert_eq!(source(), LispExp::string("global".into()));
        assert!(
            eval_str(r#"(define-key 'no-such-mode "<down>" nil)"#, &env, &ctx).is_err(),
            "a mode that does not exist is refused, as an error"
        );
    }

    // ---------------- default bindings that are sequences ----------------

    #[test]
    fn the_prefixed_region_bindings_work() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");

        // C-x h -- the whole buffer becomes the region.
        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        press(&ctx, &env, KeyCode::Char('h'), plain());
        assert_eq!(
            eval_str("(region-end)", &env, &ctx).expect("region-end"),
            LispExp::number(10.0)
        );

        // C-x C-x -- point and mark swap.
        assert_eq!(point_1d(&ctx), 0);
        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        press(&ctx, &env, KeyCode::Char('x'), ctrl());
        assert_eq!(point_1d(&ctx), 10, "point should be where the mark was");
    }

    #[test]
    fn the_listing_has_the_character_rule_once_and_no_character_on_its_own() {
        let (ctx, env) = editor_with("");
        assert_eq!(
            eval_str("(car (keymap-bindings))", &env, &ctx).unwrap(),
            eval_str(r#"(list "<char>" 'self-insert "global")"#, &env, &ctx).unwrap()
        );
        assert_eq!(
            eval_str(r#"(assoc "a" (keymap-bindings))"#, &env, &ctx).unwrap(),
            LispExp::nil()
        );
    }

    // ---------------- Delete, Home, End, the page keys, the function keys ----------------

    /// Each name parses, and the key it gives is described back by the same
    /// name: the one `describe-key` and the help pages show.
    #[test]
    fn the_editing_and_function_keys_have_names_that_read_back() {
        use crate::input::parse_key_sequence;
        for name in [
            "<delete>",
            "<insert>",
            "<home>",
            "<end>",
            "<prior>",
            "<next>",
            "<f1>",
            "<f12>",
            "<f35>",
            "C-<home>",
            "M-<f5>",
            "C-x <delete>",
        ] {
            let keys = parse_key_sequence(name).unwrap_or_else(|| panic!("{name} must parse"));
            assert_eq!(describe_keys(&keys), name);
        }
        // The names on the keys, and Emacs' other name for Delete.
        for (alias, name) in [
            ("<pageup>", "<prior>"),
            ("<pagedown>", "<next>"),
            ("<deletechar>", "<delete>"),
        ] {
            let keys = parse_key_sequence(alias).expect("an alias parses");
            assert_eq!(describe_keys(&keys), name);
        }
        for wrong in ["<f0>", "<f36>", "<f01>", "<f>", "<f+1>", "<fx>"] {
            assert_eq!(parse_key_sequence(wrong), None, "{wrong} is no key");
        }
    }

    #[test]
    fn the_editing_keys_do_what_their_emacs_keys_do() {
        let (ctx, env) = editor_with("one\ntwo\nthree");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");
        eval_str("(goto-char 5)", &env, &ctx).expect("into the second line");

        press(&ctx, &env, KeyCode::Home, plain());
        assert_eq!(point_1d(&ctx), 4, "Home goes to the start of the line");
        press(&ctx, &env, KeyCode::End, plain());
        assert_eq!(point_1d(&ctx), 7, "End to its end");
        press(&ctx, &env, KeyCode::Home, ctrl());
        assert_eq!(point_1d(&ctx), 0, "C-Home to the start of the buffer");
        press(&ctx, &env, KeyCode::Delete, plain());
        assert_eq!(text_of(&ctx), "ne\ntwo\nthree", "Delete deletes forward");
        press(&ctx, &env, KeyCode::End, ctrl());
        assert_eq!(point_1d(&ctx), 12, "C-End to the end of the buffer");

        for (key, command) in [
            ("<next>", "scroll-up-command"),
            ("<prior>", "scroll-down-command"),
        ] {
            assert_eq!(
                eval_str(
                    &format!(r#"(car (nth 1 (key-binding "{key}")))"#),
                    &env,
                    &ctx
                )
                .expect("key-binding"),
                LispExp::symbol(command.into()),
                "{key} scrolls as C-v and M-v do"
            );
        }
    }

    // ---------------- modifiers in any order ----------------

    /// One prefix only was read: `M-C-x` and `C-S-a` were not keys, and
    /// `define-key` said so to the log alone.
    #[test]
    fn modifiers_are_read_in_any_order_each_once() {
        use crate::input::parse_key;
        let key = |code, ctrl, alt| KeyEvent {
            code,
            modifiers: KeyModifiers {
                ctrl,
                alt,
                shift: false,
            },
        };
        assert_eq!(parse_key("M-C-x"), parse_key("C-M-x"));
        assert_eq!(
            parse_key("M-C-x"),
            Some(key(KeyCode::Char('x'), true, true))
        );
        assert_eq!(
            parse_key("C-S-a"),
            Some(key(KeyCode::Char('A'), true, false))
        );
        assert_eq!(
            parse_key("S-a"),
            Some(key(KeyCode::Char('A'), false, false))
        );
        assert_eq!(parse_key("C--"), Some(key(KeyCode::Char('-'), true, false)));
        let shift_tab = parse_key("S-<tab>").expect("a key");
        assert!(shift_tab.modifiers.shift && shift_tab.code == KeyCode::Tab);
        assert_eq!(parse_key("C-C-x"), None, "Control twice");
        assert_eq!(parse_key("S-1"), None, "1 has no capital");
    }

    #[test]
    fn a_binding_written_m_c_runs_on_control_meta() {
        let (ctx, env) = editor_with("");
        eval_str("(define-key nil \"M-C-y\" '(insert \"done\"))", &env, &ctx).expect("binding");
        press(
            &ctx,
            &env,
            KeyCode::Char('y'),
            KeyModifiers {
                ctrl: true,
                alt: true,
                shift: false,
            },
        );
        assert_eq!(text_of(&ctx), "done");
    }

    #[test]
    fn a_binding_in_a_mode_that_does_not_exist_is_an_error() {
        let (ctx, env) = editor_with("");
        for call in [
            "(define-key 'no-such-mode \"C-c a\" 'forward-char)",
            "(define-key 'no-such-mode \"<char>\" 'self-insert)",
        ] {
            assert!(eval_str(call, &env, &ctx).is_err(), "{call}");
        }
    }
}
