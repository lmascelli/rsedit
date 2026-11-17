//! Commenting out, and the arithmetic that makes it come out straight: each
//! command makes several edits, each moving the ones after it, so the region
//! cases use enough lines for a drift to show. The spelling comes from the
//! mode, which may have said nothing, or have only block comments. Then
//! `replace-regions`, the same arithmetic offered to Lisp.
//!
//! Design: `doc/architettura-editor.typ, <modifiche-multiple>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn editor(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        ctx.with_current_buffer_mut(|buf| {
            buf.text = GapBuffer::from(text);
            buf.text.cursor_move(0, 0);
            buf.is_modified = false;
        });
        (ctx, env)
    }

    /// An editor whose buffer is in a mode that comments like Rust.
    fn rusty(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor(text);
        run(
            r#"(make-mode 'probe-mode)
               (set-comment-syntax 'probe-mode '(("//") ("/*" "*/" t)))"#,
            &env,
            &ctx,
        );
        ctx.with_current_buffer_mut(|buf| buf.current_mode = "probe-mode".to_string());
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// Returns whether SRC signals an error.
    fn fails(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).is_err()
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|buf| buf.text.to_string())
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d())
    }

    /// Puts point at OFFSET and the mark at MARK, making a region.
    fn select(ctx: &Ctx, mark: usize, point: usize) {
        run_at(ctx, point);
        ctx.with_current_buffer_mut(|buf| {
            buf.mark = Some(crate::buffer::Mark::new(mark));
        });
    }

    fn run_at(ctx: &Ctx, offset: usize) {
        ctx.with_current_buffer_mut(|buf| {
            let (line, column) = buf.text.cursor_1d_to_2d(offset);
            buf.text.cursor_move(line, column);
        });
    }

    // ----------------------------------------------------------------
    // One line
    // ----------------------------------------------------------------

    #[test]
    fn a_line_is_commented_and_uncommented_by_the_same_key() {
        let (ctx, env) = rusty("let x = 1;\nlet y = 2;\n");
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "// let x = 1;\nlet y = 2;\n");
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "let x = 1;\nlet y = 2;\n");
    }

    #[test]
    fn point_stays_with_the_text_it_was_on() {
        // The commonest way this feels broken: the characters go in in front
        // of the cursor and the cursor does not move, so the next keystroke
        // lands inside the comment marker.
        let (ctx, env) = rusty("let x = 1;\n");
        run_at(&ctx, 4); // on the `x`
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "// let x = 1;\n");
        assert_eq!(point(&ctx), 7, "point should still be on the x");
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(point(&ctx), 4);
    }

    #[test]
    fn an_indented_line_keeps_its_indentation() {
        let (ctx, env) = rusty("    deep();\n");
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "    // deep();\n");
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "    deep();\n");
    }

    #[test]
    fn the_region_is_gone_after_commenting_it() {
        // Not a quirk of this command: every edit in this editor deactivates
        // the mark, and that is what lets an active mark be trusted without
        // ever being adjusted for an edit. Worth pinning here because a
        // second `comment-dwim' then acts on the *line*, which is the one
        // thing about this feature that surprises people.
        let (ctx, env) = rusty("one\ntwo\n");
        select(&ctx, 0, 7);
        run("(comment-dwim)", &env, &ctx);
        assert!(run("(use-region-p)", &env, &ctx).is_nil());
    }

    #[test]
    fn comment_line_ignores_the_region_that_comment_dwim_would_use() {
        let (ctx, env) = rusty("one\ntwo\nthree\n");
        select(&ctx, 0, 8); // the first two lines
        run("(comment-line)", &env, &ctx);
        assert_eq!(text(&ctx), "one\ntwo\n// three\n");
    }

    // ----------------------------------------------------------------
    // A region
    // ----------------------------------------------------------------

    #[test]
    fn every_line_of_the_region_is_commented_at_the_same_column() {
        // Four lines, because the drift this is really about only shows from
        // the second edit onwards -- and at differing indentation, because
        // the openers have to line up rather than follow the code in.
        let (ctx, env) = rusty("fn f() {\n    a();\n    b();\n}\n");
        select(&ctx, 0, 30);
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "// fn f() {\n//     a();\n//     b();\n// }\n");
        // Selected again, because the edit deactivated the mark -- every edit
        // in this editor does, which is what lets an active mark never need
        // adjusting. Toggling back is a fresh selection, not a second press.
        select(&ctx, 0, 40);
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "fn f() {\n    a();\n    b();\n}\n");
    }

    #[test]
    fn a_region_ending_at_the_start_of_a_line_leaves_that_line_alone() {
        // Selecting two lines by dragging down the left margin puts the end
        // at the start of the third, and commenting a line nobody selected is
        // the kind of thing that is only noticed after it is committed.
        let (ctx, env) = rusty("one\ntwo\nthree\n");
        select(&ctx, 0, 8);
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "// one\n// two\nthree\n");
    }

    #[test]
    fn a_blank_line_inside_a_region_is_left_alone() {
        // Nothing on it to comment, and a prefix would leave a line of
        // trailing whitespace behind.
        let (ctx, env) = rusty("one\n\ntwo\n");
        select(&ctx, 0, 9);
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "// one\n\n// two\n");
    }

    #[test]
    fn a_blank_line_on_its_own_is_commented_because_that_is_what_was_asked() {
        let (ctx, env) = rusty("\n");
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "// \n");
    }

    #[test]
    fn a_region_is_uncommented_only_when_all_of_it_is_a_comment() {
        // Half-commented goes *to* commented: it is the state somebody is
        // trying to leave, and uncommenting half of it leaves them worse off.
        let (ctx, env) = rusty("// one\ntwo\n");
        select(&ctx, 0, 11);
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "// // one\n// two\n");
    }

    #[test]
    fn uncommenting_takes_off_one_space_and_no_more() {
        // The space this command puts in, and only that one: the rest is
        // indentation that was inside the comment and belongs to whoever
        // wrote it.
        let (ctx, env) = rusty("//     deep\n");
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "    deep\n");
    }

    #[test]
    fn a_comment_with_no_space_after_the_opener_still_comes_off() {
        let (ctx, env) = rusty("//tight\n");
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "tight\n");
    }

    // ----------------------------------------------------------------
    // The halves, for calling from Lisp
    // ----------------------------------------------------------------

    #[test]
    fn comment_region_and_uncomment_region_do_not_decide() {
        let (ctx, env) = rusty("one\ntwo\n");
        run("(comment-region (point-min) (point-max))", &env, &ctx);
        assert_eq!(text(&ctx), "// one\n// two\n");
        // Again: commenting something already commented comments it twice,
        // which is what a caller that said `comment' asked for.
        run("(comment-region (point-min) (point-max))", &env, &ctx);
        assert_eq!(text(&ctx), "// // one\n// // two\n");
        run("(uncomment-region (point-min) (point-max))", &env, &ctx);
        assert_eq!(text(&ctx), "// one\n// two\n");
    }

    #[test]
    fn uncommenting_lines_that_are_not_comments_leaves_them_alone() {
        let (ctx, env) = rusty("// one\ntwo\n");
        run("(uncomment-region (point-min) (point-max))", &env, &ctx);
        assert_eq!(text(&ctx), "one\ntwo\n");
    }

    // ----------------------------------------------------------------
    // Beside the code
    // ----------------------------------------------------------------

    #[test]
    fn comment_indent_starts_a_comment_at_the_end_of_the_line() {
        let (ctx, env) = rusty("let x = 1;\n");
        run("(comment-indent)", &env, &ctx);
        assert_eq!(text(&ctx), "let x = 1; // \n");
        assert_eq!(point(&ctx), 14, "point should be in the comment");
    }

    #[test]
    fn comment_indent_goes_into_the_comment_that_is_already_there() {
        let (ctx, env) = rusty("let x = 1; // why\n");
        run_at(&ctx, 0);
        run("(comment-indent)", &env, &ctx);
        assert_eq!(text(&ctx), "let x = 1; // why\n", "nothing should be added");
        assert_eq!(point(&ctx), 14);
    }

    #[test]
    fn comment_indent_does_not_leave_the_trailing_space_behind() {
        let (ctx, env) = rusty("let x = 1;   \n");
        run("(comment-indent)", &env, &ctx);
        assert_eq!(text(&ctx), "let x = 1; // \n");
    }

    // ----------------------------------------------------------------
    // What the mode says
    // ----------------------------------------------------------------

    #[test]
    fn a_mode_that_has_said_nothing_is_told_about_rather_than_guessed_at() {
        let (ctx, env) = editor("one\n");
        assert_eq!(run("(comment-dwim)", &env, &ctx), LispExp::nil());
        assert_eq!(text(&ctx), "one\n", "nothing should have been inserted");
        assert!(
            ctx.snapshot(&env, 80, 24).echo_message.contains("comments"),
            "the user should be told: {:?}",
            ctx.snapshot(&env, 80, 24).echo_message
        );
    }

    #[test]
    fn a_language_with_only_block_comments_gets_the_scope_wrapped() {
        let (ctx, env) = editor("a { color: red }\n");
        run(
            r#"(make-mode 'css-probe-mode)
               (set-comment-syntax 'css-probe-mode '(("/*" "*/")))"#,
            &env,
            &ctx,
        );
        ctx.with_current_buffer_mut(|buf| buf.current_mode = "css-probe-mode".to_string());
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "/* a { color: red } */\n");
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "a { color: red }\n");
    }

    #[test]
    fn the_comment_syntax_can_be_read_back_in_the_shape_it_was_given() {
        let (ctx, env) = rusty("");
        let read = run("(comment-syntax)", &env, &ctx);
        assert_eq!(
            format!("{read:?}").matches("//").count(),
            1,
            "the line style should be there: {read:?}"
        );
        // Round-trips: what comes back can be handed straight back in.
        run(
            "(set-comment-syntax 'probe-mode (comment-syntax))",
            &env,
            &ctx,
        );
        run("(comment-dwim)", &env, &ctx);
        assert_eq!(text(&ctx), "// ");
    }

    // ----------------------------------------------------------------
    // Undo, and the buffer's own rules
    // ----------------------------------------------------------------

    #[test]
    fn commenting_a_region_is_one_undo_step() {
        // Four lines commented in four edits must come back in one `undo':
        // walking back through them line by line is the kind of thing that
        // makes people stop trusting the key.
        let (ctx, env) = rusty("one\ntwo\nthree\nfour\n");
        select(&ctx, 0, 19);
        run("(call-interactively \"comment-dwim\")", &env, &ctx);
        assert_eq!(text(&ctx), "// one\n// two\n// three\n// four\n");
        run("(call-interactively \"undo\")", &env, &ctx);
        assert_eq!(text(&ctx), "one\ntwo\nthree\nfour\n");
    }

    #[test]
    fn a_read_only_buffer_is_not_commented() {
        let (ctx, env) = rusty("one\n");
        run("(set-buffer-read-only t)", &env, &ctx);
        assert_eq!(run("(comment-dwim)", &env, &ctx), LispExp::nil());
        assert_eq!(text(&ctx), "one\n");
    }

    #[test]
    fn delete_region_removes_a_span_without_touching_the_kill_ring() {
        let (ctx, env) = editor("one two three");
        let before = run("(kill-ring-length)", &env, &ctx);
        assert!(!run("(delete-region 4 8)", &env, &ctx).is_nil());
        assert_eq!(text(&ctx), "one three");
        assert_eq!(run("(kill-ring-length)", &env, &ctx), before);
    }

    #[test]
    fn delete_region_is_content_with_an_empty_span_and_a_backwards_one() {
        let (ctx, env) = editor("abc");
        assert_eq!(run("(delete-region 2 2)", &env, &ctx), LispExp::nil());
        assert_eq!(text(&ctx), "abc");
        assert!(!run("(delete-region 3 1)", &env, &ctx).is_nil());
        assert_eq!(text(&ctx), "a");
    }

    // ----------------------------------------------------------------
    // Several replacements in one step
    // ----------------------------------------------------------------

    /// The buffer "let a = 1;\nlet b = 2;\n" with `a' renamed to `alpha' and
    /// `2' made `20', point at AT, and where point is afterwards.
    fn renamed_with_point_at(at: usize) -> (String, usize) {
        let (ctx, env) = editor("let a = 1;\nlet b = 2;\n");
        run_at(&ctx, at);
        run(
            r#"(replace-regions '((19 20 "20") (4 5 "alpha")))"#,
            &env,
            &ctx,
        );
        (text(&ctx), point(&ctx))
    }

    #[test]
    fn replace_regions_makes_every_replacement_given_in_any_order() {
        let (text, _) = renamed_with_point_at(0);
        assert_eq!(text, "let alpha = 1;\nlet b = 20;\n");
    }

    #[test]
    fn point_before_the_replacements_stays_where_it_was() {
        assert_eq!(renamed_with_point_at(2).1, 2);
        // On the start of one: what is replaced comes after it.
        assert_eq!(renamed_with_point_at(4).1, 4);
    }

    #[test]
    fn point_inside_a_replacement_goes_to_its_start() {
        // Inside "2", between the two replacements' effects: the first one
        // grew the text by four.
        let (ctx, env) = editor("let a = 1;\nlet b = 23;\n");
        run_at(&ctx, 20);
        run(
            r#"(replace-regions '((4 5 "alpha") (19 21 "7")))"#,
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "let alpha = 1;\nlet b = 7;\n");
        assert_eq!(point(&ctx), 23);
    }

    #[test]
    fn point_after_the_replacements_moves_with_its_text() {
        // On the `;' after "2", and at the end of the buffer.
        assert_eq!(renamed_with_point_at(20).1, 25);
        assert_eq!(renamed_with_point_at(22).1, 27);
        // Between the two: only the first one counts.
        assert_eq!(renamed_with_point_at(11).1, 15);
    }

    #[test]
    fn an_insertion_at_point_leaves_point_after_it() {
        let (ctx, env) = editor("ab");
        run_at(&ctx, 1);
        run(r#"(replace-regions '((1 1 "xy")))"#, &env, &ctx);
        assert_eq!(text(&ctx), "axyb");
        assert_eq!(point(&ctx), 3);
    }

    #[test]
    fn replacements_that_only_touch_are_both_made_in_the_order_given() {
        let (ctx, env) = editor("abcd");
        run(
            r#"(replace-regions '((2 4 "Z") (0 2 "X") (2 2 "y")))"#,
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "XyZ");
    }

    #[test]
    fn overlapping_replacements_are_refused_before_anything_changes() {
        let (ctx, env) = editor("abcdef");
        assert!(fails(
            r#"(replace-regions '((0 1 "x") (2 4 "y") (3 5 "z")))"#,
            &env,
            &ctx
        ));
        assert_eq!(text(&ctx), "abcdef");
    }

    #[test]
    fn a_replacement_outside_the_buffer_is_refused_before_anything_changes() {
        let (ctx, env) = editor("abc");
        assert!(fails(
            r#"(replace-regions '((0 1 "x") (2 9 "y")))"#,
            &env,
            &ctx
        ));
        assert!(fails(r#"(replace-regions '((2 1 "y")))"#, &env, &ctx));
        assert!(fails(r#"(replace-regions '((0 1 x)))"#, &env, &ctx));
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn a_read_only_buffer_refuses_replace_regions() {
        let (ctx, env) = editor("abc");
        run("(set-buffer-read-only t)", &env, &ctx);
        assert!(fails(r#"(replace-regions '((0 1 "x")))"#, &env, &ctx));
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn replace_regions_with_nothing_to_do_returns_nil() {
        let (ctx, env) = editor("abc");
        assert_eq!(run("(replace-regions nil)", &env, &ctx), LispExp::nil());
        assert_eq!(
            run(r#"(replace-regions '((1 1 "")))"#, &env, &ctx),
            LispExp::nil()
        );
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn replace_regions_is_one_undo_step_of_its_own() {
        let (ctx, env) = editor("one two three");
        run(r#"(insert "> ")"#, &env, &ctx);
        run(
            r#"(replace-regions '((2 5 "1") (6 9 "2") (10 15 "3")))"#,
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "> 1 2 3");
        run("(undo)", &env, &ctx);
        assert_eq!(text(&ctx), "> one two three");
    }
}
