//! Bare repeat keys, for the commands most often wanted several times over.
//!
//! `C-x z` repeats any command, which is the general answer. These are the
//! shorthand: after `C-x u`, a bare `u` undoes again. What makes it worth
//! testing separately is the thing a repeat key costs -- the letter stops
//! doing its ordinary job for exactly one keystroke, and only that one.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, LispExp, Parser, eval};
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
        ctx.get_current_buffer()
            .read()
            .expect("read lock")
            .text
            .to_string()
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

    /// Type `text`, one undoable command per character.
    fn typed(ctx: &Ctx, env: &Arc<Env<Ctx>>, text: &str) {
        for c in text.chars() {
            press(ctx, env, c, false);
            run("(undo-boundary)", env, ctx);
        }
    }

    #[test]
    fn u_after_c_x_u_undoes_again() {
        // The complaint: `C-x u` three times was three two-key sequences.
        let (ctx, env) = editor();
        typed(&ctx, &env, "abc");
        assert_eq!(contents(&ctx), "abc");

        press(&ctx, &env, 'x', true); // C-x
        press(&ctx, &env, 'u', false); // u
        assert_eq!(contents(&ctx), "ab");

        press(&ctx, &env, 'u', false); // bare u
        assert_eq!(contents(&ctx), "a");
        press(&ctx, &env, 'u', false);
        assert_eq!(contents(&ctx), "");
    }

    #[test]
    fn the_offer_is_shown_while_it_stands() {
        // A key that quietly changes meaning would be worse than no key at
        // all, so the frame says so.
        let (ctx, env) = editor();
        typed(&ctx, &env, "ab");
        press(&ctx, &env, 'x', true);
        press(&ctx, &env, 'u', false);
        let frame = ctx.snapshot(&env, 80, 24);
        assert!(
            frame.prompt.contains('u'),
            "the repeat offer should be visible, got {:?}",
            frame.prompt
        );
    }

    #[test]
    fn a_bare_u_with_no_undo_before_it_types_a_u() {
        // The letter only changes meaning for one keystroke *after* the
        // command, which is what makes a repeat key affordable.
        let (ctx, env) = editor();
        press(&ctx, &env, 'u', false);
        assert_eq!(contents(&ctx), "u");
    }

    #[test]
    fn the_offer_lasts_exactly_one_keystroke_of_something_else() {
        let (ctx, env) = editor();
        typed(&ctx, &env, "abc");
        press(&ctx, &env, 'x', true);
        press(&ctx, &env, 'u', false);
        assert_eq!(contents(&ctx), "ab");

        // Something else: the offer goes, and the key does its ordinary job.
        press(&ctx, &env, 'z', false);
        assert_eq!(contents(&ctx), "abz");
        // And now `u` is just a letter again.
        press(&ctx, &env, 'u', false);
        assert_eq!(contents(&ctx), "abzu");
    }

    #[test]
    fn redo_offers_its_own_letter() {
        let (ctx, env) = editor();
        typed(&ctx, &env, "abc");
        press(&ctx, &env, 'x', true);
        press(&ctx, &env, 'u', false);
        press(&ctx, &env, 'u', false);
        assert_eq!(contents(&ctx), "a");

        run("(redo)", &env, &ctx);
        // Through the keyboard so the offer is installed.
        let before = contents(&ctx);
        press(&ctx, &env, 'r', false);
        assert_ne!(contents(&ctx), before, "r should have redone something");
    }

    #[test]
    fn the_general_repeat_still_works_for_everything_else() {
        // The bare letters are shorthand, not a replacement.
        let (ctx, env) = editor();
        run(r#"(insert "one two three") (goto-char 0)"#, &env, &ctx);
        press(&ctx, &env, 'f', true); // C-f
        run("(repeat)", &env, &ctx);
        assert_eq!(
            run("(point)", &env, &ctx),
            LispExp::number(2.0),
            "C-x z repeats commands that have no letter of their own"
        );
    }
}
