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

    // ----------------------------------------------------------------
    // Line endings
    // ----------------------------------------------------------------
    //
    // A terminal in bracketed-paste mode hands over the selection's bytes
    // unchanged, and a line break very often arrives as CR -- that is what
    // Enter sends, so that is what many terminals put between pasted lines.
    // Nothing else in the editor treats CR as a line break: the line index
    // counts '\n' and nothing else. So a ten-line paste used to land as one
    // very long line with nine invisible characters in it.
    //
    // The symptom was reported as "pasting multiple lines keeps the line on a
    // single row", which is why these assert the line *count* as well as the
    // text: the count is what the user was looking at.

    fn line_count(ctx: &Ctx) -> usize {
        ctx.get_current_buffer()
            .read()
            .expect("read lock")
            .text
            .line_count()
    }

    #[test]
    fn a_paste_with_crlf_endings_becomes_lines() {
        let (ctx, env) = editor();
        ctx.handle_paste("one\r\ntwo\r\nthree".to_string(), &env);
        assert_eq!(contents(&ctx), "one\ntwo\nthree");
        assert_eq!(line_count(&ctx), 3, "three rows, not one");
    }

    #[test]
    fn a_paste_with_bare_carriage_returns_becomes_lines() {
        // The case the bug was actually about: CR alone, with no LF after it.
        let (ctx, env) = editor();
        ctx.handle_paste("one\rtwo\rthree".to_string(), &env);
        assert_eq!(contents(&ctx), "one\ntwo\nthree");
        assert_eq!(line_count(&ctx), 3);
    }

    #[test]
    fn a_mix_of_endings_all_become_lines() {
        // A clipboard assembled from more than one source. CRLF has to be
        // matched before the lone CR, or each pair becomes two line breaks.
        let (ctx, env) = editor();
        ctx.handle_paste("a\r\nb\rc\nd".to_string(), &env);
        assert_eq!(contents(&ctx), "a\nb\nc\nd");
        assert_eq!(line_count(&ctx), 4, "four lines, not five");
    }

    #[test]
    fn a_paste_with_plain_newlines_is_left_alone() {
        let (ctx, env) = editor();
        ctx.handle_paste("one\ntwo\n".to_string(), &env);
        assert_eq!(contents(&ctx), "one\ntwo\n");
    }

    #[test]
    fn a_trailing_carriage_return_opens_a_line() {
        let (ctx, env) = editor();
        ctx.handle_paste("one\r".to_string(), &env);
        assert_eq!(contents(&ctx), "one\n");
        assert_eq!(line_count(&ctx), 2, "the line after it is a place to be");
    }

    #[test]
    fn a_carriage_return_inserted_deliberately_is_left_alone() {
        // The asymmetry, stated rather than assumed. Normalising belongs at the
        // paste door because that is where text arrives from outside; a CR
        // written from Lisp on purpose is a CR, and an `insert' that quietly
        // rewrote its argument would be a worse surprise than the bug.
        let (ctx, env) = editor();
        let _ = &env;
        let buffer = ctx.get_current_buffer();
        ctx.mutate_buffer(buffer, |buf| {
            crate::primitives::edits::insert_at_point(buf, "one\rtwo")
        });
        assert_eq!(contents(&ctx), "one\rtwo");
        assert_eq!(line_count(&ctx), 1, "and it is still one row");
    }
}
