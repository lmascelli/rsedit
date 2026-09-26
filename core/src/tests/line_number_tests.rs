//! The line-number gutter: how wide it is, what it says, and what it moves.
//!
//! # What is worth testing here
//!
//! A gutter is not a decoration bolted on beside the text -- it *takes
//! columns away from it*, and everything that was measured in columns has to
//! be measured again against what is left. The cursor's position, the
//! horizontal scroll, every syntax and region highlight, and where a click
//! lands are five separate pieces of arithmetic that were all correct before
//! the gutter existed and are all wrong by exactly its width if one of them
//! is missed.
//!
//! So these check the seams rather than the look of it: that the text rect
//! moved, that the numbers are in the columns the text gave up and not over
//! the text, that a click four columns into a four-column gutter is column
//! zero, and that a window with no room for numbers shows none rather than
//! showing numbers instead of a file.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyModifiers, MouseButton, MouseEvent, MouseKind};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::ui::{Face, RenderableWindowView};
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

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"));
    }

    /// An editor showing LINES numbered lines of *scratch*, point at the top.
    fn editor(lines: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        let text: String = (0..lines).map(|n| format!("line {n}\n")).collect();
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text.as_str());
            b.text.cursor_move(0, 0);
        });
        (ctx, env)
    }

    /// The window showing *scratch*, as of a freshly composed frame.
    fn view(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> RenderableWindowView {
        ctx.snapshot(env, W, H)
            .views
            .into_iter()
            .find(|v| v.buffer_name == "*scratch*")
            .expect("*scratch* must be on screen")
    }

    /// What the gutter says, row by row, trailing space and all.
    fn numbers(view: &RenderableWindowView) -> Vec<String> {
        view.gutter.iter().map(|cell| cell.text.clone()).collect()
    }

    fn click(ctx: &Ctx, env: &Arc<Env<Ctx>>, column: u16, row: u16) {
        ctx.handle_mouse_event(
            MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    fn point(ctx: &Ctx) -> (usize, usize) {
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos())
            .expect("*scratch*")
    }

    // ----------------------------------------------------------------
    // Off, which is the default
    // ----------------------------------------------------------------

    #[test]
    fn a_window_shows_no_gutter_unless_something_asks_for_one() {
        // The whole feature costs columns, so nothing gets it by accident.
        let (ctx, env) = editor(10);
        let view = view(&ctx, &env);
        assert_eq!(view.gutter_width, 0);
        assert!(view.gutter.is_empty());
        assert_eq!(view.rect.x, 0, "the text still starts at the left edge");
        assert_eq!(view.rect.width, W, "and still gets the whole width");
    }

    #[test]
    fn setting_the_variable_to_nil_is_the_same_as_not_setting_it() {
        let (ctx, env) = editor(10);
        run("(setq display-line-numbers nil)", &env, &ctx);
        assert_eq!(view(&ctx, &env).gutter_width, 0);
    }

    // ----------------------------------------------------------------
    // What the numbers say
    // ----------------------------------------------------------------

    #[test]
    fn the_numbers_count_lines_from_one() {
        // Lines are counted from zero inside the editor and from one
        // everywhere a person reads them, and this is the boundary between
        // the two.
        let (ctx, env) = editor(3);
        run("(setq display-line-numbers t)", &env, &ctx);
        let view = view(&ctx, &env);
        assert_eq!(&numbers(&view)[..3], &["  1 ", "  2 ", "  3 "]);
    }

    #[test]
    fn rows_past_the_end_of_the_buffer_are_blank_rather_than_absent() {
        // Blank, so a renderer can pair cells with rows by index. Absent, and
        // a window scrolled near the end of a file would show the numbers of
        // lines that are not on it.
        let (ctx, env) = editor(3);
        // Written out rather than taken from the helper, which ends its text
        // with a newline -- and a trailing newline is a fourth, empty line,
        // which is a real line and gets a real number.
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from("one\ntwo\nthree");
            b.text.cursor_move(0, 0);
        });
        run("(setq display-line-numbers t)", &env, &ctx);
        let view = view(&ctx, &env);
        assert_eq!(
            view.gutter.len(),
            view.rect.height,
            "one cell per row drawn, not one per line of text"
        );
        assert_eq!(numbers(&view)[2], "  3 ", "the last line still has one");
        for (row, cell) in view.gutter.iter().enumerate().skip(3) {
            assert_eq!(cell.text, "    ", "row {row} is past the end of the buffer");
        }
    }

    #[test]
    fn every_cell_is_exactly_the_gutter_wide() {
        // The renderer prints these without measuring them, so a short one
        // would leave the cell beneath it showing through.
        let (ctx, env) = editor(150);
        run("(setq display-line-numbers t)", &env, &ctx);
        let view = view(&ctx, &env);
        for cell in &view.gutter {
            assert_eq!(cell.text.chars().count(), view.gutter_width);
        }
    }

    // ----------------------------------------------------------------
    // How wide it is
    // ----------------------------------------------------------------

    #[test]
    fn the_gutter_keeps_a_minimum_width_so_short_files_do_not_shuffle() {
        // Three digits and a space, for a file of three lines as much as for
        // one of three hundred. Fitting the gutter to the count instead would
        // slide the whole text one column left the moment a file shrank below
        // ten lines.
        let (ctx, env) = editor(3);
        run("(setq display-line-numbers t)", &env, &ctx);
        assert_eq!(view(&ctx, &env).gutter_width, 4);
    }

    #[test]
    fn the_gutter_widens_for_a_file_that_needs_the_room() {
        let (ctx, env) = editor(1500);
        run("(setq display-line-numbers t)", &env, &ctx);
        let view = view(&ctx, &env);
        assert_eq!(view.gutter_width, 5, "four digits and a space");
        assert_eq!(numbers(&view)[0], "   1 ");
    }

    #[test]
    fn the_minimum_is_configurable() {
        let (ctx, env) = editor(3);
        run(
            "(setq display-line-numbers t) (setq display-line-numbers-width 6)",
            &env,
            &ctx,
        );
        let view = view(&ctx, &env);
        assert_eq!(view.gutter_width, 7);
        assert_eq!(numbers(&view)[0], "     1 ");
    }

    #[test]
    fn a_nonsensical_width_falls_back_rather_than_eating_the_window() {
        // A string is not a number of digits, and a gutter the width of the
        // frame is not what anybody meant by one.
        let (ctx, env) = editor(3);
        run(
            "(setq display-line-numbers t) (setq display-line-numbers-width \"wide\")",
            &env,
            &ctx,
        );
        assert_eq!(view(&ctx, &env).gutter_width, 4);
    }

    #[test]
    fn a_window_too_narrow_for_numbers_shows_the_text_instead() {
        // The failure mode this avoids is a split narrow enough that the
        // numbers are most of what is in it.
        let (ctx, env) = editor(10);
        run("(setq display-line-numbers t)", &env, &ctx);
        let narrow = ctx.snapshot(&env, 10, H);
        let view = narrow
            .views
            .into_iter()
            .find(|v| v.buffer_name == "*scratch*")
            .expect("*scratch* must be on screen");
        assert_eq!(view.gutter_width, 0);
        assert_eq!(view.rect.width, 10, "the text keeps every column");
    }

    // ----------------------------------------------------------------
    // What it moves
    // ----------------------------------------------------------------

    #[test]
    fn the_text_rect_gives_up_exactly_the_gutters_columns() {
        // The one invariant everything else here rests on: the gutter is
        // taken *out of* the window rather than added beside it, so the sum
        // is unchanged and nothing overlaps its neighbour.
        let (ctx, env) = editor(10);
        run("(setq display-line-numbers t)", &env, &ctx);
        let view = view(&ctx, &env);
        assert_eq!(view.gutter_width, 4);
        assert_eq!(view.rect.x, 4);
        assert_eq!(view.rect.width, W - 4);
    }

    #[test]
    fn the_lines_are_not_prefixed_with_their_own_numbers() {
        // The tempting shortcut, and the reason not to take it: a number
        // pasted onto the front of a line is a number the region, the
        // highlighter and the cursor all have to subtract again.
        let (ctx, env) = editor(3);
        run("(setq display-line-numbers t)", &env, &ctx);
        let view = view(&ctx, &env);
        assert_eq!(view.lines[0], "line 0");
    }

    #[test]
    fn the_cursor_is_placed_within_the_text_and_not_the_gutter() {
        // `cursor_rel_pos` is relative to the rect, and the rect has already
        // moved, so a cursor in column zero of the text is column zero here
        // -- not column four.
        let (ctx, env) = editor(10);
        run("(setq display-line-numbers t)", &env, &ctx);
        ctx.with_buffer_mut("*scratch*", |b| b.text.cursor_move(1, 3));
        let view = view(&ctx, &env);
        assert_eq!(view.cursor_rel_pos, Some((3, 1)));
    }

    #[test]
    fn the_view_scrolls_sideways_at_the_width_the_text_actually_has() {
        // A long line, with point past where the *narrowed* window ends but
        // short of where the whole window would have. Measuring against the
        // full width would leave the cursor drawn under the right-hand edge.
        let (ctx, env) = editor(1);
        run("(setq display-line-numbers t)", &env, &ctx);
        let long: String = std::iter::repeat_n('x', 200).collect();
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(long.as_str());
            b.text.cursor_move(0, 78);
        });
        let view = view(&ctx, &env);
        assert_eq!(
            view.rect.width,
            W - 4,
            "the text has the window less its gutter"
        );
        let (cx, _) = view.cursor_rel_pos.expect("the focused window draws one");
        assert!(
            cx < view.rect.width,
            "the cursor at column {cx} must be inside the {} text columns",
            view.rect.width
        );
    }

    // ----------------------------------------------------------------
    // The line point is on
    // ----------------------------------------------------------------

    #[test]
    fn the_line_point_is_on_is_faced_differently_from_the_rest() {
        let (ctx, env) = editor(10);
        run("(setq display-line-numbers t)", &env, &ctx);
        ctx.with_buffer_mut("*scratch*", |b| b.text.cursor_move(2, 0));
        let view = view(&ctx, &env);
        assert_eq!(view.gutter[2].face, Face::LINE_NUMBER_CURRENT);
        assert_eq!(view.gutter[0].face, Face::LINE_NUMBER);
        assert_eq!(view.gutter[3].face, Face::LINE_NUMBER);
    }

    #[test]
    fn relative_numbering_counts_the_distance_from_point() {
        let (ctx, env) = editor(10);
        run("(setq display-line-numbers 'relative)", &env, &ctx);
        ctx.with_buffer_mut("*scratch*", |b| b.text.cursor_move(3, 0));
        let view = view(&ctx, &env);
        assert_eq!(
            &numbers(&view)[..7],
            &["  3 ", "  2 ", "  1 ", "  4 ", "  1 ", "  2 ", "  3 "],
            "distances either side, and point's own line by its real number"
        );
    }

    #[test]
    fn relative_numbering_shows_point_its_own_number_rather_than_a_nought() {
        // A distance of zero is the one number nobody needs, and the line you
        // are on is the one you want to be able to read off the screen.
        let (ctx, env) = editor(10);
        run("(setq display-line-numbers 'relative)", &env, &ctx);
        ctx.with_buffer_mut("*scratch*", |b| b.text.cursor_move(5, 0));
        assert_eq!(numbers(&view(&ctx, &env))[5], "  6 ");
    }

    #[test]
    fn a_value_that_is_neither_nil_nor_relative_asks_for_ordinary_numbers() {
        // A misspelt symbol shows numbers that are merely not the ones asked
        // for, rather than silently showing none.
        let (ctx, env) = editor(3);
        run("(setq display-line-numbers 'reltaive)", &env, &ctx);
        assert_eq!(&numbers(&view(&ctx, &env))[..2], &["  1 ", "  2 "]);
    }

    // ----------------------------------------------------------------
    // The mouse
    // ----------------------------------------------------------------

    #[test]
    fn a_click_lands_on_the_column_under_it_and_not_the_gutter_out() {
        // The regression the whole `Window::gutter_width` field exists for.
        // Without it every click in a numbered window is off to the right by
        // the width of its own numbers.
        let (ctx, env) = editor(10);
        run(
            "(setq mouse-mode t) (setq display-line-numbers t)",
            &env,
            &ctx,
        );
        // Compose once so the window knows where it is and how wide its
        // gutter came out.
        let _ = ctx.snapshot(&env, W, H);
        click(&ctx, &env, 4 + 3, 1);
        assert_eq!(point(&ctx), (1, 3));
    }

    #[test]
    fn a_click_on_the_numbers_lands_at_the_start_of_that_line() {
        // The honest answer for a click that is not over any character: the
        // nearest one is the first.
        let (ctx, env) = editor(10);
        run(
            "(setq mouse-mode t) (setq display-line-numbers t)",
            &env,
            &ctx,
        );
        let _ = ctx.snapshot(&env, W, H);
        click(&ctx, &env, 1, 2);
        assert_eq!(point(&ctx), (2, 0));
    }

    #[test]
    fn a_click_still_lands_correctly_once_the_numbers_are_turned_off_again() {
        // `gutter_width` is written on every composition, so turning the
        // feature off has to clear it. Left behind, it would move every click
        // in a window that used to have numbers.
        let (ctx, env) = editor(10);
        run(
            "(setq mouse-mode t) (setq display-line-numbers t)",
            &env,
            &ctx,
        );
        let _ = ctx.snapshot(&env, W, H);
        run("(setq display-line-numbers nil)", &env, &ctx);
        let _ = ctx.snapshot(&env, W, H);
        click(&ctx, &env, 3, 1);
        assert_eq!(point(&ctx), (1, 3));
    }

    // ----------------------------------------------------------------
    // Floating windows
    // ----------------------------------------------------------------

    #[test]
    fn a_floating_window_has_no_gutter() {
        // A float is a prompt or a strip of choices, and numbering rows that
        // are not lines of a file says nothing.
        let (ctx, env) = editor(10);
        run("(setq display-line-numbers t)", &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        for view in frame.views.iter().filter(|v| v.has_border) {
            assert_eq!(view.gutter_width, 0, "{} is a float", view.buffer_name);
            assert!(view.gutter.is_empty());
        }
    }
}
