//! Cursor movement: characters, lines, words, paragraphs and the whole buffer.
//!
//! Driven through `eval` against a real editor rather than by poking the
//! buffer, so the tests cover what a keystroke actually does.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    /// An editor whose *scratch* buffer holds `text`, point at the start.
    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            b.text.cursor_move(0, 0);
        });
        (ctx, env)
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode, modifiers: KeyModifiers) {
        ctx.handle_key_event(KeyEvent { code, modifiers }, env);
    }

    fn alt() -> KeyModifiers {
        KeyModifiers {
            alt: true,
            ..Default::default()
        }
    }

    fn plain() -> KeyModifiers {
        KeyModifiers::default()
    }

    fn point_1d(ctx: &Ctx) -> usize {
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos_1d())
            .expect("*scratch*")
    }

    fn point(ctx: &Ctx) -> (usize, usize) {
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos())
            .expect("*scratch*")
    }

    /// Run `src` as if it were a command with the given name, so that
    /// `last-command` is set the way `handle_key_event` would set it.
    fn run_command(name: &str, ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        eval_str(&format!("({name})"), env, ctx).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        ctx.set_last_command(Some(Arc::new(name.to_string())));
    }

    // ---------------- characters ----------------

    /// `forward-char` used to move within the line only, so point stopped dead
    /// at the end of a line instead of continuing onto the next.
    #[test]
    fn forward_char_crosses_the_end_of_a_line() {
        let (ctx, env) = editor_with("ab\ncd");
        eval_str("(forward-char 2)", &env, &ctx).expect("to end of line 0");
        assert_eq!(point(&ctx), (0, 2));

        eval_str("(forward-char)", &env, &ctx).expect("across the newline");
        assert_eq!(point(&ctx), (1, 0), "point should have moved onto line 1");
    }

    #[test]
    fn backward_char_crosses_the_start_of_a_line() {
        let (ctx, env) = editor_with("ab\ncd");
        eval_str("(goto-line 2)", &env, &ctx).expect("line 2");
        assert_eq!(point(&ctx), (1, 0));

        eval_str("(backward-char)", &env, &ctx).expect("back across the newline");
        assert_eq!(point(&ctx), (0, 2), "point should be at the end of line 0");
    }

    #[test]
    fn character_movement_stops_at_the_buffer_edges() {
        let (ctx, env) = editor_with("ab");
        eval_str("(backward-char 99)", &env, &ctx).expect("clamped at the start");
        assert_eq!(point(&ctx), (0, 0));
        eval_str("(forward-char 99)", &env, &ctx).expect("clamped at the end");
        assert_eq!(point(&ctx), (0, 2));
    }

    // ---------------- lines ----------------

    #[test]
    fn beginning_and_end_of_line() {
        let (ctx, env) = editor_with("hello\nworld");
        eval_str("(goto-line 2) (end-of-line)", &env, &ctx).expect("end of line 2");
        assert_eq!(point(&ctx), (1, 5));
        eval_str("(beginning-of-line)", &env, &ctx).expect("start of line 2");
        assert_eq!(point(&ctx), (1, 0));
    }

    /// The behaviour that was missing: a short line in the middle of a run of
    /// vertical moves must not cost you the column you started from.
    #[test]
    fn vertical_movement_keeps_the_goal_column_across_a_short_line() {
        let (ctx, env) = editor_with("aaaaaaaaaa\nbb\ncccccccccc");
        eval_str("(end-of-line)", &env, &ctx).expect("column 10 on line 0");
        assert_eq!(point(&ctx), (0, 10));

        run_command("next-line", &ctx, &env);
        assert_eq!(point(&ctx), (1, 2), "clamped to the short line");

        run_command("next-line", &ctx, &env);
        assert_eq!(
            point(&ctx),
            (2, 10),
            "the goal column must be restored on the long line below"
        );

        run_command("previous-line", &ctx, &env);
        run_command("previous-line", &ctx, &env);
        assert_eq!(point(&ctx), (0, 10), "and again on the way back up");
    }

    /// The goal is only kept while vertical movement continues -- any other
    /// command means the user is aiming somewhere new.
    #[test]
    fn another_command_resets_the_goal_column() {
        let (ctx, env) = editor_with("aaaaaaaaaa\nbb\ncccccccccc");
        eval_str("(end-of-line)", &env, &ctx).expect("column 10");
        run_command("next-line", &ctx, &env);
        assert_eq!(point(&ctx), (1, 2));

        // A horizontal move in the middle of the run.
        run_command("beginning-of-line", &ctx, &env);
        run_command("next-line", &ctx, &env);
        assert_eq!(
            point(&ctx),
            (2, 0),
            "after another command the column should be read afresh, not restored"
        );
    }

    #[test]
    fn vertical_movement_stops_at_the_first_and_last_line() {
        let (ctx, env) = editor_with("one\ntwo");
        run_command("previous-line", &ctx, &env);
        assert_eq!(point(&ctx), (0, 0), "already on the first line");
        for _ in 0..5 {
            run_command("next-line", &ctx, &env);
        }
        assert_eq!(point(&ctx).0, 1, "must not run past the last line");
    }

    // ---------------- words ----------------

    #[test]
    fn word_movement_crosses_punctuation_and_lines() {
        let (ctx, env) = editor_with("alpha, beta\ngamma_2 delta");
        eval_str("(forward-word)", &env, &ctx).expect("past alpha");
        assert_eq!(point(&ctx), (0, 5));
        eval_str("(forward-word)", &env, &ctx).expect("past beta");
        assert_eq!(point(&ctx), (0, 11));
        // Underscores and digits are part of a word.
        eval_str("(forward-word)", &env, &ctx).expect("past gamma_2");
        assert_eq!(point(&ctx), (1, 7));

        eval_str("(backward-word)", &env, &ctx).expect("back to the start of gamma_2");
        assert_eq!(point(&ctx), (1, 0));
        eval_str("(backward-word)", &env, &ctx).expect("back to the start of beta");
        assert_eq!(point(&ctx), (0, 7));
    }

    #[test]
    fn word_movement_takes_a_repeat_count() {
        let (ctx, env) = editor_with("one two three four");
        eval_str("(forward-word 3)", &env, &ctx).expect("three words on");
        assert_eq!(point(&ctx), (0, 13));
        eval_str("(backward-word 2)", &env, &ctx).expect("two words back");
        assert_eq!(point(&ctx), (0, 4));
    }

    #[test]
    fn word_movement_stops_at_the_buffer_edges() {
        let (ctx, env) = editor_with("solo");
        eval_str("(backward-word 5)", &env, &ctx).expect("clamped");
        assert_eq!(point(&ctx), (0, 0));
        eval_str("(forward-word 5)", &env, &ctx).expect("clamped");
        assert_eq!(point(&ctx), (0, 4));
    }

    // ---------------- paragraphs ----------------

    #[test]
    fn paragraph_movement_walks_between_blank_lines() {
        let (ctx, env) = editor_with("one\ntwo\n\nthree\nfour\n\nfive");
        eval_str("(forward-paragraph)", &env, &ctx).expect("to the first blank line");
        assert_eq!(point(&ctx), (2, 0));

        // Repeating steps off the separator rather than sticking to it.
        eval_str("(forward-paragraph)", &env, &ctx).expect("to the second blank line");
        assert_eq!(point(&ctx), (5, 0));

        eval_str("(backward-paragraph)", &env, &ctx).expect("back to the first blank line");
        assert_eq!(point(&ctx), (2, 0));
        eval_str("(backward-paragraph)", &env, &ctx).expect("back to the start");
        assert_eq!(point(&ctx), (0, 0));
    }

    // ---------------- whole buffer ----------------

    #[test]
    fn buffer_ends_and_goto_line() {
        let (ctx, env) = editor_with("l1\nl2\nl3\nl4");
        eval_str("(end-of-buffer)", &env, &ctx).expect("end");
        assert_eq!(point(&ctx), (3, 2));
        eval_str("(beginning-of-buffer)", &env, &ctx).expect("start");
        assert_eq!(point(&ctx), (0, 0));

        eval_str("(goto-line 3)", &env, &ctx).expect("line 3");
        assert_eq!(point(&ctx), (2, 0), "goto-line counts from 1");
        // Out of range clamps rather than failing.
        eval_str("(goto-line 999)", &env, &ctx).expect("past the end");
        assert_eq!(point(&ctx).0, 3);
        eval_str("(goto-line 0)", &env, &ctx).expect("before the start");
        assert_eq!(point(&ctx).0, 0);
    }

    /// `goto-line` is the first built-in command that takes an argument, so
    /// M-x must prompt for it rather than fail on arity.
    #[test]
    fn goto_line_is_a_command_that_prompts_for_its_argument() {
        let (ctx, env) = editor_with("l1\nl2\nl3");
        assert_eq!(
            eval_str("(commandp 'goto-line)", &env, &ctx).expect("commandp"),
            LispExp::t()
        );
        let specs = eval_str("(command-args 'goto-line)", &env, &ctx).expect("command-args");
        assert_eq!(format!("{specs:?}"), "((number \"Goto line: \"))");
    }

    /// Every movement command is reachable by name.
    #[test]
    fn the_movement_commands_are_all_registered() {
        let (ctx, env) = editor_with("x");
        for name in [
            "beginning-of-line",
            "end-of-line",
            "forward-word",
            "backward-word",
            "forward-paragraph",
            "backward-paragraph",
            "beginning-of-buffer",
            "end-of-buffer",
            "goto-line",
        ] {
            assert_eq!(
                eval_str(&format!("(commandp '{name})"), &env, &ctx).expect("commandp"),
                LispExp::t(),
                "{name} should be a command"
            );
        }
    }

    // ---------------- the keys Emacs actually uses ----------------

    /// Paragraph motion was on `M-n` and `M-p`, which Emacs does not bind
    /// globally at all -- in a prompt they mean "next/previous history entry".
    /// `M-}` and `M-{` are the real bindings.
    #[test]
    fn paragraphs_move_on_the_brace_keys() {
        let (ctx, env) = editor_with("one\n\ntwo\n\nthree\n");
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");
        eval_str("(beginning-of-buffer)", &env, &ctx).expect("start");

        press(&ctx, &env, KeyCode::Char('}'), alt());
        let forward = point_1d(&ctx);
        assert!(forward > 0, "M-}} should have moved forward");

        press(&ctx, &env, KeyCode::Char('{'), alt());
        assert!(point_1d(&ctx) < forward, "M-{{ should have moved back");
    }

    #[test]
    fn the_old_paragraph_keys_are_no_longer_bound() {
        let (ctx, env) = editor_with("one\n\ntwo\n\nthree\n");
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");
        eval_str("(beginning-of-buffer)", &env, &ctx).expect("start");

        press(&ctx, &env, KeyCode::Char('n'), alt());

        assert_eq!(point_1d(&ctx), 0, "M-n no longer moves by a paragraph");
    }

    /// `M-g` is a prefix in Emacs, and both of the keys after it reach
    /// `goto-line`.
    #[test]
    fn goto_line_is_behind_the_m_g_prefix() {
        for second in ['g', 'G'] {
            let (ctx, env) = editor_with("one\ntwo\nthree\nfour\n");
            // `goto-line` prompts, and a prompt is a floating window that has
            // to know how big the frame is.
            env.set_variable("frame-width".into(), LispExp::number(80.0));
            env.set_variable("frame-height".into(), LispExp::number(24.0));
            eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
                .expect("common-keymaps.lisp must load");

            press(&ctx, &env, KeyCode::Char('g'), alt());
            // M-g g and M-g M-g are both bound; the second press differs only
            // in its modifier.
            if second == 'g' {
                press(&ctx, &env, KeyCode::Char('g'), plain());
            } else {
                press(&ctx, &env, KeyCode::Char('g'), alt());
            }
            for c in "3".chars() {
                eval_str(&format!(r#"(self-insert "{c}")"#), &env, &ctx).expect("type");
            }
            eval_str("(minibuffer-confirm)", &env, &ctx).expect("confirm");

            assert_eq!(
                ctx.with_buffer("*scratch*", |b| b.text.cursor_pos().0)
                    .expect("*scratch*"),
                2,
                "M-g then {second} should go to line 3"
            );
        }
    }

    /// A bare `M-g` is now a prefix waiting for a second key, so it must not
    /// go anywhere on its own.
    #[test]
    fn a_bare_m_g_no_longer_prompts() {
        let (ctx, env) = editor_with("one\ntwo\nthree\n");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");

        press(&ctx, &env, KeyCode::Char('g'), alt());

        assert!(
            !ctx.has_buffer("*Minibuffer*"),
            "M-g alone is a prefix, not a command"
        );
    }
}
