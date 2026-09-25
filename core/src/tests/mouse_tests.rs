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
            !acted(MouseKind::Drag(MouseButton::Left), 4, 4),
            "nothing is bound to a drag yet"
        );
        assert!(
            !acted(MouseKind::Up(MouseButton::Left), 4, 4),
            "nor to a release"
        );
        assert!(
            !acted(MouseKind::Down(MouseButton::Right), 4, 4),
            "nor to the right button"
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
