//! Key sequences: a key that completes a binding, one that starts a longer
//! one, and one that does neither.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers, describe_keys};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
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
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| {
            b.text = GapBuffer::from(text);
            b.text.cursor_move(0, 0);
            b.is_modified = false;
        });
        (ctx, env)
    }

    fn text_of(ctx: &Ctx) -> String {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .to_string()
    }

    fn point_1d(ctx: &Ctx) -> usize {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .cursor_pos_1d()
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
            ctx.get_buffer("*scratch*")
                .expect("*scratch*")
                .read()
                .unwrap()
                .mark
                .is_none(),
            "the first key of a sequence must not run anything"
        );

        press(&ctx, &env, KeyCode::Char('m'), plain());
        assert!(
            ctx.get_buffer("*scratch*")
                .expect("*scratch*")
                .read()
                .unwrap()
                .mark
                .is_some(),
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
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
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
        // that swallows every sequence starting with it.
        eval_str(
            "(define-key nil \"C-x C-\" 'beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("no panic");

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

    // ---------------- the bindings this unblocked ----------------

    #[test]
    fn the_prefixed_region_bindings_work() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
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
}
