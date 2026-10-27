//! The kill ring, through the commands that use it: a run of kills makes one
//! entry, `yank` and `yank-pop` read it and walk it, and the ring is the
//! editor's, shared by every buffer.
//!
//! Design: `doc/architettura-editor.typ, <kill-ring>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
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

    fn mark_is_active(ctx: &Ctx) -> bool {
        ctx.with_buffer("*scratch*", |b| b.mark.is_some_and(|mark| mark.active))
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

    fn alt() -> KeyModifiers {
        KeyModifiers {
            alt: true,
            ..Default::default()
        }
    }

    #[test]
    fn kill_region_removes_the_text_and_yank_puts_it_back() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str("(set-mark) (forward-word) (kill-region)", &env, &ctx).expect("kill");
        assert_eq!(text_of(&ctx), " beta");

        eval_str("(end-of-buffer) (yank)", &env, &ctx).expect("yank");
        assert_eq!(text_of(&ctx), " betaalpha");
    }

    #[test]
    fn kill_ring_save_copies_without_deleting_and_ends_the_selection() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str("(set-mark) (forward-word) (kill-ring-save)", &env, &ctx).expect("copy");

        assert_eq!(text_of(&ctx), "alpha beta", "copying must not delete");
        assert!(
            !mark_is_active(&ctx),
            "the selection is finished with once it has been copied"
        );
        assert_eq!(
            eval_str("(current-kill)", &env, &ctx).expect("current-kill"),
            LispExp::string("alpha".into())
        );
    }

    /// The reason a run of `C-k` is useful: it yanks back as the whole
    /// passage, not just the last line of it.
    #[test]
    fn a_run_of_kills_accumulates_into_one_entry() {
        let (ctx, env) = editor_with("one\ntwo\nthree\n");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");

        for _ in 0..4 {
            press(&ctx, &env, KeyCode::Char('k'), ctrl());
        }
        assert_eq!(
            text_of(&ctx),
            "three\n",
            "two lines and their newlines gone"
        );
        assert_eq!(
            ctx.kill_ring_len(),
            1,
            "a run of kills is one entry, not one per press"
        );
        assert_eq!(
            eval_str("(current-kill)", &env, &ctx).expect("current-kill"),
            LispExp::string("one\ntwo\n".into()),
            "and it reads forwards, in the order the text was in"
        );
    }

    /// A backward kill joins onto the *front* of the entry, or the text would
    /// come back inside out.
    #[test]
    fn a_run_of_backward_kills_reads_the_right_way_round() {
        let (ctx, env) = editor_with("alpha beta gamma");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");
        eval_str("(end-of-buffer)", &env, &ctx).expect("to the end");

        press(&ctx, &env, KeyCode::Backspace, alt());
        press(&ctx, &env, KeyCode::Backspace, alt());
        assert_eq!(text_of(&ctx), "alpha ");
        assert_eq!(
            eval_str("(current-kill)", &env, &ctx).expect("current-kill"),
            LispExp::string("beta gamma".into()),
            "killing backwards twice should not reverse the text"
        );
    }

    #[test]
    fn a_command_between_two_kills_starts_a_new_entry() {
        let (ctx, env) = editor_with("one\ntwo\n");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");

        press(&ctx, &env, KeyCode::Char('k'), ctrl());
        press(&ctx, &env, KeyCode::Char('n'), ctrl());
        press(&ctx, &env, KeyCode::Char('k'), ctrl());
        assert_eq!(
            ctx.kill_ring_len(),
            2,
            "the movement between them ended the run"
        );
    }

    #[test]
    fn yank_on_an_empty_ring_does_nothing() {
        let (ctx, env) = editor_with("text");
        assert_eq!(
            eval_str("(yank)", &env, &ctx).expect("yank"),
            LispExp::nil()
        );
        assert_eq!(text_of(&ctx), "text");
    }

    #[test]
    fn yank_pop_walks_back_through_the_ring_replacing_what_it_yanked() {
        let (ctx, env) = editor_with("");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");
        eval_str(
            "(kill-new \"first\") (deactivate-mark) (kill-new \"second\")",
            &env,
            &ctx,
        )
        .expect("two kills");

        press(&ctx, &env, KeyCode::Char('y'), ctrl());
        assert_eq!(text_of(&ctx), "second", "yank takes the most recent");

        press(&ctx, &env, KeyCode::Char('y'), alt());
        assert_eq!(
            text_of(&ctx),
            "first",
            "yank-pop replaces it with the one before rather than adding to it"
        );

        press(&ctx, &env, KeyCode::Char('y'), alt());
        assert_eq!(text_of(&ctx), "second", "and wraps back round");
    }

    #[test]
    fn yank_pop_refuses_when_the_previous_command_was_not_a_yank() {
        let (ctx, env) = editor_with("");
        eval_str("(kill-new \"text\")", &env, &ctx).expect("a kill");
        assert!(
            eval_str("(yank-pop)", &env, &ctx).is_err(),
            "with no yank to replace there is nothing it could safely remove"
        );
    }

    /// One yank-pop is one command, so it is one undo step -- not a delete and
    /// an insert to be undone separately.
    #[test]
    fn a_yank_and_a_yank_pop_each_undo_in_one_step() {
        let (ctx, env) = editor_with("");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");
        eval_str("(kill-new \"first\") (kill-new \"second\")", &env, &ctx).expect("two kills");

        press(&ctx, &env, KeyCode::Char('y'), ctrl());
        press(&ctx, &env, KeyCode::Char('y'), alt());
        assert_eq!(text_of(&ctx), "first");

        eval_str("(undo)", &env, &ctx).expect("undo");
        assert_eq!(text_of(&ctx), "second", "the yank-pop undoes as a unit");
        eval_str("(undo)", &env, &ctx).expect("undo again");
        assert_eq!(text_of(&ctx), "", "and then the yank");
    }

    /// After walking back into the ring, a fresh kill must be what the next
    /// yank produces -- not wherever `yank-pop` happened to leave the read
    /// position.
    #[test]
    fn a_new_kill_resets_where_yank_reads_from() {
        let (ctx, env) = editor_with("");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");
        eval_str("(kill-new \"first\") (kill-new \"second\")", &env, &ctx).expect("two kills");

        press(&ctx, &env, KeyCode::Char('y'), ctrl());
        press(&ctx, &env, KeyCode::Char('y'), alt());
        assert_eq!(text_of(&ctx), "first", "walked one step back");

        eval_str("(clear-buffer) (kill-new \"third\")", &env, &ctx).expect("a new kill");
        assert_eq!(
            eval_str("(yank)", &env, &ctx).expect("yank"),
            LispExp::string("third".into()),
            "the newest kill is what yank gives, not the entry yank-pop left \
             the ring pointing at"
        );
    }

    /// `yank-pop` removes the text a yank just inserted. Once another command
    /// has run there is nothing it could safely take back out, even though the
    /// editor still remembers where the last yank was.
    ///
    /// Pressed rather than evaluated: "was the previous command a yank?" is
    /// answered from state that `handle_key_event` rolls over between key
    /// events, so calling the primitive directly would be asking the question
    /// outside the machinery that answers it.
    #[test]
    fn yank_pop_refuses_once_another_command_has_run() {
        let (ctx, env) = editor_with("");
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("common-keymaps.lisp must load");
        eval_str("(kill-new \"first\") (kill-new \"second\")", &env, &ctx).expect("two kills");

        press(&ctx, &env, KeyCode::Char('y'), ctrl());
        assert_eq!(text_of(&ctx), "second");
        press(&ctx, &env, KeyCode::Char('b'), ctrl());
        press(&ctx, &env, KeyCode::Char('y'), alt());

        assert_eq!(
            text_of(&ctx),
            "second",
            "a yank-pop that is not following a yank must leave the text \
             alone rather than delete whatever now sits where the yank was"
        );
        assert_eq!(
            eval_str("(current-kill)", &env, &ctx).expect("current-kill"),
            LispExp::string("second".into()),
            "and it must not have walked the ring on either"
        );
    }

    #[test]
    fn the_ring_drops_its_oldest_entries_past_the_limit() {
        let (ctx, env) = editor_with("");
        eval_str("(set-kill-ring-max 2)", &env, &ctx).expect("set the limit");
        for word in ["one", "two", "three"] {
            eval_str(&format!("(kill-new \"{word}\")"), &env, &ctx).expect("kill");
        }

        assert_eq!(ctx.kill_ring_len(), 2);
        assert_eq!(
            eval_str("(current-kill)", &env, &ctx).expect("current"),
            LispExp::string("three".into())
        );
        assert_eq!(
            eval_str("(current-kill 1)", &env, &ctx).expect("one back"),
            LispExp::string("two".into())
        );
        assert_eq!(
            eval_str("(current-kill 2)", &env, &ctx).expect("two back"),
            LispExp::nil(),
            "the oldest was dropped"
        );
    }

    /// Killing nothing should not push what you were about to yank one step
    /// further away.
    #[test]
    fn killing_an_empty_range_does_not_take_a_slot() {
        let (ctx, env) = editor_with("");
        eval_str("(kill-new \"kept\")", &env, &ctx).expect("a kill");
        eval_str("(set-mark) (kill-region)", &env, &ctx).expect("kill an empty region");

        assert_eq!(ctx.kill_ring_len(), 1);
        assert_eq!(
            eval_str("(current-kill)", &env, &ctx).expect("current"),
            LispExp::string("kept".into())
        );
    }

    #[test]
    fn a_kill_in_one_buffer_can_be_yanked_into_another() {
        let (ctx, env) = editor_with("shared text");
        eval_str(
            "(set-mark) (forward-word) (kill-region) \
             (buffer-create \"other\") (switch-to-buffer \"other\") (yank)",
            &env,
            &ctx,
        )
        .expect("kill here, yank there");

        assert_eq!(
            ctx.with_buffer("other", |b| b.text.to_string())
                .expect("other"),
            "shared",
            "the ring belongs to the editor, not to a buffer"
        );
    }

    #[test]
    fn kill_region_without_a_region_says_so() {
        let (ctx, env) = editor_with("text");
        assert!(
            eval_str("(kill-region)", &env, &ctx).is_err(),
            "a kill with no region should report it rather than silently doing \
             nothing"
        );
        assert_eq!(text_of(&ctx), "text");
    }
}
