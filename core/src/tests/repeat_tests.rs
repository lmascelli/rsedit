//! `repeat`, and the form of the last command, which it runs again: `C-x u`
//! three times is three sequences of two keys, and with a repeat key on
//! `repeat` itself, one sequence and then taps.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/common-keymaps.lisp"),
        ] {
            let ast = Parser::new(&format!("(progn {source})"))
                .next()
                .expect("shipped lisp must parse");
            eval(&ast, env.clone(), &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|b| b.text.to_string())
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_current_buffer(|b| b.text.cursor_pos_1d())
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, c: char, ctrl: bool) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers {
                    ctrl,
                    ..Default::default()
                },
            },
            env,
        );
    }

    #[test]
    fn repeat_runs_the_last_command_again() {
        let (ctx, env) = editor();
        run(r#"(insert "one two three") (goto-char 0)"#, &env, &ctx);
        press(&ctx, &env, 'f', true); // C-f
        assert_eq!(point(&ctx), 1);
        run("(repeat)", &env, &ctx);
        assert_eq!(point(&ctx), 2);
    }

    #[test]
    fn repeating_carries_the_arguments_the_binding_supplied() {
        // The reason a *name* is not enough to repeat with: the keymaps store
        // `(self-insert "a")`, and the character lives in the form.
        let (ctx, env) = editor();
        press(&ctx, &env, 'x', false);
        run("(repeat)", &env, &ctx);
        run("(repeat)", &env, &ctx);
        assert_eq!(contents(&ctx), "xxx");
    }

    #[test]
    fn repeat_never_becomes_the_thing_it_repeats() {
        // Were `repeat` to remember itself, the second call would repeat the
        // repeating, and nothing would move.
        let (ctx, env) = editor();
        run(r#"(insert "abcdef") (goto-char 0)"#, &env, &ctx);
        press(&ctx, &env, 'f', true);
        for _ in 0..3 {
            run("(repeat)", &env, &ctx);
        }
        assert_eq!(point(&ctx), 4);
    }

    #[test]
    fn undo_can_be_repeated() {
        // `C-x u`, again without typing the whole sequence.
        let (ctx, env) = editor();
        for c in ['a', 'b', 'c'] {
            press(&ctx, &env, c, false);
            run("(undo-boundary)", &env, &ctx);
        }
        assert_eq!(contents(&ctx), "abc");
        // Through the keys, not `eval`: what `repeat` remembers is the form the
        // key handler ran, and a command called from Lisp is not it.
        press(&ctx, &env, 'x', true); // C-x
        press(&ctx, &env, 'u', false); // u
        assert_eq!(contents(&ctx), "ab", "C-x u undoes once");
        run("(repeat)", &env, &ctx);
        assert_eq!(contents(&ctx), "a", "and repeat undoes again");
    }

    #[test]
    fn repeat_with_nothing_to_repeat_says_so_rather_than_failing() {
        let (ctx, env) = editor();
        let answer = run("(repeat)", &env, &ctx);
        assert!(answer.is_nil());
    }

    #[test]
    fn a_failed_command_is_not_remembered_as_repeatable() {
        // Only a command that ran is remembered, so `repeat` after an error
        // repeats the last thing that worked.
        let (ctx, env) = editor();
        run(r#"(insert "ab") (goto-char 0)"#, &env, &ctx);
        press(&ctx, &env, 'f', true);
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('\u{1}'),
                modifiers: KeyModifiers {
                    ctrl: true,
                    alt: true,
                    ..Default::default()
                },
            },
            &env,
        );
        run("(repeat)", &env, &ctx);
        assert_eq!(point(&ctx), 2, "C-f is still what repeats");
    }
}
