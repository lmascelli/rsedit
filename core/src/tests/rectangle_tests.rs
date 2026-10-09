//! Blocks of text: what a pair of columns covers, and what each command does
//! to it.
//!
//! # What is worth testing here
//!
//! Almost all of it is the ragged case. A rectangle over lines that are all
//! long enough is arithmetic nobody gets wrong; a rectangle over real text is
//! a pair of columns applied to lines that stop in different places, and
//! every command has its own right answer about what to do with a line that
//! does not reach:
//!
//! - taking text out takes what is there and leaves the line's length alone;
//! - putting text in pads the line out first, because a column that does not
//!   exist yet is exactly what it was asked to fill;
//! - except `clear-rectangle`, which pads nothing: there was nothing there to
//!   blank, and filling the line with spaces would lengthen it in order to
//!   erase something that was never written.
//!
//! The other silent one is the order edits are applied in. A rectangle
//! command changes several lines, and a change to one line moves every offset
//! after it -- so a command that walks top-down corrupts every line but the
//! first, and does it in a way that looks like a plausible rectangle.
#[cfg(test)]
mod tests {
    use crate::buffer::BufferTrait;
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn try_run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        try_run(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|buf| buf.text.to_string())
    }

    /// A buffer holding LINES, with the mark at (MARK-LINE, MARK-COL) and
    /// point at (POINT-LINE, POINT-COL). Lines and columns count from 0 here,
    /// because this is the geometry's own counting.
    fn marked(
        lines: &[&str],
        mark: (usize, usize),
        point: (usize, usize),
        env: &Arc<Env<Ctx>>,
        ctx: &Ctx,
    ) {
        let body = lines.join("\n");
        ctx.with_current_buffer_mut(|buf| {
            buf.text = GapBuffer::from(body.as_str());
            let at = buf.text.cursor_2d_to_1d(mark.0, mark.1);
            buf.text.cursor_move(mark.0, mark.1);
            buf.mark = Some(crate::buffer::Mark::new(at));
            buf.text.cursor_move(point.0, point.1);
        });
        let _ = (env, ctx);
    }

    fn lines_of(exp: &LispExp<Ctx>) -> Vec<String> {
        exp.iter()
            .map(|item| match item {
                LispExp::String(s) => (*s).to_string(),
                other => panic!("expected a string, got {other:?}"),
            })
            .collect()
    }

    // ----------------------------------------------------------------
    // The keys
    // ----------------------------------------------------------------

    #[test]
    fn every_rectangle_key_reaches_its_command() {
        // The bindings are in a `.lisp` file and the commands are in Rust, so
        // nothing but this checks that the two agree -- a key written with
        // the wrong name for the space bar, or a three-key sequence the
        // parser reads differently from how it was meant, is silent until
        // somebody presses it.
        let (ctx, env) = editor();
        run(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx);
        for (keys, command) in [
            ("C-x <space>", "rectangle-mark-mode"),
            ("C-x r k", "kill-rectangle"),
            ("C-x r d", "delete-rectangle"),
            ("C-x r M-w", "copy-rectangle-as-kill"),
            ("C-x r y", "yank-rectangle"),
            ("C-x r o", "open-rectangle"),
            ("C-x r c", "clear-rectangle"),
            ("C-x r t", "string-rectangle"),
        ] {
            // `key-binding` answers (KEYS TARGET SOURCE), and the target of
            // a command binding is the call it would make.
            assert_eq!(
                run(
                    &format!(r#"(car (nth 1 (key-binding "{keys}")))"#),
                    &env,
                    &ctx
                ),
                LispExp::symbol(command.into()),
                "{keys} should run {command}"
            );
        }
    }

    // ----------------------------------------------------------------
    // What a pair of columns covers
    // ----------------------------------------------------------------

    #[test]
    fn a_rectangle_is_the_columns_the_lines_have_in_common() {
        // Not the region: the region from the middle of one line to the
        // middle of a later one takes in whole lines on the way, and this
        // takes the two columns from each.
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl", "mnopqr"], (0, 2), (2, 4), &env, &ctx);
        assert_eq!(
            lines_of(&run(
                "(copy-rectangle-as-kill) (killed-rectangle)",
                &env,
                &ctx
            )),
            vec!["cd", "ij", "op"]
        );
    }

    #[test]
    fn either_corner_may_be_either_end() {
        // Dragged up and to the left is the same block as dragged down and to
        // the right, and no command should have to know which way round the
        // user made it.
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl", "mnopqr"], (2, 4), (0, 2), &env, &ctx);
        assert_eq!(
            lines_of(&run(
                "(copy-rectangle-as-kill) (killed-rectangle)",
                &env,
                &ctx
            )),
            vec!["cd", "ij", "op"]
        );
    }

    #[test]
    fn the_bounds_are_reported_with_lines_from_one_and_columns_from_zero() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl", "mnopqr"], (0, 2), (2, 4), &env, &ctx);
        assert_eq!(
            run("(rectangle-bounds)", &env, &ctx)
                .iter()
                .collect::<Vec<_>>(),
            vec![
                LispExp::number(1.0),
                LispExp::number(3.0),
                LispExp::number(2.0),
                LispExp::number(4.0),
            ]
        );
    }

    #[test]
    fn with_no_mark_there_is_no_rectangle() {
        let (ctx, env) = editor();
        run(r#"(insert "abc")"#, &env, &ctx);
        assert!(run("(rectangle-bounds)", &env, &ctx).is_nil());
        assert!(
            try_run("(kill-rectangle)", &env, &ctx).is_err(),
            "and a command says so rather than quietly doing nothing"
        );
    }

    // ----------------------------------------------------------------
    // Taking one out
    // ----------------------------------------------------------------

    #[test]
    fn killing_a_rectangle_leaves_the_rest_of_each_line() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl", "mnopqr"], (0, 2), (2, 4), &env, &ctx);
        assert_eq!(run("(kill-rectangle)", &env, &ctx), LispExp::number(3.0));
        assert_eq!(text(&ctx), "abef\nghkl\nmnqr");
    }

    #[test]
    fn the_lines_are_edited_from_the_bottom_up() {
        // Said as its own test because getting it wrong produces something
        // that still looks like a rectangle was cut. Editing the first line
        // moves every offset after it, so a top-down walk takes the second
        // line's block from two characters to the left, the third from four,
        // and so on -- the damage grows down the page and the top of the
        // result is correct.
        let (ctx, env) = editor();
        marked(
            &["aaaaXXaaaa", "bbbbXXbbbb", "ccccXXcccc", "ddddXXdddd"],
            (0, 4),
            (3, 6),
            &env,
            &ctx,
        );
        run("(kill-rectangle)", &env, &ctx);
        assert_eq!(
            text(&ctx),
            "aaaaaaaa\nbbbbbbbb\ncccccccc\ndddddddd",
            "every line lost its own X's, not the ones the line above shifted"
        );
    }

    #[test]
    fn a_line_too_short_takes_part_with_nothing() {
        // And still takes part: leaving it out would change the block's
        // height depending on the text in it, and a block yanked back
        // elsewhere would have its lines out of step.
        let (ctx, env) = editor();
        marked(&["abcdef", "gh", "mnopqr"], (0, 3), (2, 5), &env, &ctx);
        assert_eq!(
            lines_of(&run(
                "(copy-rectangle-as-kill) (killed-rectangle)",
                &env,
                &ctx
            )),
            vec!["de", "", "pq"],
            "three lines, one of them empty"
        );
        run("(kill-rectangle)", &env, &ctx);
        assert_eq!(
            text(&ctx),
            "abcf\ngh\nmnor",
            "and the short line is untouched"
        );
    }

    #[test]
    fn deleting_a_rectangle_saves_nothing() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 1), (1, 3), &env, &ctx);
        run(r#"(copy-rectangle-as-kill)"#, &env, &ctx);
        let saved = lines_of(&run("(killed-rectangle)", &env, &ctx));
        marked(&["abcdef", "ghijkl"], (0, 3), (1, 5), &env, &ctx);
        run("(delete-rectangle)", &env, &ctx);
        assert_eq!(text(&ctx), "abcf\nghil");
        assert_eq!(
            lines_of(&run("(killed-rectangle)", &env, &ctx)),
            saved,
            "what was saved before is still what is saved"
        );
    }

    #[test]
    fn a_killed_rectangle_does_not_go_on_the_kill_ring() {
        // The whole reason it has a store of its own: `C-y' after a rectangle
        // kill puts back whatever was killed before it, rather than a block.
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 0), (0, 0), &env, &ctx);
        run(r#"(kill-new "on the ring")"#, &env, &ctx);
        marked(&["abcdef", "ghijkl"], (0, 2), (1, 4), &env, &ctx);
        run("(kill-rectangle)", &env, &ctx);
        run("(end-of-buffer) (yank)", &env, &ctx);
        assert!(
            text(&ctx).ends_with("on the ring"),
            "C-y is untouched by a rectangle kill: {:?}",
            text(&ctx)
        );
    }

    // ----------------------------------------------------------------
    // Putting one in
    // ----------------------------------------------------------------

    #[test]
    fn yanking_a_rectangle_pushes_what_is_there_to_the_right() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 2), (1, 4), &env, &ctx);
        run("(kill-rectangle)", &env, &ctx);
        assert_eq!(text(&ctx), "abef\nghkl");
        run(
            "(goto-line 1) (beginning-of-line) (yank-rectangle)",
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "cdabef\nijghkl");
    }

    #[test]
    fn yanking_at_the_end_of_a_buffer_gives_it_the_lines_it_needs() {
        // A block is a block: arriving cut off by where the text happens to
        // stop would make where you yank it change what you get.
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl", "mnopqr"], (0, 0), (2, 2), &env, &ctx);
        run("(copy-rectangle-as-kill)", &env, &ctx);
        run("(end-of-buffer) (yank-rectangle)", &env, &ctx);
        // Every line of the block lands at the column point was in, the new
        // lines included -- a block yanked at column six is a block at column
        // six, not one that straightens itself out against the left margin.
        assert_eq!(text(&ctx), "abcdef\nghijkl\nmnopqrab\n      gh\n      mn");
    }

    #[test]
    fn yanking_onto_short_lines_pads_them_out() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 4), (1, 6), &env, &ctx);
        run("(copy-rectangle-as-kill)", &env, &ctx);
        marked(&["xy", "z"], (0, 0), (0, 0), &env, &ctx);
        run("(goto-line 1) (end-of-line) (yank-rectangle)", &env, &ctx);
        assert_eq!(text(&ctx), "xyef\nz kl");
    }

    #[test]
    fn opening_a_rectangle_makes_room() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 2), (1, 4), &env, &ctx);
        assert_eq!(run("(open-rectangle)", &env, &ctx), LispExp::number(2.0));
        assert_eq!(text(&ctx), "ab  cdef\ngh  ijkl");
    }

    #[test]
    fn opening_a_rectangle_reaches_lines_that_stop_short_of_it() {
        // The ragged case, and the one that makes this worth having: a column
        // of text is moved over even where the lines beside it do not get
        // that far.
        let (ctx, env) = editor();
        marked(&["abcdef", "gh", "mnopqr"], (0, 4), (2, 6), &env, &ctx);
        run("(open-rectangle)", &env, &ctx);
        assert_eq!(text(&ctx), "abcd  ef\ngh    \nmnop  qr");
    }

    #[test]
    fn clearing_a_rectangle_keeps_the_columns_lined_up() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 2), (1, 4), &env, &ctx);
        assert_eq!(run("(clear-rectangle)", &env, &ctx), LispExp::number(2.0));
        assert_eq!(text(&ctx), "ab  ef\ngh  kl");
    }

    #[test]
    fn clearing_does_not_lengthen_a_line_that_stops_short() {
        // The one command that does not pad. There was nothing there to
        // blank, and spaces added to erase text that was never written are
        // trailing whitespace the user did not ask for.
        let (ctx, env) = editor();
        marked(&["abcdef", "gh", "mnopqr"], (0, 3), (2, 5), &env, &ctx);
        run("(clear-rectangle)", &env, &ctx);
        assert_eq!(text(&ctx), "abc  f\ngh\nmno  r");
    }

    // ----------------------------------------------------------------
    // Replacing one
    // ----------------------------------------------------------------

    #[test]
    fn string_rectangle_replaces_the_block_on_every_line() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 2), (1, 4), &env, &ctx);
        assert_eq!(
            run(r#"(string-rectangle "--")"#, &env, &ctx),
            LispExp::number(2.0)
        );
        assert_eq!(text(&ctx), "ab--ef\ngh--kl");
    }

    #[test]
    fn the_replacement_need_not_be_the_same_width() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 2), (1, 4), &env, &ctx);
        run(r#"(string-rectangle "LONGER")"#, &env, &ctx);
        assert_eq!(text(&ctx), "abLONGERef\nghLONGERkl");
    }

    #[test]
    fn a_rectangle_with_no_width_is_how_a_passage_is_prefixed() {
        // `C-x SPC' and then straight down: nothing is replaced, and the
        // string goes in at that column on every line. The ordinary way to
        // comment out or indent a run of lines by hand.
        let (ctx, env) = editor();
        marked(&["one", "two", "three"], (0, 0), (2, 0), &env, &ctx);
        run(r#"(string-rectangle "> ")"#, &env, &ctx);
        assert_eq!(text(&ctx), "> one\n> two\n> three");
    }

    #[test]
    fn string_rectangle_pads_a_line_that_stops_short_of_the_column() {
        let (ctx, env) = editor();
        marked(&["abcdef", "gh", "mnopqr"], (0, 4), (2, 4), &env, &ctx);
        run(r#"(string-rectangle "|")"#, &env, &ctx);
        assert_eq!(text(&ctx), "abcd|ef\ngh  |\nmnop|qr");
    }

    // ----------------------------------------------------------------
    // Seeing one
    // ----------------------------------------------------------------

    #[test]
    fn rectangle_mark_mode_turns_on_and_off_without_losing_the_region() {
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 2), (1, 4), &env, &ctx);
        assert!(!run("(rectangle-mark-mode)", &env, &ctx).is_nil(), "on");
        assert!(ctx.with_current_buffer(|buf| buf.mark.is_some_and(|mark| mark.rectangle)));
        assert!(
            run("(rectangle-mark-mode)", &env, &ctx).is_nil(),
            "off again"
        );
        assert!(
            ctx.with_current_buffer(|buf| buf.mark.is_some_and(|mark| mark.active)),
            "and the selection is still there, which is what `C-x SPC' twice should leave"
        );
    }

    #[test]
    fn rectangle_mark_mode_sets_a_mark_when_there_is_none() {
        let (ctx, env) = editor();
        run(r#"(insert "abcdef")"#, &env, &ctx);
        assert!(!run("(rectangle-mark-mode)", &env, &ctx).is_nil());
        assert!(
            ctx.with_current_buffer(|buf| buf
                .mark
                .is_some_and(|mark| mark.active && mark.rectangle)),
            "so `C-x SPC' can begin a selection as well as change one"
        );
    }

    #[test]
    fn an_edit_ends_rectangle_mode_with_the_region() {
        // Which is the reason the flag lives on the mark: there is no second
        // thing to remember to turn off, and no way to be left in a mode
        // whose selection has gone.
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl"], (0, 2), (1, 4), &env, &ctx);
        run("(rectangle-mark-mode)", &env, &ctx);
        run(r#"(insert "x")"#, &env, &ctx);
        assert!(
            ctx.with_current_buffer(|buf| !buf.mark.is_some_and(|mark| mark.active)),
            "the mark is deactivated, so the rectangle mode is gone with it"
        );
    }

    #[test]
    fn the_region_is_drawn_column_to_column_in_rectangle_mode() {
        // What the mode is for. Without it the block is selected and the
        // screen shows a ragged run -- which is not the text the commands
        // would take, and is the whole difficulty of cutting one blind.
        let (ctx, env) = editor();
        marked(&["abcdef", "ghijkl", "mnopqr"], (0, 2), (2, 4), &env, &ctx);

        let ragged = highlight_columns(&ctx, &env);
        assert_eq!(
            ragged,
            vec![(2, 7), (0, 7), (0, 4)],
            "an ordinary region: from the corner to the end, whole lines, then to the corner"
        );

        run("(rectangle-mark-mode)", &env, &ctx);
        assert_eq!(
            highlight_columns(&ctx, &env),
            vec![(2, 4), (2, 4), (2, 4)],
            "and now the same two columns on every line"
        );
    }

    #[test]
    fn a_line_too_short_is_drawn_with_no_highlight_at_all() {
        let (ctx, env) = editor();
        marked(&["abcdef", "gh", "mnopqr"], (0, 3), (2, 5), &env, &ctx);
        run("(rectangle-mark-mode)", &env, &ctx);
        assert_eq!(
            highlight_columns(&ctx, &env),
            vec![(3, 5), (2, 2), (3, 5)],
            "the short line's edges collapse to its end rather than hanging past it"
        );
    }

    /// The region highlight the renderer would draw, as (start, end) columns
    /// per row.
    ///
    /// A collapsed span is dropped before it reaches the renderer, so it is
    /// reported here as the pair it collapsed to rather than being missing --
    /// which would make a short line indistinguishable from a line that is
    /// not in the rectangle at all.
    fn highlight_columns(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<(usize, usize)> {
        let frame = ctx.snapshot(env, 80, 24);
        let view = frame.views.first().expect("a window");
        let rows = ctx.with_current_buffer(|buf| buf.text.line_count());
        (0..rows)
            .map(|row| {
                view.highlights
                    .iter()
                    .find(|highlight| highlight.row == row)
                    .map(|highlight| (highlight.start_col, highlight.end_col))
                    .unwrap_or_else(|| collapsed(ctx, row))
            })
            .collect()
    }

    /// Where a line's edges collapsed to, for a line the highlight left out.
    fn collapsed(ctx: &Ctx, row: usize) -> (usize, usize) {
        let width = ctx.with_current_buffer(|buf| {
            buf.text
                .get_lines(row, row + 1)
                .first()
                .map(|line| line.chars().count())
                .unwrap_or(0)
        });
        (width, width)
    }
}
