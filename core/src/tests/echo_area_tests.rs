//! The echo area's height: a message of several lines takes a row for each,
//! as many as `max-echo-area-height` lets it, from the windows above, which
//! have them back when it goes.
//!
//! Design: `doc/architettura-editor.typ, <snapshot>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::ui::width::cells;
    use crate::ui::{FrameSnapshot, echo_rows};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env must build")
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|err| panic!("{src}: {err:?}"))
    }

    /// The row under the lowest tiled window, its status line counted: where
    /// the windows give way to the echo area.
    fn windows_end(frame: &FrameSnapshot) -> usize {
        frame
            .views
            .iter()
            .filter(|view| !view.has_border)
            .map(|view| {
                view.rect.y as usize + view.rect.height + usize::from(view.mode_line.is_some())
            })
            .max()
            .expect("a tiled window")
    }

    /// N lines, "line 1" to "line N".
    fn lines(n: usize) -> String {
        (1..=n)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // ----------------------------------------------------------------
    // Cutting a text into rows
    // ----------------------------------------------------------------

    #[test]
    fn a_line_is_a_row_and_nothing_is_an_empty_one() {
        assert_eq!(echo_rows("hello", 6, W), vec!["hello"]);
        assert_eq!(echo_rows("", 6, W), vec![""]);
        assert_eq!(echo_rows("\n", 6, W), vec![""]);
    }

    #[test]
    fn each_line_is_a_row_and_the_last_newline_starts_none() {
        assert_eq!(echo_rows("a\nb\n", 6, W), vec!["a", "b"]);
        assert_eq!(echo_rows("a\r\nb\r\n", 6, W), vec!["a", "b"]);
        assert_eq!(echo_rows("a\n\nb", 6, W), vec!["a", "", "b"]);
    }

    #[test]
    fn a_line_wider_than_the_frame_goes_on_in_the_rows_under_it() {
        let long = format!("{}{}{}", "a".repeat(W), "b".repeat(W), "c".repeat(5));
        assert_eq!(
            echo_rows(&long, 6, W),
            vec!["a".repeat(W), "b".repeat(W), "c".repeat(5)]
        );
        assert_eq!(
            echo_rows(&"x".repeat(W), 6, W),
            vec!["x".repeat(W)],
            "a line as wide as the frame is one row"
        );
        assert_eq!(
            echo_rows(&format!("short\n{}", "y".repeat(W + 1)), 6, W),
            vec!["short".to_string(), "y".repeat(W), "y".to_string()]
        );
    }

    #[test]
    fn a_wide_character_is_not_cut_in_two_by_a_break() {
        assert_eq!(echo_rows("ab中c", 6, 3), vec!["ab", "中c"]);
    }

    #[test]
    fn an_unmeasured_frame_wraps_nothing() {
        let long = "x".repeat(W * 2);
        assert_eq!(echo_rows(&long, 6, 0), vec![long.clone()]);
    }

    #[test]
    fn rows_wrapped_past_the_limit_are_counted_on_the_last_row() {
        let rows = echo_rows(&"z".repeat(W * 5), 3, W);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], "z".repeat(W));
        assert_eq!(rows[2], format!("{} [+2 lines]", "z".repeat(W - 11)));
        assert!(rows.iter().all(|row| cells(row) <= W), "{rows:?}");
    }

    #[test]
    fn lines_past_the_limit_are_counted_on_the_last_row() {
        assert_eq!(
            echo_rows(&lines(5), 3, W),
            vec!["line 1", "line 2", "line 3 [+2 lines]"]
        );
        assert_eq!(
            echo_rows(&lines(4), 3, W),
            vec!["line 1", "line 2", "line 3 [+1 line]"]
        );
        assert_eq!(echo_rows(&lines(3), 1, W), vec!["line 1 [+2 lines]"]);
        assert_eq!(
            echo_rows(&lines(3), 0, W),
            vec!["line 1 [+2 lines]"],
            "one row at least"
        );
    }

    #[test]
    fn the_count_stays_in_sight_at_the_end_of_a_long_last_row() {
        // The long line is five rows of twenty, and the four of them past
        // the first, with "third", are what is left out.
        let long = "y".repeat(100);
        let rows = echo_rows(&format!("first\n{long}\nthird"), 2, 20);
        assert_eq!(rows[0], "first");
        assert_eq!(rows[1], format!("{} [+5 lines]", "y".repeat(9)));
        assert_eq!(cells(&rows[1]), 20);
    }

    // ----------------------------------------------------------------
    // The frame
    // ----------------------------------------------------------------

    #[test]
    fn a_one_line_message_takes_the_last_row_as_ever() {
        let (ctx, env) = editor();
        ctx.set_echo_message("hello");
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area, vec!["hello"]);
        assert_eq!(frame.echo_top(), H - 1);
        assert_eq!(windows_end(&frame), H - 1);
    }

    #[test]
    fn nothing_to_say_is_one_empty_row() {
        let (ctx, env) = editor();
        ctx.set_echo_message("");
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area, vec![""]);
        assert_eq!(windows_end(&frame), H - 1);
    }

    #[test]
    fn a_message_of_several_lines_takes_a_row_for_each_from_the_windows() {
        let (ctx, env) = editor();
        ctx.set_echo_message("one\ntwo\nthree");
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_message, "one\ntwo\nthree", "the text as it was");
        assert_eq!(frame.echo_area, vec!["one", "two", "three"]);
        assert_eq!(frame.echo_height(), 3);
        assert_eq!(frame.echo_top(), H - 3);
        assert_eq!(windows_end(&frame), H - 3);
    }

    #[test]
    fn a_message_wider_than_the_frame_takes_a_row_for_each_part() {
        let (ctx, env) = editor();
        let long = "m".repeat(W + 20);
        ctx.set_echo_message(&long);
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area, vec!["m".repeat(W), "m".repeat(20)]);
        assert_eq!(frame.echo_top(), H - 2);
        assert_eq!(windows_end(&frame), H - 2);
    }

    #[test]
    fn a_wrapped_message_stops_at_max_echo_area_height() {
        let (ctx, env) = editor();
        run("(setq max-echo-area-height 2)", &env, &ctx);
        ctx.set_echo_message(&"m".repeat(W * 4));
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area.len(), 2);
        assert!(
            frame.echo_area[1].ends_with(" [+2 lines]"),
            "{:?}",
            frame.echo_area
        );
        assert_eq!(windows_end(&frame), H - 2);
    }

    #[test]
    fn the_windows_have_the_rows_back_when_the_message_goes() {
        let (ctx, env) = editor();
        ctx.set_echo_message("one\ntwo\nthree");
        assert_eq!(windows_end(&ctx.snapshot(&env, W, H)), H - 3);
        ctx.set_echo_message("short");
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area, vec!["short"]);
        assert_eq!(windows_end(&frame), H - 1);
    }

    #[test]
    fn an_expired_message_gives_its_rows_back() {
        let (ctx, env) = editor();
        run("(setq echo-message-timeout 0)", &env, &ctx);
        ctx.set_echo_message("one\ntwo\nthree");
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area, vec![""]);
        assert_eq!(windows_end(&frame), H - 1);
    }

    #[test]
    fn a_message_set_from_lisp_grows_the_echo_area() {
        let (ctx, env) = editor();
        run(r#"(set-echo-message "first\nsecond")"#, &env, &ctx);
        assert_eq!(ctx.snapshot(&env, W, H).echo_area, vec!["first", "second"]);
    }

    #[test]
    fn by_default_the_echo_area_grows_to_a_quarter_of_the_frame() {
        let (ctx, env) = editor();
        ctx.set_echo_message(&lines(20));
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area.len(), H / 4);
        assert_eq!(frame.echo_area[0], "line 1");
        assert_eq!(frame.echo_area[5], "line 6 [+14 lines]");
        assert_eq!(windows_end(&frame), H - H / 4);
    }

    #[test]
    fn a_number_of_one_or_more_is_a_number_of_rows() {
        let (ctx, env) = editor();
        run("(setq max-echo-area-height 3)", &env, &ctx);
        ctx.set_echo_message(&lines(20));
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(
            frame.echo_area,
            vec!["line 1", "line 2", "line 3 [+17 lines]"]
        );
        assert_eq!(windows_end(&frame), H - 3);
    }

    #[test]
    fn a_number_under_one_is_a_part_of_the_frame() {
        let (ctx, env) = editor();
        run("(setq max-echo-area-height 0.5)", &env, &ctx);
        ctx.set_echo_message(&lines(20));
        assert_eq!(ctx.snapshot(&env, W, H).echo_area.len(), H / 2);
        assert_eq!(ctx.snapshot(&env, W, 10).echo_area.len(), 5);
    }

    #[test]
    fn one_keeps_every_message_on_a_row() {
        let (ctx, env) = editor();
        run("(setq max-echo-area-height 1)", &env, &ctx);
        ctx.set_echo_message("one\ntwo\nthree");
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area, vec!["one [+2 lines]"]);
        assert_eq!(windows_end(&frame), H - 1);
    }

    #[test]
    fn what_is_not_a_positive_number_counts_as_the_default() {
        let (ctx, env) = editor();
        ctx.set_echo_message(&lines(20));
        for value in ["nil", "0", "-3", "\"ten\""] {
            run(&format!("(setq max-echo-area-height {value})"), &env, &ctx);
            assert_eq!(ctx.snapshot(&env, W, H).echo_area.len(), H / 4, "{value}");
        }
    }

    #[test]
    fn the_windows_keep_a_row_however_large_the_limit() {
        let (ctx, env) = editor();
        run("(setq max-echo-area-height 1000)", &env, &ctx);
        ctx.set_echo_message(&lines(100));
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area.len(), H - 1);
        assert_eq!(frame.echo_top(), 1);
    }

    #[test]
    fn a_tiny_frame_still_has_its_echo_row() {
        let (ctx, env) = editor();
        ctx.set_echo_message("one\ntwo");
        assert_eq!(ctx.snapshot(&env, W, 1).echo_area.len(), 1);
        assert_eq!(ctx.snapshot(&env, W, 2).echo_area.len(), 1);
        assert_eq!(ctx.snapshot(&env, W, 8).echo_area.len(), 2);
    }

    #[test]
    fn a_transient_keymaps_lines_take_the_echo_area_in_place_of_the_message() {
        let (ctx, env) = editor();
        ctx.set_echo_message("a message");
        run(
            "(defun pick () nil)
             (set-transient-keymap (list (cons \"n\" 'pick)) \"n: next\nq: quit\")",
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, W, H);
        assert_eq!(frame.echo_area, vec!["n: next", "q: quit"]);
        assert_eq!(windows_end(&frame), H - 2);
    }

    #[test]
    fn the_minibuffer_stays_just_above_the_echo_area() {
        let (ctx, env) = editor();
        run(
            "(minibuffer-read \"Ask:\" (lambda (answer) nil))",
            &env,
            &ctx,
        );
        ctx.set_echo_message("one\ntwo\nthree");
        let frame = ctx.snapshot(&env, W, H);
        let prompt = frame
            .views
            .iter()
            .find(|view| view.buffer_name == "*Minibuffer*")
            .expect("the minibuffer's window");
        assert_eq!(prompt.rect.height, 1, "it keeps its `minibuffer-lines'");
        assert_eq!(windows_end(&frame), H - 3);
        assert_eq!(
            prompt.rect.y as usize + prompt.rect.height + usize::from(prompt.mode_line.is_some()),
            frame.echo_top()
        );
    }

    #[test]
    fn point_stays_in_sight_in_the_rows_the_windows_keep() {
        let (ctx, env) = editor();
        run(
            "(dotimes (i 200) (insert \"text\\n\")) (goto-char (point-max))",
            &env,
            &ctx,
        );
        ctx.snapshot(&env, W, H);
        ctx.set_echo_message(&lines(6));
        let frame = ctx.snapshot(&env, W, H);
        let view = frame.focused_view().expect("a focused window");
        let (_, row) = view.cursor_rel_pos.expect("point is in sight");
        assert!(row < view.rect.height, "{row} in {}", view.rect.height);
        assert!(view.rect.y as usize + view.rect.height <= frame.echo_top());
    }
}
