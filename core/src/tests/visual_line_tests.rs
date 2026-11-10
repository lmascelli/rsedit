//! Word wrapping is a view: text and undo do not change, and drawing, motion,
//! scrolling, highlights and mouse events agree on the original positions.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseKind};
    use crate::tests::shipped::module_source;
    use crate::ui::width::cells;
    use crate::ui::{Division, Face, Orientation, RenderableWindowView};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("{src}: {why:?}"))
    }

    fn editor(text: &str, width: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("editor");
        env.set_variable("frame-width".into(), LispExp::number(width as f64));
        env.set_variable("frame-height".into(), LispExp::number(20.0));
        for module in ["commands", "debug", "common-keymaps", "visual-line"] {
            run(&module_source(module), &env, &ctx);
        }
        ctx.with_current_buffer_mut(|buf| {
            buf.text = GapBuffer::from(text);
            buf.text.cursor_move(0, 0);
        });
        (ctx, env)
    }

    fn wrapped(text: &str, width: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor(text, width);
        run("(visual-line-mode 1)", &env, &ctx);
        (ctx, env)
    }

    fn view(ctx: &Ctx, env: &Arc<Env<Ctx>>, width: usize, height: usize) -> RenderableWindowView {
        ctx.snapshot(env, width, height).views.remove(0)
    }

    fn point(ctx: &Ctx) -> (usize, usize) {
        ctx.with_current_buffer(|buf| buf.text.cursor_pos())
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode, ctrl: bool) {
        ctx.handle_key_event(
            KeyEvent {
                code,
                modifiers: KeyModifiers {
                    ctrl,
                    ..Default::default()
                },
            },
            env,
        );
    }

    fn mouse(ctx: &Ctx, env: &Arc<Env<Ctx>>, kind: MouseKind, x: usize, y: usize) {
        ctx.handle_mouse_event(
            MouseEvent {
                kind,
                column: x as u16,
                row: y as u16,
                modifiers: Default::default(),
            },
            env,
        );
    }

    #[test]
    fn the_mode_starts_off_and_is_local_to_each_buffer() {
        let (ctx, env) = editor("alpha beta gamma", 8);
        assert!(run("(minor-mode-p 'visual-line-mode)", &env, &ctx).is_nil());
        assert_eq!(view(&ctx, &env, 8, 20).lines[0], "alpha be");
        run("(visual-line-mode)", &env, &ctx);
        assert!(run("(minor-mode-p 'visual-line-mode)", &env, &ctx).is_truthy());
        assert!(view(&ctx, &env, 8, 20).mode_line.unwrap().contains("Wrap"));
        run(
            r#"(buffer-create "other") (switch-to-buffer "other")"#,
            &env,
            &ctx,
        );
        assert!(run("(minor-mode-p 'visual-line-mode)", &env, &ctx).is_nil());
        run(
            r#"(switch-to-buffer "*scratch*") (visual-line-mode 0)"#,
            &env,
            &ctx,
        );
        assert_eq!(view(&ctx, &env, 8, 20).lines[0], "alpha be");
        run("(visual-line-mode 1) (visual-line-mode -1)", &env, &ctx);
        assert!(run("(minor-mode-p 'visual-line-mode)", &env, &ctx).is_nil());
    }

    #[test]
    fn wrapping_never_changes_text_modified_state_or_undo() {
        let (ctx, env) = editor("alpha beta gamma\n", 8);
        run("(visual-line-mode 1)", &env, &ctx);
        let _ = view(&ctx, &env, 8, 20);
        let _ = view(&ctx, &env, 5, 20);
        run("(visual-line-mode 0) (undo)", &env, &ctx);
        ctx.with_current_buffer(|buf| {
            assert_eq!(buf.text.to_string(), "alpha beta gamma\n");
            assert!(!buf.is_modified);
            assert_eq!(buf.version, 0);
        });
        run(r#"(insert "X") (visual-line-mode 1) (undo)"#, &env, &ctx);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.to_string()),
            "alpha beta gamma\n"
        );
    }

    #[test]
    fn words_long_words_empty_lines_and_exact_width_endings() {
        for (text, width, expected) in [
            ("one two three", 8, vec!["one two ", "three"]),
            ("alpha beta gamma", 8, vec!["alpha ", "beta ", "gamma"]),
            ("abcdefghij", 5, vec!["abcde", "fghij", ""]),
            ("", 5, vec![""]),
            ("\n\n", 5, vec!["", "", ""]),
            ("abcde", 5, vec!["abcde", ""]),
        ] {
            let (ctx, env) = wrapped(text, width);
            assert_eq!(view(&ctx, &env, width, 20).lines, expected, "{text:?}");
        }
    }

    #[test]
    fn tabs_wide_characters_and_combining_accents_use_screen_cells() {
        for (text, width, expected) in [
            ("a\tbc", 5, vec!["a   ", "bc"]),
            ("éé 界界 xyz", 6, vec!["éé ", "界界 ", "xyz"]),
            ("a\u{301}bcdef", 3, vec!["a\u{301}bc", "def", ""]),
        ] {
            let (ctx, env) = wrapped(text, width);
            let drawn = view(&ctx, &env, width, 20);
            assert_eq!(drawn.lines, expected, "{text:?}");
            assert!(drawn.lines.iter().all(|line| cells(line) <= width));
        }
        let (ctx, env) = wrapped("a\tbc", 5);
        run("(goto-char 2)", &env, &ctx);
        assert_eq!(view(&ctx, &env, 5, 20).cursor_rel_pos, Some((0, 1)));
    }

    #[test]
    fn the_cursor_at_an_exact_width_end_is_visible() {
        let (ctx, env) = wrapped("abcdefghij", 5);
        run("(end-of-buffer)", &env, &ctx);
        assert_eq!(view(&ctx, &env, 5, 20).cursor_rel_pos, Some((0, 2)));
    }

    #[test]
    fn keys_move_by_visual_rows_and_turn_back_into_logical_motion() {
        let (ctx, env) = wrapped("alpha beta gamma\nnext", 8);
        let _ = view(&ctx, &env, 8, 20);
        run("(goto-char 2)", &env, &ctx);
        press(&ctx, &env, KeyCode::Down, false);
        assert_eq!(point(&ctx), (0, 8));
        press(&ctx, &env, KeyCode::Char('n'), true);
        assert_eq!(point(&ctx), (0, 13));
        press(&ctx, &env, KeyCode::Up, false);
        assert_eq!(point(&ctx), (0, 8));
        press(&ctx, &env, KeyCode::Home, false);
        assert_eq!(point(&ctx), (0, 6));
        press(&ctx, &env, KeyCode::Char('e'), true);
        assert_eq!(point(&ctx), (0, 10));
        press(&ctx, &env, KeyCode::Char('a'), true);
        assert_eq!(point(&ctx), (0, 6));
        run("(visual-line-mode 0)", &env, &ctx);
        press(&ctx, &env, KeyCode::Down, false);
        assert_eq!(point(&ctx).0, 1);
        press(&ctx, &env, KeyCode::End, false);
        assert_eq!(point(&ctx), (1, 4));
    }

    #[test]
    fn vertical_moves_keep_the_goal_across_short_rows_and_reset_after_horizontal_motion() {
        let (ctx, env) = wrapped("abcdefgh\nx\nabcdefgh", 20);
        let _ = view(&ctx, &env, 20, 20);
        run("(goto-char 6)", &env, &ctx);
        press(&ctx, &env, KeyCode::Down, false);
        assert_eq!(point(&ctx), (1, 1));
        press(&ctx, &env, KeyCode::Down, false);
        assert_eq!(point(&ctx), (2, 6));
        press(&ctx, &env, KeyCode::Left, false);
        press(&ctx, &env, KeyCode::Up, false);
        press(&ctx, &env, KeyCode::Up, false);
        assert_eq!(point(&ctx), (0, 5));
    }

    #[test]
    fn logical_commands_remain_available_and_visual_counts_clamp_at_buffer_edges() {
        let (ctx, env) = wrapped("alpha beta gamma\nnext", 8);
        run("(goto-char 2) (next-line)", &env, &ctx);
        assert_eq!(point(&ctx), (1, 2));
        run("(previous-visual-line 2)", &env, &ctx);
        assert_eq!(point(&ctx), (0, 8));
        run("(next-visual-line 1000000)", &env, &ctx);
        assert_eq!(point(&ctx), (1, 2));
        run("(next-visual-line -1000000)", &env, &ctx);
        assert_eq!(point(&ctx), (0, 2));
    }

    #[test]
    fn scroll_and_recenter_can_stay_inside_a_single_logical_line() {
        let text: String = (0..40).map(|n| format!("{n:02} ")).collect();
        let (ctx, env) = wrapped(&text, 8);
        let first = view(&ctx, &env, 8, 6);
        let id = ctx.get_focused_window_id();
        assert!(ctx.scroll_window_by(id, 3));
        let after = view(&ctx, &env, 8, 6);
        assert_ne!(first.lines[0], after.lines[0]);
        assert_eq!(point(&ctx).0, 0);
        assert!(after.cursor_rel_pos.is_some());
        run("(scroll-up-command)", &env, &ctx);
        assert_ne!(view(&ctx, &env, 8, 6).lines[0], after.lines[0]);
        run("(end-of-buffer) (recenter)", &env, &ctx);
        let centered = view(&ctx, &env, 8, 6);
        assert_eq!(centered.cursor_rel_pos.unwrap().1, 2);
    }

    #[test]
    fn resizing_reflows_around_point_without_moving_it() {
        let (ctx, env) = wrapped("one two three four five six seven eight nine ten", 20);
        run("(goto-char 35)", &env, &ctx);
        let _ = view(&ctx, &env, 20, 6);
        for width in [7, 3, 30, 1] {
            let drawn = view(&ctx, &env, width, 6);
            let (x, y) = drawn.cursor_rel_pos.expect("visible point after resize");
            assert!(x < width && y < drawn.rect.height, "{width}: {x},{y}");
            assert_eq!(ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()), 35);
        }
        let _ = view(&ctx, &env, 0, 0);
    }

    #[test]
    fn two_windows_wrap_the_same_buffer_to_their_own_widths() {
        let (ctx, env) = wrapped("one two three four five six seven eight", 28);
        ctx.split_focused_window(Orientation::Vertical, Division::FirstFixed(10));
        let frame = ctx.snapshot(&env, 28, 20);
        assert_eq!(frame.views.len(), 2);
        assert_ne!(frame.views[0].lines, frame.views[1].lines);
        for view in frame.views {
            assert!(view.lines.iter().all(|line| cells(line) <= view.rect.width));
        }
    }

    #[test]
    fn the_gutter_numbers_logical_lines_and_spends_width_before_wrapping() {
        let (ctx, env) = wrapped("one two three four five six\nsecond", 20);
        run("(setq display-line-numbers t)", &env, &ctx);
        let drawn = view(&ctx, &env, 20, 20);
        assert!(drawn.gutter_width > 0);
        assert_eq!(drawn.gutter[0].text.trim(), "1");
        assert_eq!(drawn.gutter[1].text.trim(), "");
        let second = drawn
            .lines
            .iter()
            .position(|line| line == "second")
            .unwrap();
        assert_eq!(drawn.gutter[second].text.trim(), "2");
        assert!(
            drawn
                .lines
                .iter()
                .all(|line| cells(line) <= drawn.rect.width)
        );
    }

    #[test]
    fn regions_and_overlays_are_placed_on_every_continuation() {
        let (ctx, env) = wrapped("alpha beta gamma", 8);
        run(
            "(make-overlay 3 13 'error) (set-mark 2) (goto-char 14)",
            &env,
            &ctx,
        );
        let drawn = view(&ctx, &env, 8, 20);
        let regions: Vec<_> = drawn
            .highlights
            .iter()
            .filter(|h| h.face == Face::REGION)
            .map(|h| (h.row, h.start_col, h.end_col))
            .collect();
        assert_eq!(regions, [(0, 2, 6), (1, 0, 5), (2, 0, 3)]);
        let errors: Vec<_> = drawn
            .highlights
            .iter()
            .filter(|h| h.face == Face::intern("error"))
            .map(|h| (h.row, h.start_col, h.end_col))
            .collect();
        assert_eq!(errors, [(0, 3, 6), (1, 0, 5), (2, 0, 2)]);
    }

    #[test]
    fn clicks_and_drags_on_continuations_use_original_offsets() {
        let (ctx, env) = wrapped("alpha beta gamma", 8);
        run("(setq mouse-mode t)", &env, &ctx);
        let _ = view(&ctx, &env, 8, 20);
        mouse(&ctx, &env, MouseKind::Down(MouseButton::Left), 2, 1);
        assert_eq!(point(&ctx), (0, 8));
        mouse(&ctx, &env, MouseKind::Drag(MouseButton::Left), 3, 2);
        assert_eq!(point(&ctx), (0, 14));
        assert_eq!(run("(region-beginning)", &env, &ctx), LispExp::number(8.0));
        assert_eq!(run("(region-end)", &env, &ctx), LispExp::number(14.0));
    }

    #[test]
    fn virtual_text_wraps_and_preserves_cursor_click_round_trips() {
        let (ctx, env) = wrapped("abc def", 6);
        run(
            r#"(make-virtual-text 3 " hint ") (setq mouse-mode t)"#,
            &env,
            &ctx,
        );
        assert_eq!(view(&ctx, &env, 6, 20).lines.concat(), "abc hint  def");
        for column in 0..=7 {
            run(&format!("(goto-char {column})"), &env, &ctx);
            let drawn = view(&ctx, &env, 6, 20);
            let (x, y) = drawn.cursor_rel_pos.unwrap();
            assert!(x < 6);
            mouse(&ctx, &env, MouseKind::Down(MouseButton::Left), x, y);
            mouse(&ctx, &env, MouseKind::Up(MouseButton::Left), x, y);
            assert_eq!(point(&ctx), (0, column));
        }
        assert!(
            view(&ctx, &env, 6, 20)
                .highlights
                .iter()
                .any(|h| h.face == Face::intern("comment"))
        );
    }

    #[test]
    fn motion_crosses_continuations_that_have_no_buffer_position() {
        let (ctx, env) = wrapped("abcdef", 4);
        run(
            r#"(make-virtual-text 2 "XXXXXXXXXX") (goto-char 2)"#,
            &env,
            &ctx,
        );
        let _ = view(&ctx, &env, 4, 20);
        press(&ctx, &env, KeyCode::Up, false);
        assert!(point(&ctx).1 < 2);
        press(&ctx, &env, KeyCode::Down, false);
        assert!(point(&ctx).1 >= 2);
    }

    #[test]
    fn edits_reflow_continuations_and_disabling_the_mode_clears_each_windows_mapping() {
        let (ctx, env) = wrapped("alpha beta gamma delta", 8);
        ctx.split_focused_window(Orientation::Vertical, Division::Ratio(0.5));
        let _ = ctx.snapshot(&env, 20, 20);
        run(r#"(goto-char 6) (insert "new ")"#, &env, &ctx);
        let changed = ctx.snapshot(&env, 20, 20);
        assert!(
            changed
                .views
                .iter()
                .all(|view| view.lines.concat() == "alpha new beta gamma delta")
        );
        run("(visual-line-mode 0)", &env, &ctx);
        let _ = ctx.snapshot(&env, 20, 20);
        ctx.windows(|windows| {
            for id in windows.root().window_ids() {
                assert!(windows.root().window(id).unwrap().wrap.is_none());
            }
        });
    }

    #[test]
    fn every_buffer_position_maps_to_a_visible_cell_across_narrow_widths() {
        use crate::buffer::VirtualTextTable;
        use crate::ui::visual_lines::{VisualLines, VisualPoint};
        for text in [
            "one two three",
            "a\tbc",
            "éé 界界 xyz",
            "    abc",
            "abcde\n\n def",
        ] {
            let text = GapBuffer::from(text);
            for width in 1..=12 {
                for hints in [false, true] {
                    let mut virtual_text = VirtualTextTable::default();
                    if hints {
                        virtual_text.insert(2, " a long hint ", Face::intern("comment"), 0, "test");
                    }
                    let visual = VisualLines::new(&text, &virtual_text, width, 4);
                    let layout = visual.compose(VisualPoint::default(), usize::MAX);
                    assert!(
                        layout
                            .visible_text(0, width)
                            .iter()
                            .all(|row| cells(row) <= width)
                    );
                    for position in 0..=text.len() {
                        let (line, column) = text.cursor_1d_to_2d(position);
                        let (_, row) = layout
                            .row_of_position(line, column)
                            .expect("point has a row");
                        let x = row.to_screen(column);
                        assert!(
                            x < width,
                            "{width} cells, position {position}, hints {hints}"
                        );
                        assert_eq!(row.to_buffer(x), column);
                    }
                }
            }
        }
    }
}
