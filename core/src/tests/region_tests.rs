//! The mark, the region, and the kill ring.
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

    fn point_1d(ctx: &Ctx) -> usize {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .cursor_pos_1d()
    }

    fn mark_is_active(ctx: &Ctx) -> bool {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .mark
            .is_some_and(|mark| mark.active)
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

    fn t() -> LispExp<Ctx> {
        LispExp::symbol("t".into())
    }

    // ---------------- the mark ----------------

    #[test]
    fn the_region_runs_between_mark_and_point_whichever_way_round() {
        let (ctx, env) = editor_with("alpha beta gamma");

        eval_str(
            "(end-of-buffer) (set-mark) (beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("mark at the end, point at the start");
        assert_eq!(
            eval_str("(region-beginning)", &env, &ctx).expect("beginning"),
            LispExp::number(0.0)
        );
        assert_eq!(
            eval_str("(region-end)", &env, &ctx).expect("end"),
            LispExp::number(16.0),
            "the region is ordered, not point-first"
        );
    }

    #[test]
    fn there_is_no_region_until_the_mark_is_set() {
        let (ctx, env) = editor_with("text");
        assert_eq!(
            eval_str("(use-region-p)", &env, &ctx).expect("use-region-p"),
            LispExp::nil()
        );
        assert!(
            eval_str("(region-beginning)", &env, &ctx).is_err(),
            "asking for a region there is no way to have should say so, not \
             quietly answer 0"
        );

        eval_str("(set-mark)", &env, &ctx).expect("set-mark");
        assert_eq!(
            eval_str("(use-region-p)", &env, &ctx).expect("use-region-p"),
            t()
        );
    }

    /// The rule that keeps an active region honest: it can never have had an
    /// edit under it, so its offsets never need adjusting for one.
    #[test]
    fn any_edit_deactivates_the_mark() {
        for edit in [
            "(self-insert \"x\")",
            "(insert-newline)",
            "(delete-char)",
            "(kill-word)",
            "(clear-buffer)",
        ] {
            let (ctx, env) = editor_with("alpha beta");
            // Point left mid-buffer on purpose: `delete-char` at the end of
            // the buffer deletes nothing, and a command that made no edit
            // would not be testing the rule.
            eval_str("(set-mark) (forward-word)", &env, &ctx).expect("a region");
            assert!(mark_is_active(&ctx), "{edit}: the region starts active");

            eval_str(edit, &env, &ctx).unwrap_or_else(|e| panic!("{edit}: {e:?}"));
            assert!(
                !mark_is_active(&ctx),
                "{edit} changed the text, so the region should be over"
            );
        }
    }

    #[test]
    fn moving_point_keeps_the_region_and_grows_it() {
        let (ctx, env) = editor_with("alpha beta gamma");
        eval_str("(set-mark) (forward-word)", &env, &ctx).expect("select a word");
        assert_eq!(
            eval_str("(region-end)", &env, &ctx).expect("end"),
            LispExp::number(5.0)
        );

        eval_str("(forward-word)", &env, &ctx).expect("select another");
        assert!(mark_is_active(&ctx), "movement does not end a selection");
        assert_eq!(
            eval_str("(region-end)", &env, &ctx).expect("end"),
            LispExp::number(10.0),
            "the region should have grown with point"
        );
    }

    /// A deactivated mark is a place to go back to, not a forgotten one.
    #[test]
    fn a_deactivated_mark_keeps_its_position() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(
            "(end-of-buffer) (set-mark) (beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("mark");
        eval_str("(deactivate-mark)", &env, &ctx).expect("deactivate");

        assert_eq!(
            eval_str("(use-region-p)", &env, &ctx).expect("use-region-p"),
            LispExp::nil(),
            "there is no region any more"
        );
        assert_eq!(
            eval_str("(mark)", &env, &ctx).expect("mark"),
            LispExp::number(10.0),
            "but the mark itself is still there"
        );
    }

    #[test]
    fn exchange_point_and_mark_swaps_them_and_reactivates() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(
            "(end-of-buffer) (set-mark) (beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("mark");
        eval_str("(deactivate-mark)", &env, &ctx).expect("deactivate");

        eval_str("(exchange-point-and-mark)", &env, &ctx).expect("exchange");
        assert_eq!(point_1d(&ctx), 10, "point goes to where the mark was");
        assert_eq!(
            eval_str("(mark)", &env, &ctx).expect("mark"),
            LispExp::number(0.0),
            "and the mark to where point was"
        );
        assert!(
            mark_is_active(&ctx),
            "going back to the mark makes the region live again"
        );
    }

    /// Undo applies its changes below the editing layer -- that is what stops
    /// it recording itself as new history -- so it is the one thing that can
    /// shorten the text under a mark that is still active. The region has to
    /// stay inside the buffer anyway.
    #[test]
    fn a_region_is_clamped_when_undo_shortens_the_text_under_it() {
        let (ctx, env) = editor_with("");
        eval_str("(self-insert \"a\") (self-insert \"b\")", &env, &ctx).expect("type");
        eval_str(
            "(end-of-buffer) (set-mark) (beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("mark");
        assert_eq!(
            eval_str("(region-end)", &env, &ctx).expect("end"),
            LispExp::number(2.0)
        );

        eval_str("(undo)", &env, &ctx).expect("undo the typing");
        assert_eq!(text_of(&ctx), "");
        assert!(
            mark_is_active(&ctx),
            "undo does not go through the editing layer, so it leaves the \
             region active -- which is exactly why the region has to be clamped"
        );
        assert_eq!(
            eval_str("(region-end)", &env, &ctx).expect("end"),
            LispExp::number(0.0),
            "the region must not claim to reach past the end of the buffer"
        );
    }

    /// An inactive mark is never adjusted for edits under it, so it can end up
    /// past the end of the buffer. Going back to it must land somewhere real
    /// rather than trusting a stale offset.
    #[test]
    fn a_stale_mark_is_clamped_when_going_back_to_it() {
        let (ctx, env) = editor_with("alpha beta gamma");
        eval_str(
            "(end-of-buffer) (set-mark) (beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("mark");
        eval_str("(clear-buffer)", &env, &ctx).expect("empty the buffer");

        assert_eq!(
            eval_str("(exchange-point-and-mark)", &env, &ctx).expect("exchange must not panic"),
            LispExp::number(0.0),
            "the mark still says 16, but there is nowhere past 0 to go"
        );
        assert_eq!(point_1d(&ctx), 0);
    }

    #[test]
    fn mark_whole_buffer_selects_everything() {
        let (ctx, env) = editor_with("alpha\nbeta\ngamma");
        eval_str("(mark-whole-buffer)", &env, &ctx).expect("mark-whole-buffer");
        assert_eq!(
            eval_str("(region-beginning)", &env, &ctx).expect("beginning"),
            LispExp::number(0.0)
        );
        assert_eq!(
            eval_str("(region-end)", &env, &ctx).expect("end"),
            LispExp::number(16.0)
        );
    }

    // ---------------- the kill ring ----------------

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
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
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
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
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
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
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
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
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
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
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
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
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
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
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
            ctx.get_buffer("other")
                .expect("other")
                .read()
                .unwrap()
                .text
                .to_string(),
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

    // ---------------- highlighting ----------------

    /// The spans the renderer draws, for the window showing *scratch*.
    fn highlights(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<crate::ui::Highlight> {
        ctx.snapshot(env, 80, 24)
            .views
            .into_iter()
            .find(|v| v.buffer_name == "*scratch*")
            .expect("*scratch* must be on screen")
            .highlights
    }

    #[test]
    fn there_is_nothing_to_highlight_without_an_active_region() {
        let (ctx, env) = editor_with("alpha beta");
        assert!(highlights(&ctx, &env).is_empty());

        eval_str("(set-mark) (forward-word) (deactivate-mark)", &env, &ctx).expect("then cancel");
        assert!(
            highlights(&ctx, &env).is_empty(),
            "a cancelled selection must stop being drawn"
        );
    }

    #[test]
    fn a_region_within_one_line_is_one_span() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str("(set-mark) (forward-word)", &env, &ctx).expect("select a word");

        let spans = highlights(&ctx, &env);
        assert_eq!(spans.len(), 1, "one line selected, one span");
        assert_eq!(
            (spans[0].row, spans[0].start_col, spans[0].end_col),
            (0, 0, 5)
        );
        assert_eq!(spans[0].face, crate::ui::Face::REGION);
    }

    /// A multi-line selection has to cover the line endings too, or it reads
    /// as a set of ragged fragments rather than as whole lines.
    #[test]
    fn a_region_across_lines_covers_each_line_to_its_end() {
        let (ctx, env) = editor_with("one\ntwo\nthree");
        eval_str(
            "(set-mark) (next-line) (next-line) (forward-char 2)",
            &env,
            &ctx,
        )
        .expect("select down into the third line");

        let spans = highlights(&ctx, &env);
        assert_eq!(spans.len(), 3, "three rows touched: {spans:?}");
        assert_eq!(
            (spans[0].row, spans[0].start_col, spans[0].end_col),
            (0, 0, 4),
            "the first row runs one past its last character, so the line \
             ending looks selected too"
        );
        assert_eq!(
            (spans[1].row, spans[1].start_col, spans[1].end_col),
            (1, 0, 4),
            "a line wholly inside the region is selected end to end"
        );
        assert_eq!(
            (spans[2].row, spans[2].start_col, spans[2].end_col),
            (2, 0, 2),
            "the last row stops where point is"
        );
    }

    /// A region ending exactly at the start of a line selects nothing on that
    /// line, and an empty span is not worth telling the renderer about.
    #[test]
    fn a_row_with_nothing_selected_on_it_produces_no_span() {
        let (ctx, env) = editor_with("one\ntwo\nthree");
        eval_str("(set-mark) (next-line) (next-line)", &env, &ctx)
            .expect("select down to the start of the third line");

        let spans = highlights(&ctx, &env);
        assert_eq!(
            spans.len(),
            2,
            "the third row has no selected characters on it: {spans:?}"
        );
    }

    #[test]
    fn a_region_selected_backwards_highlights_the_same_span() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(
            "(forward-word) (set-mark) (beginning-of-buffer)",
            &env,
            &ctx,
        )
        .expect("select backwards");

        let spans = highlights(&ctx, &env);
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start_col, spans[0].end_col), (0, 5));
    }

    /// Highlights for a window with an arbitrary scroll position, built
    /// directly.
    ///
    /// Driving this through a real window would mean provoking the auto-scroll
    /// that follows point, which decides the offsets for you; the point here is
    /// what the spans look like *given* an offset, so the offset is stated.
    fn highlights_scrolled(
        ctx: &Ctx,
        scroll_x: usize,
        scroll_y: usize,
        height: usize,
    ) -> Vec<crate::ui::Highlight> {
        let mut buffers = std::collections::HashMap::new();
        buffers.insert(
            "*scratch*".to_string(),
            ctx.get_buffer("*scratch*").expect("*scratch*"),
        );
        crate::ui::region_highlights(
            &crate::ui::Window {
                id: 0,
                buffer_name: "*scratch*".into(),
                scroll_x,
                scroll_y,
            },
            &crate::ui::Rect {
                x: 0,
                y: 0,
                width: 20,
                height,
            },
            &buffers,
        )
    }

    /// Rows come back relative to the window, so a region scrolled up the
    /// screen is drawn where it now appears rather than where it is in the
    /// file.
    #[test]
    fn a_vertically_scrolled_region_reports_rows_relative_to_the_window() {
        let (ctx, env) = editor_with("one\ntwo\nthree\nfour\nfive");
        eval_str("(goto-line 4) (set-mark) (end-of-line)", &env, &ctx)
            .expect("select part of the fourth line");

        let spans = highlights_scrolled(&ctx, 0, 3, 10);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            spans[0].row, 0,
            "the fourth line is the first one on screen when three are scrolled \
             past: {spans:?}"
        );
    }

    /// Columns are shifted by the horizontal scroll too, and a span that has
    /// scrolled entirely off the left is not reported at all.
    #[test]
    fn a_horizontally_scrolled_region_reports_shifted_columns() {
        let (ctx, env) = editor_with("abcdefghij");
        eval_str("(forward-char 2) (set-mark) (forward-char 5)", &env, &ctx)
            .expect("select columns 2..7");

        let spans = highlights_scrolled(&ctx, 2, 0, 10);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            (spans[0].start_col, spans[0].end_col),
            (0, 5),
            "two columns scrolled past shifts the span two to the left: {spans:?}"
        );

        assert!(
            highlights_scrolled(&ctx, 8, 0, 10).is_empty(),
            "a selection scrolled off the left edge has nothing left to draw"
        );
    }

    #[test]
    fn a_region_scrolled_out_of_view_is_not_reported() {
        let (ctx, env) = editor_with("one\ntwo\nthree\nfour\nfive");
        eval_str("(set-mark) (end-of-line)", &env, &ctx).expect("select the first line");

        assert!(
            highlights_scrolled(&ctx, 0, 3, 10).is_empty(),
            "a selection above the visible rows costs nothing to not draw"
        );
    }

    // ---------------- bindings ----------------

    #[test]
    fn the_region_and_kill_keys_are_bound() {
        let (ctx, env) = editor_with("alpha beta");
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("common-keymaps.lisp must load");

        // C-<space> is the one whose key name could silently fail to parse.
        press(&ctx, &env, KeyCode::Char(' '), ctrl());
        assert!(mark_is_active(&ctx), "C-<space> should set the mark");

        eval_str("(forward-word)", &env, &ctx).expect("select a word");
        press(&ctx, &env, KeyCode::Char('w'), ctrl());
        assert_eq!(text_of(&ctx), " beta", "C-w should kill the region");

        press(&ctx, &env, KeyCode::Char('y'), ctrl());
        assert_eq!(text_of(&ctx), "alpha beta", "C-y should yank it back");
    }
}
