//! The mouse: where a click lands, and what it is allowed to do.
//!
//! # What is worth testing here
//!
//! Almost all of it is arithmetic with an off-by-one in every corner. A click
//! is a cell on a screen and point is a character in a buffer, and between the
//! two sit a window's position, its scroll, the row its status line occupies
//! and the length of the line actually clicked on. Every one of those is a
//! chance to land one out, and one out is the kind of wrong a user blames on
//! the terminal rather than reporting.
//!
//! So these check the seams rather than the happy path: the far edge of a
//! rectangle, a window that has never been drawn, a window scrolled away from
//! the top, a click past the end of a short line, and a float sitting over a
//! buffer it would be wrong to move point in.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{DEFAULT_INIT_LISP, EditorState, Hit, create_global_env};
    use crate::input::{KeyModifiers, MouseButton, MouseEvent, MouseKind};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::ui::Rect;
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

    /// An editor with the mouse on, showing LINES numbered lines, with a frame
    /// composed so the windows know where they are.
    fn editor(lines: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        // Off by default in the editor; the shipped init.lisp turns it on, and
        // a test that wants to click has to say so too.
        run("(setq mouse-mode t)", &env, &ctx);
        let text: String = (0..lines).map(|n| format!("line {n}\n")).collect();
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| {
            b.text = GapBuffer::from(text.as_str());
            b.text.cursor_move(0, 0);
        });
        compose(&ctx, &env);
        (ctx, env)
    }

    fn compose(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        let _ = ctx.snapshot(env, W, H);
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

    fn wheel(ctx: &Ctx, env: &Arc<Env<Ctx>>, kind: MouseKind, column: u16, row: u16) {
        ctx.handle_mouse_event(
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    fn point(ctx: &Ctx) -> (usize, usize) {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .expect("read lock")
            .text
            .cursor_pos()
    }

    /// The rows of text the sole window has, and the row its status line is on.
    fn text_rows(ctx: &Ctx) -> usize {
        ctx.layout_root
            .read()
            .expect("layout")
            .window_at(0, 0)
            .expect("a window at the origin")
            .text_height
    }

    // ----------------------------------------------------------------
    // Rectangles
    // ----------------------------------------------------------------

    #[test]
    fn a_rectangle_owns_its_near_edges_and_not_its_far_ones() {
        // Half-open, as a rectangle of cells has to be: a window at x=0 of
        // width 80 owns columns 0 to 79, and 80 belongs to whatever is beside
        // it. Getting this wrong makes two windows both claim one column.
        let rect = Rect {
            x: 10,
            y: 5,
            width: 4,
            height: 3,
        };
        assert!(rect.contains(10, 5), "the top-left corner is inside");
        assert!(rect.contains(13, 7), "and so is the bottom-right cell");
        assert!(!rect.contains(14, 7), "one past the right edge is not");
        assert!(!rect.contains(13, 8), "nor one past the bottom");
        assert!(!rect.contains(9, 5), "nor one before the left");
        assert!(!rect.contains(10, 4), "nor one above the top");
    }

    #[test]
    fn a_window_that_has_never_been_drawn_cannot_be_clicked() {
        // Its rect is zero-sized until the first frame, which is the whole
        // reason the rect is stored rather than assumed.
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        assert!(
            ctx.layout_root
                .read()
                .expect("layout")
                .window_at(0, 0)
                .is_none(),
            "nothing has been composed, so nothing is anywhere yet"
        );
    }

    // ----------------------------------------------------------------
    // What is under the pointer
    // ----------------------------------------------------------------

    #[test]
    fn a_click_in_the_text_reports_the_line_and_column_under_it() {
        let (ctx, env) = editor(100);
        let _ = &env;
        assert_eq!(
            ctx.hit_test(4, 2),
            Some(Hit::Text {
                window: ctx.get_focused_window_id(),
                line: 2,
                column: 4
            })
        );
    }

    #[test]
    fn a_click_on_the_status_line_is_not_a_click_in_the_text() {
        let (ctx, env) = editor(100);
        let _ = &env;
        let rows = text_rows(&ctx);
        assert!(matches!(
            ctx.hit_test(4, rows as isize),
            Some(Hit::ModeLine { .. })
        ));
        assert!(matches!(
            ctx.hit_test(4, rows as isize - 1),
            Some(Hit::Text { .. })
        ));
    }

    #[test]
    fn the_echo_area_belongs_to_no_window() {
        let (ctx, env) = editor(100);
        let _ = &env;
        assert_eq!(ctx.hit_test(0, H as isize - 1), None);
    }

    #[test]
    fn a_scrolled_window_reports_the_line_actually_shown() {
        // The one that catches a forgotten `scroll_y`: on screen it is row
        // two, in the buffer it is line fifty-two.
        let (ctx, env) = editor(400);
        run("(goto-char (point-max))", &env, &ctx);
        compose(&ctx, &env);
        let top = ctx
            .layout_root
            .read()
            .expect("layout")
            .window_at(0, 0)
            .expect("a window")
            .scroll_y;
        assert!(
            top > 0,
            "the fixture has to be scrolled for this to mean anything"
        );
        assert_eq!(
            ctx.hit_test(0, 2),
            Some(Hit::Text {
                window: ctx.get_focused_window_id(),
                line: top + 2,
                column: 0
            })
        );
    }

    // ----------------------------------------------------------------
    // What a click does
    // ----------------------------------------------------------------

    #[test]
    fn clicking_puts_point_where_the_pointer_was() {
        let (ctx, env) = editor(100);
        click(&ctx, &env, 4, 3);
        assert_eq!(point(&ctx), (3, 4));
    }

    #[test]
    fn clicking_past_the_end_of_a_line_lands_at_its_end() {
        // Clamped against the buffer, not against the window: the screen has
        // no opinion about how long a line is.
        let (ctx, env) = editor(100);
        click(&ctx, &env, 70, 3);
        assert_eq!(point(&ctx), (3, "line 3".len()));
    }

    #[test]
    fn clicking_below_the_last_line_lands_on_the_last_line() {
        let (ctx, env) = editor(3);
        let rows = text_rows(&ctx);
        click(&ctx, &env, 0, rows as u16 - 1);
        let (line, _) = point(&ctx);
        assert_eq!(line, 3, "the empty line after the final newline");
    }

    #[test]
    fn clicking_in_the_other_window_focuses_it() {
        let (ctx, env) = editor(400);
        let first = ctx.get_focused_window_id();
        run("(split-window-below)", &env, &ctx);
        compose(&ctx, &env);
        let lower = ctx
            .layout_root
            .read()
            .expect("layout")
            .window_at(0, H as isize - 3)
            .expect("a lower window")
            .id;
        assert_ne!(lower, first, "the fixture needs two windows");

        click(&ctx, &env, 0, H as u16 - 3);
        assert_eq!(ctx.get_focused_window_id(), lower);
    }

    #[test]
    fn a_click_on_a_floating_window_is_swallowed() {
        // A float is drawn over a tiled window. Falling through would move
        // point in a buffer the pointer is not over and the user cannot see.
        let (ctx, env) = editor(100);
        run(r#"(minibuffer-read "P:" nil nil nil)"#, &env, &ctx);
        compose(&ctx, &env);
        let before = point(&ctx);
        let float = ctx
            .floating_windows
            .read()
            .expect("floats")
            .first()
            .expect("the prompt is a float")
            .rect;
        assert!(matches!(
            ctx.hit_test(float.x, float.y),
            Some(Hit::Floating { .. })
        ));
        click(&ctx, &env, float.x as u16, float.y as u16);
        assert_eq!(point(&ctx), before, "the file's point did not move");
    }

    // ----------------------------------------------------------------
    // The wheel
    // ----------------------------------------------------------------

    #[test]
    fn the_wheel_scrolls_without_moving_point_or_focus() {
        // Rolling the wheel over a window is a way of looking at it. Stealing
        // the cursor would make the next keystroke land somewhere unexpected.
        let (ctx, env) = editor(400);
        let before_point = point(&ctx);
        let before_focus = ctx.get_focused_window_id();
        wheel(&ctx, &env, MouseKind::ScrollDown, 0, 2);
        let top = ctx
            .layout_root
            .read()
            .expect("layout")
            .window_at(0, 0)
            .expect("a window")
            .scroll_y;
        assert_eq!(top, 3, "one notch is three lines");
        assert_eq!(point(&ctx), before_point);
        assert_eq!(ctx.get_focused_window_id(), before_focus);
    }

    #[test]
    fn the_wheel_stops_at_the_top() {
        let (ctx, env) = editor(400);
        wheel(&ctx, &env, MouseKind::ScrollUp, 0, 2);
        assert_eq!(
            ctx.layout_root
                .read()
                .expect("layout")
                .window_at(0, 0)
                .expect("a window")
                .scroll_y,
            0
        );
    }

    #[test]
    fn the_wheel_stops_with_the_last_line_in_view() {
        // Past that the window fills with the nothing after the end of the
        // buffer, which the reader then has to scroll back out of by hand.
        let (ctx, env) = editor(10);
        for _ in 0..20 {
            wheel(&ctx, &env, MouseKind::ScrollDown, 0, 2);
        }
        let top = ctx
            .layout_root
            .read()
            .expect("layout")
            .window_at(0, 0)
            .expect("a window")
            .scroll_y;
        assert_eq!(top, 10, "the last line, and no further");
    }

    #[test]
    fn the_wheel_keeps_going_once_point_is_off_the_screen() {
        // The bug this comes from: scrolling stopped dead the moment the
        // cursor reached an edge. The view had been reconciled with point on
        // every frame, so as soon as the wheel moved the text far enough for
        // point to leave the window, the next frame dragged it straight back.
        //
        // A frame has to be composed between the notches, because composing is
        // where that reconciliation happened -- a test that only turned the
        // wheel would have passed throughout.
        let (ctx, env) = editor(400);
        let rows = text_rows(&ctx);
        let notches = rows / 3 + 4;
        for _ in 0..notches {
            wheel(&ctx, &env, MouseKind::ScrollDown, 0, 2);
            compose(&ctx, &env);
        }
        let top = ctx
            .layout_root
            .read()
            .expect("layout")
            .window_at(0, 0)
            .expect("a window")
            .scroll_y;
        assert_eq!(top, notches * 3, "every notch moved the view");
        assert!(top > rows, "and point is well above the top of it now");
        assert_eq!(point(&ctx), (0, 0), "while point stayed where it was");
    }

    #[test]
    fn moving_point_brings_the_view_back_to_it() {
        // The other half: a view scrolled away from point stays there until
        // something says where you are, and then it follows again.
        let (ctx, env) = editor(400);
        for _ in 0..20 {
            wheel(&ctx, &env, MouseKind::ScrollDown, 0, 2);
            compose(&ctx, &env);
        }
        run("(next-line)", &env, &ctx);
        compose(&ctx, &env);
        let (top, rows) = {
            let layout = ctx.layout_root.read().expect("layout");
            let win = layout.window_at(0, 0).expect("a window");
            (win.scroll_y, win.text_height)
        };
        let (line, _) = point(&ctx);
        assert!(
            (top..top + rows).contains(&line),
            "point is visible again: line {line} in rows {top}..{}",
            top + rows
        );
        // The smallest move that brings it back, not a recentre: point is on
        // the top row rather than in the middle, which is what `C-p' into the
        // line above the window does everywhere else.
        assert_eq!(top, line);
    }

    // ----------------------------------------------------------------
    // Dragging out a selection
    // ----------------------------------------------------------------

    fn drag(ctx: &Ctx, env: &Arc<Env<Ctx>>, column: u16, row: u16) {
        ctx.handle_mouse_event(
            MouseEvent {
                kind: MouseKind::Drag(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    fn release(ctx: &Ctx, env: &Arc<Env<Ctx>>, column: u16, row: u16) {
        ctx.handle_mouse_event(
            MouseEvent {
                kind: MouseKind::Up(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    /// (mark, point) as offsets, and whether the region is live.
    fn region(ctx: &Ctx) -> (Option<usize>, usize, bool) {
        let buffer = ctx.get_buffer("*scratch*").expect("*scratch*");
        let buf = buffer.read().expect("read lock");
        (
            buf.mark.as_ref().map(|mark| mark.at),
            buf.text.cursor_pos_1d(),
            buf.mark.as_ref().is_some_and(|mark| mark.active),
        )
    }

    #[test]
    fn dragging_selects_from_where_the_button_went_down() {
        let (ctx, env) = editor(100);
        click(&ctx, &env, 2, 1);
        let (_, start, active) = region(&ctx);
        assert!(!active, "a click on its own selects nothing");

        drag(&ctx, &env, 4, 3);
        let (mark, end, active) = region(&ctx);
        assert!(active, "a drag makes a region");
        assert_eq!(mark, Some(start), "anchored where the click was");
        assert_eq!(point(&ctx), (3, 4));
        assert!(end > start);
    }

    #[test]
    fn the_anchor_stays_put_across_a_whole_drag() {
        // The mark is set by the *first* event of the drag and left alone
        // after it, which is what "whether it is already active" decides.
        let (ctx, env) = editor(100);
        click(&ctx, &env, 2, 1);
        let (_, start, _) = region(&ctx);
        for row in 2..8 {
            drag(&ctx, &env, 4, row);
        }
        assert_eq!(region(&ctx).0, Some(start));
        assert_eq!(point(&ctx), (7, 4));
    }

    #[test]
    fn dragging_off_the_bottom_selects_to_the_last_visible_line() {
        // Clamped to what the window is showing. Following the pointer off the
        // window by scrolling is a separate feature and a worse one to get
        // subtly wrong.
        let (ctx, env) = editor(400);
        let rows = text_rows(&ctx);
        click(&ctx, &env, 0, 1);
        drag(&ctx, &env, 0, rows as u16 + 40);
        assert_eq!(point(&ctx).0, rows - 1);
    }

    #[test]
    fn a_drag_after_the_button_comes_up_does_nothing() {
        let (ctx, env) = editor(100);
        click(&ctx, &env, 2, 1);
        drag(&ctx, &env, 4, 3);
        release(&ctx, &env, 4, 3);
        let before = point(&ctx);
        drag(&ctx, &env, 8, 9);
        assert_eq!(point(&ctx), before);
    }

    #[test]
    fn a_drag_stays_in_the_window_it_started_in() {
        // Over the *other* window by the end, and still selecting in this one.
        // A selection that changed buffers halfway through is nobody's idea of
        // a selection.
        let (ctx, env) = editor(400);
        run("(split-window-below)", &env, &ctx);
        compose(&ctx, &env);
        let first = ctx.get_focused_window_id();

        click(&ctx, &env, 0, 1);
        drag(&ctx, &env, 0, H as u16 - 3);
        assert_eq!(
            ctx.get_focused_window_id(),
            first,
            "focus did not follow the pointer into the other window"
        );
        assert!(region(&ctx).2, "and the region is still being made here");
    }

    // ----------------------------------------------------------------
    // Dragging past the edge
    // ----------------------------------------------------------------

    fn scroll_of(ctx: &Ctx) -> usize {
        ctx.layout_root
            .read()
            .expect("layout")
            .window_at(0, 0)
            .expect("a window")
            .scroll_y
    }

    #[test]
    fn a_drag_inside_the_window_asks_for_no_timer() {
        let (ctx, env) = editor(400);
        click(&ctx, &env, 0, 1);
        drag(&ctx, &env, 4, 5);
        assert_eq!(ctx.drag_scroll_in(), None);
        assert!(!ctx.drag_scroll_tick(&env));
    }

    #[test]
    fn a_drag_below_the_window_keeps_scrolling_while_the_pointer_sits_still() {
        // The reason this is a timer at all: a pointer held outside the window
        // sends nothing, and that is exactly where somebody selecting a long
        // passage leaves it. The test holds it still too -- every turn below
        // is driven by the clock, not by an event.
        let (ctx, env) = editor(400);
        let rows = text_rows(&ctx);
        click(&ctx, &env, 0, 1);
        drag(&ctx, &env, 0, rows as u16 + 5);
        assert_eq!(scroll_of(&ctx), 0, "the drag itself does not scroll");
        assert!(ctx.drag_scroll_in().is_some(), "but it asks to be woken");

        for turn in 1..=5 {
            assert!(ctx.drag_scroll_tick(&env));
            assert_eq!(scroll_of(&ctx), turn, "one line a turn");
        }
        assert!(region(&ctx).2, "and the selection came with it");
        assert_eq!(
            point(&ctx).0,
            scroll_of(&ctx) + rows - 1,
            "reaching to the last line now showing"
        );
    }

    #[test]
    fn a_drag_at_the_top_of_the_frame_scrolls_back_and_stops_there() {
        // A terminal cannot report a row above zero, so a drag off the top of
        // the screen arrives clamped to row 0 -- which is inside the topmost
        // window. Its first row therefore has to count as "above", or upward
        // auto-scroll would not work at all in the ordinary single window.
        let (ctx, env) = editor(400);
        run("(goto-char (point-max))", &env, &ctx);
        compose(&ctx, &env);
        let start = scroll_of(&ctx);
        assert!(start > 10, "the fixture has to be scrolled to begin with");

        click(&ctx, &env, 0, 2);
        drag(&ctx, &env, 0, 0);
        assert!(ctx.drag_scroll_in().is_some());
        for _ in 0..3 {
            assert!(ctx.drag_scroll_tick(&env));
        }
        assert_eq!(scroll_of(&ctx), start - 3, "one line a turn, upwards");

        for _ in 0..(start + 10) {
            ctx.drag_scroll_tick(&env);
        }
        assert_eq!(scroll_of(&ctx), 0, "and it stops at the top");
        assert!(
            !ctx.drag_scroll_tick(&env),
            "with nothing left to do, so the turns stop costing anything"
        );
    }

    #[test]
    fn selecting_along_the_top_row_of_a_buffer_already_at_the_top_scrolls_nothing() {
        // The cost of the rule above, and it is nil: there is nothing to
        // scroll to, so the turn does nothing and the selection is ordinary.
        let (ctx, env) = editor(400);
        click(&ctx, &env, 0, 0);
        drag(&ctx, &env, 6, 0);
        assert!(!ctx.drag_scroll_tick(&env));
        assert_eq!(scroll_of(&ctx), 0);
        assert_eq!(point(&ctx), (0, 6));
    }

    #[test]
    fn the_timer_is_forgotten_when_the_button_comes_up() {
        let (ctx, env) = editor(400);
        let rows = text_rows(&ctx);
        click(&ctx, &env, 0, 1);
        drag(&ctx, &env, 0, rows as u16 + 5);
        assert!(ctx.drag_scroll_in().is_some());
        release(&ctx, &env, 0, rows as u16 + 5);
        assert_eq!(ctx.drag_scroll_in(), None);
        assert!(!ctx.drag_scroll_tick(&env));
    }

    #[test]
    fn dragging_a_boundary_never_asks_for_the_timer() {
        // Only a *selection* runs off the end of what is on screen. A boundary
        // is bounded by the frame, and a resize that kept going while the hand
        // was still would be alarming.
        let (ctx, env) = editor(100);
        run("(split-window-right)", &env, &ctx);
        compose(&ctx, &env);
        let column = rule_column(&ctx, &env);
        click(&ctx, &env, column as u16, 2);
        drag(&ctx, &env, column as u16 + 2, 200);
        assert_eq!(ctx.drag_scroll_in(), None);
    }

    // ----------------------------------------------------------------
    // Dragging a boundary
    // ----------------------------------------------------------------

    /// Where the rule between two side-by-side windows is drawn.
    fn rule_column(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> isize {
        ctx.snapshot(env, W, H)
            .separators
            .first()
            .expect("a rule between the two windows")
            .rect
            .x
    }

    fn first_window_width(ctx: &Ctx) -> usize {
        ctx.layout_root
            .read()
            .expect("layout")
            .window_at(0, 0)
            .expect("a window")
            .rect
            .width
    }

    #[test]
    fn the_rule_between_two_windows_is_something_the_pointer_can_find() {
        let (ctx, env) = editor(100);
        run("(split-window-right)", &env, &ctx);
        compose(&ctx, &env);
        let column = rule_column(&ctx, &env);
        assert!(matches!(
            ctx.hit_test(column, 2),
            Some(Hit::Separator { .. })
        ));
        assert!(
            matches!(ctx.hit_test(column - 1, 2), Some(Hit::Text { .. })),
            "and the column beside it still belongs to a window"
        );
    }

    #[test]
    fn dragging_the_rule_moves_it() {
        let (ctx, env) = editor(100);
        run("(split-window-right)", &env, &ctx);
        compose(&ctx, &env);
        let column = rule_column(&ctx, &env);
        let before = first_window_width(&ctx);

        click(&ctx, &env, column as u16, 2);
        drag(&ctx, &env, column as u16 + 10, 2);
        compose(&ctx, &env);

        assert_eq!(first_window_width(&ctx), before + 10);
        assert_eq!(rule_column(&ctx, &env), column + 10);
    }

    #[test]
    fn a_boundary_cannot_be_dragged_past_the_edge() {
        // Both windows keep enough to be worth looking at. A window dragged to
        // nothing is a window that cannot be dragged back.
        let (ctx, env) = editor(100);
        run("(split-window-right)", &env, &ctx);
        compose(&ctx, &env);
        let column = rule_column(&ctx, &env);

        click(&ctx, &env, column as u16, 2);
        drag(&ctx, &env, 0, 2);
        compose(&ctx, &env);
        assert!(
            first_window_width(&ctx) >= crate::ui::MIN_DRAGGED_WIDTH,
            "the left window kept a usable width"
        );

        drag(&ctx, &env, W as u16, 2);
        compose(&ctx, &env);
        assert!(
            W - first_window_width(&ctx) >= crate::ui::MIN_DRAGGED_WIDTH,
            "and so did the right one"
        );
    }

    #[test]
    fn dragging_a_status_line_resizes_the_windows_it_divides() {
        let (ctx, env) = editor(400);
        run("(split-window-below)", &env, &ctx);
        compose(&ctx, &env);
        let rows = text_rows(&ctx);

        click(&ctx, &env, 4, rows as u16);
        drag(&ctx, &env, 4, rows as u16 + 3);
        compose(&ctx, &env);

        assert_eq!(text_rows(&ctx), rows + 3, "the upper window grew");
    }

    #[test]
    fn the_bottom_windows_status_line_divides_nothing() {
        // The row under it is the echo area, not another window.
        let (ctx, env) = editor(400);
        run("(split-window-below)", &env, &ctx);
        compose(&ctx, &env);
        let lower = ctx
            .layout_root
            .read()
            .expect("layout")
            .window_at(0, H as isize - 3)
            .expect("a lower window")
            .id;
        assert_eq!(
            ctx.layout_root
                .read()
                .expect("layout")
                .mode_line_divider(lower),
            None
        );
    }

    // ----------------------------------------------------------------
    // The setting
    // ----------------------------------------------------------------

    #[test]
    fn nothing_happens_at_all_while_mouse_mode_is_off() {
        // Checked in the editor rather than only where the terminal switches
        // capture on, so the setting means the same thing to every frontend.
        let (ctx, env) = editor(100);
        run("(setq mouse-mode nil)", &env, &ctx);
        let before = point(&ctx);
        click(&ctx, &env, 4, 3);
        wheel(&ctx, &env, MouseKind::ScrollDown, 0, 2);
        assert_eq!(point(&ctx), before);
        assert_eq!(
            ctx.layout_root
                .read()
                .expect("layout")
                .window_at(0, 0)
                .expect("a window")
                .scroll_y,
            0
        );
    }

    // ----------------------------------------------------------------
    // What the frontend is told
    // ----------------------------------------------------------------

    #[test]
    fn only_an_event_that_did_something_asks_for_a_frame() {
        // A terminal reporting the mouse sends motion and drag events by the
        // dozen per second, and almost none of them mean anything here. The
        // frontend redraws on this answer, so an over-eager `true` is a screen
        // repainting continuously with a frame identical to the last.
        let (ctx, env) = editor(400);

        let acted = |kind, column, row| {
            ctx.handle_mouse_event(
                MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers: KeyModifiers::default(),
                },
                &env,
            )
        };

        assert!(
            acted(MouseKind::Down(MouseButton::Left), 4, 3),
            "a click moves point"
        );
        assert!(acted(MouseKind::ScrollDown, 0, 2), "a notch moves the view");
        assert!(
            acted(MouseKind::Drag(MouseButton::Left), 4, 4),
            "a drag extends the selection the click started"
        );
        assert!(
            !acted(MouseKind::Up(MouseButton::Left), 4, 4),
            "a release only forgets the drag -- the screen is already right"
        );
        assert!(
            !acted(MouseKind::Drag(MouseButton::Left), 6, 6),
            "and a drag with no button behind it does nothing"
        );
        assert!(
            !acted(MouseKind::Down(MouseButton::Right), 4, 4),
            "nor does the right button"
        );
        assert!(
            !acted(MouseKind::Down(MouseButton::Left), 0, H as u16 - 1),
            "nor to a click on the echo area"
        );
    }

    #[test]
    fn nothing_is_asked_for_while_the_mouse_is_off() {
        let (ctx, env) = editor(100);
        run("(setq mouse-mode nil)", &env, &ctx);
        assert!(!ctx.handle_mouse_event(
            MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                column: 4,
                row: 3,
                modifiers: KeyModifiers::default(),
            },
            &env
        ));
    }

    // ----------------------------------------------------------------
    // The configuration the editor writes for itself
    // ----------------------------------------------------------------

    /// What the editor writes for somebody with no configuration loads into an
    /// editor with *nothing else in it*.
    ///
    /// # Why the bare environment is the whole test
    ///
    /// That file is evaluated by `create_global_env` with `?`, so anything it
    /// raises takes the editor down rather than degrading. And the modules it
    /// loads with `eval-file` are not always there: a test binary has no
    /// `data/lisp` beside it, and an installation can have a module removed.
    /// When they are missing, `defcommand` is not a macro -- so a form written
    /// with it is evaluated as an ordinary call and raises `UnboundVariable`
    /// on its own name.
    ///
    /// An earlier version of this test loaded `commands.lisp` first to make
    /// the configuration work, which is precisely the condition that does not
    /// hold when it matters. So: nothing is loaded here on purpose.
    #[test]
    fn the_default_configuration_loads_into_a_bare_editor() {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        eval_str(DEFAULT_INIT_LISP, &env, &ctx)
            .expect("the default configuration must load with no modules present");
        assert!(
            env.get_variable("mouse-mode")
                .is_some_and(|value| !value.is_nil()),
            "and it turns the mouse on"
        );
    }

    #[test]
    fn the_mouse_can_be_turned_off_and_on_again() {
        // A built-in command rather than Lisp in the configuration, for the
        // reason above: the configuration cannot define commands.
        let (ctx, env) = editor(100);
        run("(mouse-mode-toggle)", &env, &ctx);
        assert!(
            env.get_variable("mouse-mode")
                .is_some_and(|value| value.is_nil()),
            "off"
        );
        click(&ctx, &env, 4, 3);
        assert_eq!(
            point(&ctx),
            (0, 0),
            "and the mouse does nothing while it is"
        );

        run("(mouse-mode-toggle)", &env, &ctx);
        click(&ctx, &env, 4, 3);
        assert_eq!(point(&ctx), (3, 4), "on again, and clicking works");
    }
}
