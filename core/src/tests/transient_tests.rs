//! Keymaps that last for a moment: repeat keys, and the two things a map can
//! do with a key it does not bind.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers, Keymap, OnUnbound, TransientKeymap};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| {
            b.text = GapBuffer::from(text);
            b.text.cursor_move(0, 0);
            b.is_modified = false;
        });
        (ctx, env)
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode) {
        ctx.handle_key_event(
            KeyEvent {
                code,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    fn press_ctrl(ctx: &Ctx, env: &Arc<Env<Ctx>>, c: char) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
            env,
        );
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c))
    }

    fn text_of(ctx: &Ctx) -> String {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .to_string()
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .cursor_pos_1d()
    }

    /// What the frame says a transient map is offering.
    fn offer(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
        ctx.snapshot(env, W, H).prompt
    }

    /// A map binding one key to a counter, so a test can see how many of its
    /// presses the map answered.
    fn counting_map(
        ctx: &Ctx,
        env: &Arc<Env<Ctx>>,
        on_unbound: OnUnbound,
    ) -> TransientKeymap<GapBuffer> {
        eval_str(
            "(progn (setq hits 0) (defun bump () (setq hits (+ hits 1))))",
            env,
            ctx,
        )
        .expect("defining the counter");
        let mut keymap = Keymap::new();
        keymap.insert_key(key('k'), LispExp::symbol("bump".into()));
        TransientKeymap {
            keymap,
            on_unbound,
            message: "[k]".into(),
        }
    }

    fn hits(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> f64 {
        match eval_str("hits", env, ctx).expect("hits") {
            LispExp::Number(n) => n,
            other => panic!("hits should be a number, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // The map is consulted first
    // -----------------------------------------------------------------------

    #[test]
    fn a_transient_map_answers_before_the_global_one() {
        let (ctx, env) = editor_with("");
        eval_str(r#"(define-key nil "k" 'end-of-buffer)"#, &env, &ctx).expect("bind");
        ctx.set_transient_keymap(counting_map(&ctx, &env, OnUnbound::Release));

        press(&ctx, &env, KeyCode::Char('k'));

        assert_eq!(hits(&ctx, &env), 1.0, "the transient map took it");
        assert_eq!(
            text_of(&ctx),
            "",
            "and the global binding for the same key did not also run"
        );
    }

    #[test]
    fn the_map_says_it_is_there() {
        let (ctx, env) = editor_with("");
        assert_eq!(offer(&ctx, &env), "");

        ctx.set_transient_keymap(counting_map(&ctx, &env, OnUnbound::Release));
        assert_eq!(
            offer(&ctx, &env),
            "[k]",
            "an offer is the editor waiting for a key, so it is reported there"
        );

        ctx.clear_transient_keymap();
        assert_eq!(offer(&ctx, &env), "");
    }

    // -----------------------------------------------------------------------
    // Release: the key is handed on
    // -----------------------------------------------------------------------

    /// The whole point of `Release`: ignoring the offer has to cost nothing.
    #[test]
    fn release_hands_an_unbound_key_to_the_keymaps_underneath() {
        let (ctx, env) = editor_with("hello");
        ctx.set_transient_keymap(counting_map(&ctx, &env, OnUnbound::Release));

        // `z` self-inserts, and it must still do so.
        press(&ctx, &env, KeyCode::Char('z'));

        assert_eq!(text_of(&ctx), "zhello", "the key did its ordinary job");
        assert_eq!(hits(&ctx, &env), 0.0);
        assert!(
            !ctx.transient_keymap_active(),
            "and the map is gone, having been ignored"
        );
    }

    #[test]
    fn release_leaves_nothing_on_screen_once_dismissed() {
        let (ctx, env) = editor_with("hello");
        ctx.set_transient_keymap(counting_map(&ctx, &env, OnUnbound::Release));
        press(&ctx, &env, KeyCode::Char('z'));

        assert_eq!(offer(&ctx, &env), "");
    }

    // -----------------------------------------------------------------------
    // Refuse: the key is swallowed
    // -----------------------------------------------------------------------

    /// The whole point of `Refuse`: a question must not be walked away from,
    /// because nothing would then say it was still waiting.
    #[test]
    fn refuse_swallows_an_unbound_key() {
        let (ctx, env) = editor_with("hello");
        ctx.set_transient_keymap(counting_map(&ctx, &env, OnUnbound::Refuse));

        press(&ctx, &env, KeyCode::Char('z'));

        assert_eq!(text_of(&ctx), "hello", "the key did nothing at all");
        assert!(
            ctx.transient_keymap_active(),
            "and the map is still waiting for an answer"
        );
        assert_eq!(offer(&ctx, &env), "[k]");
    }

    /// A modal map takes `C-g` too, so anything using one has to bind its own
    /// way out -- which is why this is worth pinning.
    #[test]
    fn refuse_swallows_even_keyboard_quit() {
        let (ctx, env) = editor_with("hello");
        eval_str(r#"(define-key nil "C-g" 'keyboard-quit)"#, &env, &ctx).expect("bind");
        ctx.set_transient_keymap(counting_map(&ctx, &env, OnUnbound::Refuse));

        press_ctrl(&ctx, &env, 'g');

        assert!(
            ctx.transient_keymap_active(),
            "C-g is not special to a modal map: it must bind its own exit"
        );
    }

    /// A question is answered by its *own* commands, and the repeat-key rule
    /// runs after every command -- so that rule must not take the question
    /// down. This is what a `query-replace` needs: `y` replaces one match and
    /// the question for the next one is still there.
    #[test]
    fn a_refuse_map_survives_a_command_run_from_its_own_binding() {
        let (ctx, env) = editor_with("");
        ctx.set_transient_keymap(counting_map(&ctx, &env, OnUnbound::Refuse));

        for expected in [1.0, 2.0, 3.0] {
            press(&ctx, &env, KeyCode::Char('k'));
            assert_eq!(hits(&ctx, &env), expected);
            assert!(
                ctx.transient_keymap_active(),
                "answering must not dismiss the map that asked"
            );
        }
    }

    #[test]
    fn a_bound_key_runs_whatever_the_policy_is() {
        for policy in [OnUnbound::Refuse, OnUnbound::Release] {
            let (ctx, env) = editor_with("");
            ctx.set_transient_keymap(counting_map(&ctx, &env, policy));
            press(&ctx, &env, KeyCode::Char('k'));
            assert_eq!(
                hits(&ctx, &env),
                1.0,
                "{policy:?} must still answer its own key"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Repeat keys
    // -----------------------------------------------------------------------

    fn two_windows(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        eval_str(
            r#"(progn (define-key nil "C-x o" 'other-window)
                      (define-repeat-key 'other-window "o")
                      (split-window-right))"#,
            env,
            ctx,
        )
        .expect("two windows and a repeat key");
    }

    fn focused(ctx: &Ctx) -> usize {
        ctx.get_focused_window_id()
    }

    #[test]
    fn the_repeat_key_is_offered_after_the_command_runs() {
        let (ctx, env) = editor_with("");
        two_windows(&ctx, &env);
        assert!(!ctx.transient_keymap_active());

        press_ctrl(&ctx, &env, 'x');
        press(&ctx, &env, KeyCode::Char('o'));

        assert!(ctx.transient_keymap_active(), "the offer stands");
        assert_eq!(offer(&ctx, &env), "[o]");
    }

    /// `C-x o o o` -- the reason this mechanism exists.
    #[test]
    fn a_bare_repeat_key_runs_the_command_again() {
        let (ctx, env) = editor_with("");
        two_windows(&ctx, &env);
        let start = focused(&ctx);

        press_ctrl(&ctx, &env, 'x');
        press(&ctx, &env, KeyCode::Char('o'));
        let after_one = focused(&ctx);
        assert_ne!(after_one, start, "the prefixed form moved");

        press(&ctx, &env, KeyCode::Char('o'));
        assert_eq!(focused(&ctx), start, "and a bare `o' moved again");

        press(&ctx, &env, KeyCode::Char('o'));
        assert_eq!(focused(&ctx), after_one, "and again");
    }

    /// The offer is renewed by the same path that made it, rather than by the
    /// command keeping itself alive.
    #[test]
    fn the_offer_is_renewed_after_each_repeat() {
        let (ctx, env) = editor_with("");
        two_windows(&ctx, &env);

        press_ctrl(&ctx, &env, 'x');
        press(&ctx, &env, KeyCode::Char('o'));
        for _ in 0..3 {
            press(&ctx, &env, KeyCode::Char('o'));
            assert!(ctx.transient_keymap_active(), "still offering");
        }
    }

    #[test]
    fn any_other_key_ends_the_run_and_does_its_own_job() {
        let (ctx, env) = editor_with("hello");
        two_windows(&ctx, &env);
        press_ctrl(&ctx, &env, 'x');
        press(&ctx, &env, KeyCode::Char('o'));

        press(&ctx, &env, KeyCode::Char('z'));

        assert!(!ctx.transient_keymap_active(), "the run is over");
        assert!(
            text_of(&ctx).contains('z'),
            "and the key that ended it still typed, got {:?}",
            text_of(&ctx)
        );
    }

    /// A command with no repeat key takes down whatever the previous one
    /// offered -- otherwise `o` would keep cycling windows long afterwards.
    #[test]
    fn a_command_without_a_repeat_key_takes_the_offer_down() {
        let (ctx, env) = editor_with("hello");
        two_windows(&ctx, &env);
        eval_str(r#"(define-key nil "C-e" 'end-of-buffer)"#, &env, &ctx).expect("bind");
        press_ctrl(&ctx, &env, 'x');
        press(&ctx, &env, KeyCode::Char('o'));

        press_ctrl(&ctx, &env, 'e');

        assert!(!ctx.transient_keymap_active());
        assert_eq!(point(&ctx), 5, "and it did its own job");
    }

    /// Without a declaration nothing repeats, which is the point of declaring:
    /// `C-x C-f` followed by `f` must not re-open `find-file`.
    #[test]
    fn nothing_repeats_unless_it_was_declared() {
        let (ctx, env) = editor_with("");
        eval_str(
            r#"(progn (define-key nil "C-x o" 'other-window) (split-window-right))"#,
            &env,
            &ctx,
        )
        .expect("two windows, no repeat key");

        press_ctrl(&ctx, &env, 'x');
        press(&ctx, &env, KeyCode::Char('o'));

        assert!(!ctx.transient_keymap_active());
    }

    #[test]
    fn a_repeat_key_must_be_a_single_key() {
        let (ctx, env) = editor_with("");
        assert!(
            eval_str(r#"(define-repeat-key 'other-window "C-x o")"#, &env, &ctx).is_err(),
            "a two-press repeat key would save nobody anything"
        );
        assert!(
            eval_str(
                r#"(define-repeat-key 'other-window "nonsense")"#,
                &env,
                &ctx
            )
            .is_err()
        );
        assert!(eval_str(r#"(define-repeat-key 'other-window "C-o")"#, &env, &ctx).is_ok());
    }

    /// The offer must not eat a key that begins a longer binding of its own.
    #[test]
    fn the_offer_does_not_swallow_a_prefix_key() {
        let (ctx, env) = editor_with("hello");
        two_windows(&ctx, &env);
        eval_str(r#"(define-key nil "C-x h" 'mark-whole-buffer)"#, &env, &ctx).expect("bind");
        press_ctrl(&ctx, &env, 'x');
        press(&ctx, &env, KeyCode::Char('o'));

        // `C-x` is not `o`, so the offer is dismissed and the sequence begins.
        press_ctrl(&ctx, &env, 'x');
        press(&ctx, &env, KeyCode::Char('h'));

        assert_eq!(
            ctx.get_buffer("*scratch*")
                .expect("*scratch*")
                .read()
                .unwrap()
                .mark
                .map(|m| m.at),
            Some(5),
            "the two-key sequence must have completed"
        );
    }
}
