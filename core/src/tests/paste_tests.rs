//! Bracketed paste: text arriving from the system clipboard as one event.
//!
//! The whole value of this is in what it *stops*. Without bracketed paste a
//! terminal delivers a paste as the keystrokes it resembles, so pasting a
//! function is indistinguishable from typing it: one undo entry per character,
//! every hook run per character, and -- once auto-pairing exists -- every
//! bracket in the pasted text doubled.
//!
//! So most of these tests check that a paste is *not* typing.
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

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// The editor with pairing on, which is what makes a mishandled paste
    /// visible rather than merely inefficient.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/common-keymaps.lisp"),
            include_str!("../../lisp/electric-pair.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
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

    fn point(ctx: &Ctx) -> usize {
        ctx.get_current_buffer()
            .read()
            .expect("read lock")
            .text
            .cursor_pos_1d()
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, c: char) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    #[test]
    fn a_paste_arrives_whole() {
        let (ctx, env) = editor();
        ctx.handle_paste("hello world".to_string(), &env);
        assert_eq!(contents(&ctx), "hello world");
        assert_eq!(point(&ctx), 11);
    }

    #[test]
    fn a_paste_keeps_its_newlines() {
        let (ctx, env) = editor();
        ctx.handle_paste("fn main() {\n    todo!()\n}\n".to_string(), &env);
        assert_eq!(contents(&ctx), "fn main() {\n    todo!()\n}\n");
    }

    #[test]
    fn brackets_in_pasted_text_are_not_paired() {
        let (ctx, env) = editor();
        // The test this feature exists for. Typed one character at a time with
        // pairing on, `fn main() {}` would come out as `fn main(()) {{}}` or
        // worse -- every opener bringing a partner that the text already had.
        ctx.handle_paste("fn main() {}".to_string(), &env);
        assert_eq!(contents(&ctx), "fn main() {}");
    }

    #[test]
    fn typing_the_same_brackets_does_pair_them() {
        // The other half of the previous test: pairing is on, so the
        // difference above is the paste path and not a disabled feature.
        let (ctx, env) = editor();
        press(&ctx, &env, '(');
        assert_eq!(contents(&ctx), "()", "typing pairs");
    }

    #[test]
    fn a_paste_is_one_undo_step() {
        let (ctx, env) = editor();
        run(r#"(insert "before ")"#, &env, &ctx);
        run("(undo-boundary)", &env, &ctx);
        ctx.handle_paste("pasted text".to_string(), &env);
        assert_eq!(contents(&ctx), "before pasted text");
        run("(undo)", &env, &ctx);
        // The whole paste, not its last character. This is what makes undo
        // usable after pasting a function in.
        assert_eq!(contents(&ctx), "before ");
    }

    #[test]
    fn an_empty_paste_does_nothing_at_all() {
        let (ctx, env) = editor();
        run(r#"(insert "text")"#, &env, &ctx);
        ctx.handle_paste(String::new(), &env);
        assert_eq!(contents(&ctx), "text");
    }

    #[test]
    fn a_paste_into_a_read_only_buffer_is_refused() {
        let (ctx, env) = editor();
        run("(set-buffer-read-only t)", &env, &ctx);
        ctx.handle_paste("nope".to_string(), &env);
        // Through the same door as every other edit, so it obeys the same
        // refusal rather than having its own opinion about it.
        assert_eq!(contents(&ctx), "");
    }

    #[test]
    fn the_primitive_is_callable_from_lisp() {
        // So that a command pasting from somewhere other than the terminal
        // gets exactly the treatment a terminal paste gets, and so this is
        // testable without a terminal.
        let (ctx, env) = editor();
        run(r#"(insert-pasted-text "from lisp")"#, &env, &ctx);
        assert_eq!(contents(&ctx), "from lisp");
    }

    #[test]
    fn a_paste_is_remembered_as_the_last_command() {
        let (ctx, env) = editor();
        ctx.handle_paste("x".to_string(), &env);
        // It went through the command machinery rather than round it, which is
        // what gets it undo grouping and `post-command-hook' for free.
        assert!(ctx.last_command_is("insert-pasted-text"));
    }
}
