//! Text shown in a window that is not in the buffer.
//!
//! # What is worth testing here
//!
//! The mapping, mostly. Everything the editor does with a column used to rest
//! on buffer column equalling screen column, and the whole of this feature is
//! that they can now differ. The symptom of getting it wrong is the cursor in
//! one place and the text in another -- visible, and hard to attribute to the
//! line that caused it, because the line that caused it is a hint somebody
//! else put there.
//!
//! So: both directions, at every interesting position, and the four things
//! that consume them -- what is drawn, where the cursor goes, where a
//! highlight lands, and which character a click is on.
//!
//! The other half is that none of it reaches the buffer. A preview that
//! modified the text and took it back out again would leave the file marked
//! unsaved and the undo history holding something nobody typed, and both are
//! invisible until much later.
#[cfg(test)]
mod tests {
    use crate::buffer::BufferTrait;
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, LispExp, Parser, eval};
    use crate::ui::layout::Layout;
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 40;
    const H: usize = 10;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn with_text(body: &str, ctx: &Ctx) {
        ctx.with_current_buffer_mut(|buf| {
            buf.text = GapBuffer::from(body);
            buf.text.cursor_move(0, 0);
        });
    }

    /// The rows the window would draw.
    fn rows(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<String> {
        ctx.snapshot(env, W, H)
            .views
            .first()
            .expect("a window")
            .lines
            .clone()
    }

    /// The layout the renderer composes, to ask about directly.
    fn layout(ctx: &Ctx) -> Layout {
        ctx.with_current_buffer(|buf| Layout::compose(&buf.text, &buf.virtual_text, 0, H))
    }

    // ----------------------------------------------------------------
    // What is drawn
    // ----------------------------------------------------------------

    #[test]
    fn virtual_text_appears_in_the_row_and_not_in_the_buffer() {
        let (ctx, env) = editor();
        with_text("let x = 1", &ctx);
        run(r#"(make-virtual-text 5 ": int")"#, &env, &ctx);
        assert_eq!(rows(&ctx, &env)[0], "let x: int = 1");
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.to_string()),
            "let x = 1",
            "the buffer is exactly what it was"
        );
    }

    #[test]
    fn previewing_marks_nothing_unsaved_and_records_no_undo() {
        // The reason this is not done by inserting the text and taking it out
        // again: both of those are invisible at the time and wrong much
        // later, when the file is saved or the undo history is walked.
        let (ctx, env) = editor();
        with_text("one\ntwo", &ctx);
        ctx.with_current_buffer_mut(|buf| buf.is_modified = false);
        run(
            r#"(make-virtual-text 0 "hint") (clear-virtual-text)"#,
            &env,
            &ctx,
        );
        assert!(
            !ctx.with_current_buffer(|buf| buf.is_modified),
            "nothing was modified"
        );
        run("(undo)", &env, &ctx);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.to_string()),
            "one\ntwo",
            "and there was nothing to undo"
        );
    }

    #[test]
    fn text_at_the_end_of_a_line_is_placed_by_its_newline() {
        let (ctx, env) = editor();
        with_text("one\ntwo", &ctx);
        run(r#"(make-virtual-text 3 "  <- here")"#, &env, &ctx);
        assert_eq!(rows(&ctx, &env)[0], "one  <- here");
        assert_eq!(rows(&ctx, &env)[1], "two", "and the next line is its own");
    }

    #[test]
    fn two_at_one_position_are_drawn_in_priority_order() {
        let (ctx, env) = editor();
        with_text("ab", &ctx);
        run(
            r#"(make-virtual-text 1 "[low]" nil 0 'a)
               (make-virtual-text 1 "[high]" nil 5 'b)"#,
            &env,
            &ctx,
        );
        assert_eq!(rows(&ctx, &env)[0], "a[high][low]b");
    }

    #[test]
    fn empty_text_makes_nothing() {
        let (ctx, env) = editor();
        with_text("ab", &ctx);
        assert!(run(r#"(make-virtual-text 1 "")"#, &env, &ctx).is_nil());
        assert_eq!(rows(&ctx, &env)[0], "ab");
    }

    // ----------------------------------------------------------------
    // The mapping, both ways
    // ----------------------------------------------------------------

    #[test]
    fn a_buffer_column_maps_past_the_text_in_front_of_it() {
        // The rule that keeps the cursor on real characters: text anchored at
        // a column comes *before* it, so the column itself is drawn after.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX")"#, &env, &ctx);
        let layout = layout(&ctx);
        let (_, row) = layout.row_of_line(0).expect("a row");
        assert_eq!(row.text, "abcXXdef");
        assert_eq!(
            (0..=6).map(|col| row.to_screen(col)).collect::<Vec<_>>(),
            vec![0, 1, 2, 5, 6, 7, 8],
            "columns before it are where they were; from it on, two further along"
        );
    }

    #[test]
    fn a_screen_column_inside_virtual_text_answers_what_it_sits_before() {
        // There is nowhere else for point to go: clicking a hint has to put
        // the cursor on a character, and the character it is in front of is
        // the one it is about.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX")"#, &env, &ctx);
        let layout = layout(&ctx);
        let (_, row) = layout.row_of_line(0).expect("a row");
        assert_eq!(
            (0..=8).map(|col| row.to_buffer(col)).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 3, 3, 4, 5, 6],
            "screen columns 3 and 4 are the hint, and answer the column it is at"
        );
    }

    #[test]
    fn the_two_directions_are_inverse_outside_the_virtual_text() {
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(
            r#"(make-virtual-text 3 "XX") (make-virtual-text 0 "Y")"#,
            &env,
            &ctx,
        );
        let layout = layout(&ctx);
        let (_, row) = layout.row_of_line(0).expect("a row");
        for column in 0..=6 {
            assert_eq!(
                row.to_buffer(row.to_screen(column)),
                column,
                "column {column} should survive the round trip"
            );
        }
    }

    #[test]
    fn a_column_past_the_end_of_the_line_maps_past_the_end_of_the_row() {
        // What a multi-line selection depends on: it marks a line's ending as
        // selected by running one column past the last character, and a
        // mapping that clamped to the text turned every one of those back
        // into a span that stopped at it.
        let (ctx, env) = editor();
        with_text("abc", &ctx);
        run(r#"(make-virtual-text 1 "ZZ")"#, &env, &ctx);
        let layout = layout(&ctx);
        let (_, row) = layout.row_of_line(0).expect("a row");
        assert_eq!(row.text, "aZZbc");
        assert_eq!(row.to_screen(3), 5, "the newline's column");
        assert_eq!(row.to_screen(4), 6, "and one past it");
    }

    #[test]
    fn a_row_with_nothing_on_it_is_its_line() {
        let (ctx, _env) = editor();
        with_text("abcdef", &ctx);
        let layout = layout(&ctx);
        let (_, row) = layout.row_of_line(0).expect("a row");
        assert_eq!(row.text, "abcdef");
        // Both directions are the identity, inside the text and past its
        // end: a row with nothing added to it is its line, and the mapping
        // has to cost nothing and change nothing in the case that is almost
        // every row of almost every frame.
        for column in 0..=8 {
            assert_eq!(row.to_screen(column), column);
            assert_eq!(row.to_buffer(column), column);
        }
    }

    // ----------------------------------------------------------------
    // What the mapping is for
    // ----------------------------------------------------------------

    #[test]
    fn the_cursor_is_drawn_past_the_text_in_front_of_it() {
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX")"#, &env, &ctx);
        run("(goto-char 3)", &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(
            frame.views.first().expect("a window").cursor_rel_pos,
            Some((5, 0)),
            "point is on `d', which is drawn at screen column five"
        );
    }

    #[test]
    fn a_highlight_lands_on_the_characters_it_covers() {
        // The three producers -- colouring, overlays and the region -- all
        // answer in buffer columns and all go through one placement, so this
        // checks the placement with the one that is easiest to ask for.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 1 "ZZ")"#, &env, &ctx);
        run("(goto-char 2) (set-mark 2) (goto-char 4)", &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        let highlight = frame
            .views
            .first()
            .expect("a window")
            .highlights
            .iter()
            .find(|highlight| highlight.face == crate::ui::Face::REGION)
            .expect("the region is highlighted");
        assert_eq!(
            (highlight.start_col, highlight.end_col),
            (4, 6),
            "buffer columns 2..4 are drawn at screen columns 4..6"
        );
    }

    #[test]
    fn a_click_past_a_hint_lands_on_the_character_under_the_pointer() {
        // End to end, through the real click path: what the pointer is over
        // is a screen cell, and point is a buffer offset, and between them is
        // the mapping. Driving it any shorter would check the mapping against
        // itself.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX")"#, &env, &ctx);
        // The frame has to have been laid out for there to be a window to
        // click in: a hit test answers in cells of the last frame.
        let frame = ctx.snapshot(&env, W, H);
        let rect = frame.views.first().expect("a window").rect;
        // The row reads `abcXXdef', so screen column six is `e'.
        click(&ctx, &env, (rect.x + 6) as u16, rect.y as u16);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()),
            4,
            "which is buffer offset four"
        );
    }

    #[test]
    fn a_click_on_a_hint_lands_on_what_it_is_about() {
        // There is nowhere else to put point: a hint is not a place in the
        // text, so clicking one has to choose a character, and the character
        // it sits in front of is the one it is describing.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX")"#, &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        let rect = frame.views.first().expect("a window").rect;
        // Screen column four is the second `X'.
        click(&ctx, &env, (rect.x + 4) as u16, rect.y as u16);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()),
            3,
            "the character the hint is in front of"
        );
    }

    #[test]
    fn dragging_across_a_hint_selects_the_characters_under_the_pointer() {
        // The drag path is a second caller of the mapping, and it reports
        // where the pointer is on every event rather than once -- so getting
        // it wrong is a selection that grows by the width of every hint it
        // crosses, which reads as the mouse being wrong rather than the hint.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 2 "XXX")"#, &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        let rect = frame.views.first().expect("a window").rect;
        // The row reads `abXXXcdef'. Down on `a', drag to screen column
        // seven, which is `d'.
        click(&ctx, &env, rect.x as u16, rect.y as u16);
        drag(&ctx, &env, (rect.x + 7) as u16, rect.y as u16);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()),
            4,
            "the selection reaches buffer offset four, not seven"
        );
    }

    #[test]
    fn typing_exactly_where_it_sits_pushes_it_along() {
        // At, not after. The alternative leaves the hint in front of whatever
        // was just typed rather than in front of what it was placed against,
        // which is the position it was describing.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX")"#, &env, &ctx);
        run(r#"(goto-char 3) (insert "..")"#, &env, &ctx);
        assert_eq!(
            rows(&ctx, &env)[0],
            "abc..XXdef",
            "still in front of `d', which is what it was placed in front of"
        );
    }

    #[test]
    fn virtual_text_is_drawn_in_its_own_face() {
        // Over the colouring of the row around it, not under: it is not part
        // of that text, and colouring it as a keyword because the words
        // beside it are keywords would be the editor claiming it is code.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX" 'keyword)"#, &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        let highlight = frame
            .views
            .first()
            .expect("a window")
            .highlights
            .iter()
            .find(|highlight| highlight.face == crate::ui::Face::intern("keyword"))
            .expect("the hint has a face of its own");
        assert_eq!(
            (highlight.start_col, highlight.end_col),
            (3, 5),
            "covering exactly the screen columns the hint occupies"
        );
    }

    /// A left drag to a cell of the last frame.
    fn drag(ctx: &Ctx, env: &Arc<Env<Ctx>>, column: u16, row: u16) {
        ctx.handle_mouse_event(
            crate::input::MouseEvent {
                kind: crate::input::MouseKind::Drag(crate::input::MouseButton::Left),
                column,
                row,
                modifiers: crate::input::KeyModifiers::default(),
            },
            env,
        );
    }

    /// A left click at a cell of the last frame.
    fn click(ctx: &Ctx, env: &Arc<Env<Ctx>>, column: u16, row: u16) {
        ctx.handle_mouse_event(
            crate::input::MouseEvent {
                kind: crate::input::MouseKind::Down(crate::input::MouseButton::Left),
                column,
                row,
                modifiers: crate::input::KeyModifiers::default(),
            },
            env,
        );
    }

    // ----------------------------------------------------------------
    // Moving with the text
    // ----------------------------------------------------------------

    #[test]
    fn typing_before_it_carries_it_along() {
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX")"#, &env, &ctx);
        run(r#"(goto-char 0) (insert "..")"#, &env, &ctx);
        assert_eq!(rows(&ctx, &env)[0], "..abcXXdef");
    }

    #[test]
    fn deleting_what_it_sat_in_front_of_leaves_it_where_that_began() {
        // And does not delete it. An overlay goes when everything it covered
        // does, because a span with no width says nothing; a position has no
        // width to lose, so there is no such state for it to be in.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX")"#, &env, &ctx);
        run("(delete-region 2 5)", &env, &ctx);
        assert_eq!(rows(&ctx, &env)[0], "abXXf");
        assert_eq!(
            run("(length (virtual-text-at 2))", &env, &ctx),
            LispExp::number(1.0)
        );
    }

    // ----------------------------------------------------------------
    // The door Lisp comes through
    // ----------------------------------------------------------------

    #[test]
    fn a_producer_replaces_its_own_batch_and_nobody_elses() {
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(
            r#"(make-virtual-text 1 "A" nil 0 'mine)
               (make-virtual-text 2 "B" nil 0 'mine)
               (make-virtual-text 3 "C" nil 0 'theirs)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(clear-virtual-text 'mine)", &env, &ctx),
            LispExp::number(2.0)
        );
        assert_eq!(rows(&ctx, &env)[0], "abcCdef", "theirs is untouched");
    }

    #[test]
    fn asking_what_is_at_a_position_reports_the_text_face_and_category() {
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 2 "hi" 'keyword 0 'mine)"#, &env, &ctx);
        let entry = run("(car (virtual-text-at 2))", &env, &ctx);
        let parts: Vec<LispExp<Ctx>> = entry.iter().collect();
        assert_eq!(parts[0], LispExp::string("hi".into()));
        assert_eq!(parts[1], LispExp::symbol("keyword".into()));
        assert_eq!(parts[2], LispExp::symbol("mine".into()));
    }

    #[test]
    fn a_handle_takes_back_exactly_one() {
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(setq one (make-virtual-text 1 "A"))"#, &env, &ctx);
        run(r#"(make-virtual-text 2 "B")"#, &env, &ctx);
        assert!(!run("(delete-virtual-text one)", &env, &ctx).is_nil());
        assert!(
            run("(delete-virtual-text one)", &env, &ctx).is_nil(),
            "and only once"
        );
        assert_eq!(rows(&ctx, &env)[0], "abBcdef");
    }

    #[test]
    fn reverting_a_buffer_takes_it_down() {
        // For the same reason reverting clears the overlays: it describes a
        // position in text that no longer exists.
        let (ctx, env) = editor();
        with_text("abcdef", &ctx);
        run(r#"(make-virtual-text 3 "XX")"#, &env, &ctx);
        ctx.with_current_buffer_mut(|buf| buf.adopt_text("something else"));
        assert_eq!(rows(&ctx, &env)[0], "something else");
    }

    #[test]
    fn pending_command_names_the_prompt_in_front_of_you() {
        // What a preview asks, and the only thing it needs from the editor
        // that is not about drawing.
        let (ctx, env) = editor();
        assert!(run("(pending-command)", &env, &ctx).is_nil());
        run(r#"(eval-file "commands")"#, &env, &ctx);
        run(r#"(call-interactively "string-rectangle")"#, &env, &ctx);
        assert_eq!(
            run("(pending-command)", &env, &ctx),
            LispExp::string("string-rectangle".into()),
            "the prompt is up and says which command it belongs to"
        );
    }
}
