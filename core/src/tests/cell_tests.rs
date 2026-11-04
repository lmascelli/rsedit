//! Cells against columns: a Tab drawn to the next stop, an ideogram across
//! two cells, an accent over the letter before it. Checked where the two can
//! disagree: the text a window draws, where the cursor goes, what a
//! highlight covers, where a click lands, and the width a mode gives a Tab.
//!
//! Design: `doc/architettura-editor.typ, <righe>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, VirtualTextTable, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyModifiers, MouseButton, MouseEvent, MouseKind};
    use crate::ui::Face;
    use crate::ui::layout::Layout;
    use crate::ui::width::{cells, cut, fit};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// An editor showing TEXT in `*scratch*`, point at offset AT, the mouse
    /// on.
    fn editor(text: &str, at: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        run("(setq mouse-mode t)", &env, &ctx);
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            let (line, column) = b.text.cursor_1d_to_2d(at);
            b.text.cursor_move(line, column);
        });
        (ctx, env)
    }

    /// The first window's first row, as drawn, and where its cursor is.
    fn drawn(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> (String, Option<(usize, usize)>) {
        let frame = ctx.snapshot(env, W, H);
        let view = &frame.views[0];
        (view.lines[0].trim_end().to_string(), view.cursor_rel_pos)
    }

    fn click(ctx: &Ctx, env: &Arc<Env<Ctx>>, column: u16) -> usize {
        let _ = ctx.snapshot(env, W, H);
        ctx.handle_mouse_event(
            MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                column,
                row: 0,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos().1)
            .expect("*scratch*")
    }

    fn row(text: &str, tab_width: usize) -> Layout {
        Layout::compose(
            &GapBuffer::from(text),
            &VirtualTextTable::default(),
            0,
            1,
            tab_width,
        )
    }

    // ----------------------------------------------------------------
    // Measuring
    // ----------------------------------------------------------------

    #[test]
    fn a_wide_character_takes_two_cells_and_an_accent_none() {
        assert_eq!(cells("abc"), 3);
        assert_eq!(cells("日本"), 4);
        assert_eq!(cells("e\u{301}"), 1, "e and a combining acute");
        assert_eq!(
            cells("\u{1b}"),
            1,
            "a control character, drawn as a picture"
        );
    }

    /// A wide character cut by an edge leaves a space for each of its cells
    /// inside, so that what follows it stays where it was counted.
    #[test]
    fn cutting_through_a_wide_character_leaves_spaces() {
        assert_eq!(cut("日本x", 0, 4), "日本");
        assert_eq!(cut("日本x", 1, 3), " 本");
        assert_eq!(cut("日本x", 0, 3), "日 ");
        assert_eq!(
            cut("e\u{301}x", 0, 1),
            "e\u{301}",
            "the accent goes with its letter"
        );
        assert_eq!(fit("日", 4), "日  ");
    }

    // ----------------------------------------------------------------
    // The rows
    // ----------------------------------------------------------------

    #[test]
    fn a_tab_reaches_to_the_next_stop() {
        let layout = row("a\tb\tc", 4);
        let line = layout.row(0).expect("a row");
        assert_eq!(line.text, "a   b   c");
        assert_eq!(
            (0..6).map(|c| line.to_screen(c)).collect::<Vec<_>>(),
            vec![0, 1, 4, 5, 8, 9]
        );
        assert_eq!(
            (0..10).map(|s| line.to_buffer(s)).collect::<Vec<_>>(),
            vec![0, 1, 1, 1, 2, 3, 3, 3, 4, 5],
            "every cell of a Tab is the Tab's"
        );
        assert_eq!(row("\tx", 8).row(0).expect("a row").text, "        x");
    }

    #[test]
    fn a_wide_character_is_two_cells_of_the_row() {
        let layout = row("日本x", 4);
        let line = layout.row(0).expect("a row");
        assert_eq!(
            (0..4).map(|c| line.to_screen(c)).collect::<Vec<_>>(),
            vec![0, 2, 4, 5]
        );
        assert_eq!(
            (0..6).map(|s| line.to_buffer(s)).collect::<Vec<_>>(),
            vec![0, 0, 1, 1, 2, 3]
        );
        assert_eq!(layout.visible_text(1, 4), vec![" 本x".to_string()]);
    }

    // ----------------------------------------------------------------
    // The frame
    // ----------------------------------------------------------------

    /// The cursor after a Tab was drawn one cell after its start, where the
    /// terminal had drawn the text four cells along.
    #[test]
    fn the_cursor_after_a_tab_is_where_the_text_is() {
        let (ctx, env) = editor("\tx = 1", 1);
        let (text, cursor) = drawn(&ctx, &env);
        assert_eq!(text, "    x = 1");
        assert_eq!(cursor, Some((4, 0)));
    }

    #[test]
    fn the_cursor_after_a_wide_character_is_two_cells_on() {
        let (ctx, env) = editor("日本x", 2);
        let (text, cursor) = drawn(&ctx, &env);
        assert_eq!(text, "日本x");
        assert_eq!(cursor, Some((4, 0)));
    }

    #[test]
    fn a_highlight_covers_the_cells_of_what_it_covers() {
        let (ctx, env) = editor("日本x", 0);
        ctx.with_buffer_mut("*scratch*", |b| {
            b.mark = Some(crate::buffer::Mark::new(0));
            b.text.cursor_move(0, 2);
        });
        let frame = ctx.snapshot(&env, W, H);
        let region: Vec<_> = frame.views[0]
            .highlights
            .iter()
            .filter(|h| h.face == Face::REGION)
            .map(|h| (h.start_col, h.end_col))
            .collect();
        assert_eq!(region, vec![(0, 4)]);
    }

    /// The width is the mode's own, as indentation reads it, or the
    /// variable's.
    #[test]
    fn a_tab_is_as_wide_as_the_mode_says() {
        let (ctx, env) = editor("\tx", 0);
        run("(setq tab-width 8)", &env, &ctx);
        assert_eq!(drawn(&ctx, &env).0, "        x");
        run("(put (major-mode) 'tab-width 2)", &env, &ctx);
        assert_eq!(drawn(&ctx, &env).0, "  x");
        run("(put (major-mode) 'tab-width nil)", &env, &ctx);
        run("(setq tab-width \"wide\")", &env, &ctx);
        assert_eq!(drawn(&ctx, &env).0, "    x", "not a number: 4");
    }

    // ----------------------------------------------------------------
    // The mouse
    // ----------------------------------------------------------------

    #[test]
    fn a_click_on_any_cell_of_a_tab_lands_on_the_tab() {
        let (ctx, env) = editor("\tx", 0);
        assert_eq!(click(&ctx, &env, 2), 0);
        assert_eq!(click(&ctx, &env, 4), 1, "the x");
        assert_eq!(click(&ctx, &env, 5), 2, "past the end");
    }

    #[test]
    fn a_click_on_either_cell_of_a_wide_character_lands_on_it() {
        let (ctx, env) = editor("日本x", 0);
        assert_eq!(click(&ctx, &env, 1), 0);
        assert_eq!(click(&ctx, &env, 3), 1);
        assert_eq!(click(&ctx, &env, 4), 2);
    }

    #[test]
    fn a_click_follows_the_width_of_the_mode() {
        let (ctx, env) = editor("\tx", 0);
        run("(put (major-mode) 'tab-width 8)", &env, &ctx);
        assert_eq!(click(&ctx, &env, 6), 0, "still the Tab");
        assert_eq!(click(&ctx, &env, 8), 1, "the x");
    }
}
