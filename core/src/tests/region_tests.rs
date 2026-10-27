//! The mark and the region: where they are, what an edit does to them, and
//! how a window draws the region.
//!
//! Design: `doc/architettura-editor.typ, <mark-overlay>`.
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

    fn point_1d(ctx: &Ctx) -> usize {
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos_1d())
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
        // The editor's own table, rather than one built for the occasion: the
        // renderer takes a `Buffers`, and a second one assembled here would be
        // a different set of buffers than the editor has.
        let window = crate::ui::Window {
            scroll_x,
            scroll_y,
            ..crate::ui::Window::new(0, "*scratch*")
        };
        let rect = crate::ui::Rect {
            x: 0,
            y: 0,
            width: 20,
            height,
        };
        // Composed and then placed, which is the pair of calls the renderer
        // makes: `region_highlights` answers in buffer columns and the layout
        // puts them on the rows. Doing only the first here would test a
        // coordinate system nothing draws in.
        ctx.buffers(|buffers| {
            let layout = crate::ui::compose_layout(&window, &rect, buffers);
            crate::ui::layout::place(
                &layout,
                crate::ui::region_highlights(&window, &rect, buffers),
                scroll_x,
                rect.width,
            )
        })
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
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
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
