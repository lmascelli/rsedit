//! Deletion commands, and their correspondence with the movement commands.
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

    fn point(ctx: &Ctx) -> (usize, usize) {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .cursor_pos()
    }

    // ---------------- characters ----------------

    #[test]
    fn delete_char_removes_forward_and_crosses_lines() {
        let (ctx, env) = editor_with("ab\ncd");
        eval_str("(delete-char)", &env, &ctx).expect("delete a");
        assert_eq!(text_of(&ctx), "b\ncd");

        eval_str("(end-of-line) (delete-char)", &env, &ctx).expect("delete the newline");
        assert_eq!(
            text_of(&ctx),
            "bcd",
            "deleting at end of line should join them"
        );
    }

    #[test]
    fn delete_char_takes_a_count_and_stops_at_the_end() {
        let (ctx, env) = editor_with("abcdef");
        eval_str("(delete-char 3)", &env, &ctx).expect("three characters");
        assert_eq!(text_of(&ctx), "def");
        eval_str("(delete-char 99)", &env, &ctx).expect("clamped");
        assert_eq!(text_of(&ctx), "");
    }

    // ---------------- lines ----------------

    #[test]
    fn kill_line_clears_to_the_end_then_takes_the_newline() {
        let (ctx, env) = editor_with("hello world\nsecond");
        eval_str("(forward-char 5)", &env, &ctx).expect("after hello");
        eval_str("(kill-line)", &env, &ctx).expect("to end of line");
        assert_eq!(text_of(&ctx), "hello\nsecond");

        // Point is now at the end of the line, so the next kill takes the
        // newline and joins the lines -- what makes repeated C-k progress.
        eval_str("(kill-line)", &env, &ctx).expect("the newline");
        assert_eq!(text_of(&ctx), "hellosecond");
    }

    #[test]
    fn repeated_kill_line_swallows_a_paragraph() {
        let (ctx, env) = editor_with("one\ntwo\nthree\n");
        for _ in 0..6 {
            eval_str("(kill-line)", &env, &ctx).expect("kill");
        }
        assert_eq!(text_of(&ctx), "");
    }

    #[test]
    fn kill_whole_line_takes_the_line_and_its_newline() {
        let (ctx, env) = editor_with("one\ntwo\nthree");
        eval_str(
            "(goto-line 2) (forward-char 1) (kill-whole-line)",
            &env,
            &ctx,
        )
        .expect("kill the middle line from within it");
        assert_eq!(text_of(&ctx), "one\nthree");
        assert_eq!(point(&ctx), (1, 0));
    }

    // ---------------- words ----------------

    #[test]
    fn kill_word_and_backward_kill_word() {
        let (ctx, env) = editor_with("alpha beta gamma");
        eval_str("(kill-word)", &env, &ctx).expect("kill alpha");
        assert_eq!(text_of(&ctx), " beta gamma");

        eval_str("(end-of-line) (backward-kill-word)", &env, &ctx).expect("kill gamma");
        assert_eq!(text_of(&ctx), " beta ");
    }

    #[test]
    fn word_deletion_takes_a_count() {
        let (ctx, env) = editor_with("one two three four");
        eval_str("(kill-word 2)", &env, &ctx).expect("two words");
        assert_eq!(text_of(&ctx), " three four");
    }

    // ---------------- paragraphs ----------------

    #[test]
    fn paragraph_deletion() {
        let (ctx, env) = editor_with("one\ntwo\n\nthree\nfour");
        eval_str("(kill-paragraph)", &env, &ctx).expect("kill the first paragraph");
        assert_eq!(text_of(&ctx), "\nthree\nfour");

        let (ctx, env) = editor_with("one\ntwo\n\nthree\nfour");
        eval_str("(end-of-buffer) (backward-kill-paragraph)", &env, &ctx)
            .expect("kill back over the second paragraph");
        assert_eq!(text_of(&ctx), "one\ntwo\n");
    }

    // ---------------- the coupling ----------------

    /// The property that keeps these honest: a deletion removes exactly the
    /// text its movement counterpart travels over. Checked by running the
    /// movement to find where it lands, then the deletion, and comparing
    /// against the text with that span cut out.
    #[test]
    fn each_deletion_removes_exactly_what_its_movement_traverses() {
        const TEXT: &str = "alpha, beta_2 gamma\ndelta epsilon\n\nzeta eta";
        for (movement, deletion, start) in [
            ("(forward-word 2)", "(kill-word 2)", 0usize),
            ("(forward-word)", "(kill-word)", 7),
            ("(backward-word)", "(backward-kill-word)", 19),
            ("(forward-paragraph)", "(kill-paragraph)", 0),
            ("(end-of-line)", "(kill-line)", 7),
        ] {
            // Where does the movement land?
            let (ctx, env) = editor_with(TEXT);
            eval_str(&format!("(forward-char {start}) {movement}"), &env, &ctx).expect("movement");
            let landed = ctx
                .get_buffer("*scratch*")
                .unwrap()
                .read()
                .unwrap()
                .text
                .cursor_pos_1d();

            // Cut that span out by hand.
            let chars: Vec<char> = TEXT.chars().collect();
            let (from, to) = (start.min(landed), start.max(landed));
            let expected: String = chars[..from].iter().chain(chars[to..].iter()).collect();

            // And what does the deletion actually leave?
            let (ctx, env) = editor_with(TEXT);
            eval_str(&format!("(forward-char {start}) {deletion}"), &env, &ctx).expect("deletion");
            assert_eq!(
                text_of(&ctx),
                expected,
                "{deletion} did not remove what {movement} traverses"
            );
        }
    }

    // ---------------- bookkeeping ----------------

    #[test]
    fn deleting_marks_the_buffer_modified() {
        let (ctx, env) = editor_with("abc");
        assert!(
            !ctx.get_buffer("*scratch*")
                .unwrap()
                .read()
                .unwrap()
                .is_modified
        );
        eval_str("(delete-char)", &env, &ctx).expect("delete");
        assert!(
            ctx.get_buffer("*scratch*")
                .unwrap()
                .read()
                .unwrap()
                .is_modified,
            "a deletion must mark the buffer modified"
        );
    }

    #[test]
    fn the_deletion_commands_are_all_registered() {
        let (ctx, env) = editor_with("x");
        for name in [
            "delete-char",
            "kill-line",
            "kill-whole-line",
            "kill-word",
            "backward-kill-word",
            "kill-paragraph",
            "backward-kill-paragraph",
        ] {
            assert_eq!(
                eval_str(&format!("(commandp '{name})"), &env, &ctx).expect("commandp"),
                LispExp::t(),
                "{name} should be a command"
            );
        }
    }

    /// The bindings must actually reach the commands -- in particular
    /// `M-<backspace>`, which is the only one whose key name is not a plain
    /// character and so is the one that could silently fail to parse.
    #[test]
    fn the_deletion_keys_are_bound() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");

        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
            &env,
        );
        assert_eq!(text_of(&ctx), "lpha beta", "C-d should delete a character");

        eval_str("(end-of-line)", &env, &ctx).expect("to the end");
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers {
                    alt: true,
                    ..Default::default()
                },
            },
            &env,
        );
        assert_eq!(
            text_of(&ctx),
            "lpha ",
            "M-<backspace> should kill the word before point"
        );
    }
}
