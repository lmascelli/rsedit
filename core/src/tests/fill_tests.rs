//! Filling: words laid out to a column behind a prefix, a paragraph and a
//! comment found and filled with point kept on its word, the column the
//! buffer, the mode or the variable says, and lines broken as they are typed.
//!
//! Design: `doc/architettura-editor.typ, <riempimento>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent};
    use crate::text::fill::{Measure, break_at, comment_prefix, lay_out, list_marker};
    use crate::ui::{Face, width::cells};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// An editor with TEXT in *scratch*, point at POINT, and the column at
    /// WIDTH.
    fn editor_with(text: &str, point: usize, width: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        env.set_variable("fill-column".into(), LispExp::number(width as f64));
        set_text(&ctx, text, point);
        (ctx, env)
    }

    fn set_text(ctx: &Ctx, text: &str, point: usize) {
        ctx.with_current_buffer_mut(|b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(point);
            b.text.cursor_move(line, col);
            b.is_modified = false;
        });
    }

    /// Makes *scratch* a buffer of a mode that comments with `//`.
    fn as_code(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            r#"(make-mode 'slash-mode)
               (set-comment-syntax 'slash-mode '(("//")))
               (set-major-mode 'slash-mode)"#,
            env,
            ctx,
        );
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|b| b.text.to_string())
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_current_buffer(|b| b.text.cursor_pos_1d())
    }

    fn with_fill_module(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/fill.lisp"),
        ] {
            eval_str(source, env, ctx).expect("loading the shipped lisp");
        }
    }

    fn type_keys(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for c in text.chars() {
            ctx.handle_key_event(KeyEvent::new(KeyCode::Char(c)), env);
        }
    }

    fn measure(width: usize) -> Measure<fn(char) -> usize> {
        Measure {
            width,
            tab: 8,
            cells: |_| 1,
        }
    }

    // ----------------------------------------------------------------
    // Laying words out
    // ----------------------------------------------------------------

    #[test]
    fn words_go_on_a_line_while_they_fit() {
        let words = ["one", "two", "three", "four"];
        assert_eq!(lay_out(&words, "", "", &measure(9)), "one two\nthree\nfour");
        assert_eq!(lay_out(&words, "", "", &measure(13)), "one two three\nfour");
    }

    #[test]
    fn a_word_wider_than_a_line_has_a_line_of_its_own() {
        let words = ["a", "enormous", "b"];
        assert_eq!(lay_out(&words, "", "", &measure(4)), "a\nenormous\nb");
    }

    #[test]
    fn the_prefixes_count_towards_the_width() {
        let words = ["aa", "bb", "cc"];
        assert_eq!(
            lay_out(&words, "// ", "//   ", &measure(9)),
            "// aa bb\n//   cc"
        );
        assert_eq!(
            lay_out(&["aa", "bb"], "\t", "\t", &measure(12)),
            "\taa\n\tbb",
            "a Tab reaches column 8"
        );
    }

    #[test]
    fn a_comment_prefix_takes_the_opener_its_repeats_and_the_blanks_after() {
        let openers = vec!["//".to_string(), ";".to_string()];
        assert_eq!(comment_prefix("  // text", &openers), Some("  // "));
        assert_eq!(comment_prefix("/// doc", &openers), Some("/// "));
        assert_eq!(comment_prefix("//! inner", &openers), Some("//! "));
        assert_eq!(comment_prefix(";;; head", &openers), Some(";;; "));
        assert_eq!(comment_prefix("//", &openers), Some("//"));
        assert_eq!(comment_prefix("let x; // no", &openers), None);
    }

    #[test]
    fn a_line_that_fits_is_not_broken() {
        assert_eq!(break_at("one two ", "", &measure(7)), None);
        assert_eq!(break_at("one twoo ", "", &measure(7)), Some((3, 4)));
    }

    #[test]
    fn the_break_is_the_last_blank_whose_word_fits() {
        assert_eq!(break_at("aa bb cc dd ", "", &measure(7)), Some((5, 6)));
        assert_eq!(break_at("// aa bb", "// ", &measure(6)), Some((5, 6)));
    }

    #[test]
    fn a_first_word_too_wide_is_broken_after() {
        assert_eq!(break_at("enormous a", "", &measure(4)), Some((8, 9)));
        assert_eq!(break_at("enormous", "", &measure(4)), None, "no blank");
    }

    // ----------------------------------------------------------------
    // fill-paragraph
    // ----------------------------------------------------------------

    #[test]
    fn a_paragraph_is_laid_out_again_to_the_column() {
        let (ctx, env) = editor_with("one two\nthree four five six\nseven", 0, 14);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "one two three\nfour five six\nseven");
    }

    #[test]
    fn only_the_paragraph_point_is_in_is_filled() {
        let (ctx, env) = editor_with("a\nb\n\nc\nd\n", 0, 70);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "a b\n\nc\nd\n");
    }

    #[test]
    fn a_line_indented_otherwise_is_another_paragraph() {
        // A heading directly above its text, as the manual pages are.
        let (ctx, env) = editor_with("HEADING\n    one\n    two\n", 10, 70);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "HEADING\n    one two\n");
    }

    #[test]
    fn point_stays_on_its_word() {
        let source = "alpha beta\ngamma delta";
        let at = source.find("mma").expect("gamma");
        let (ctx, env) = editor_with(source, at, 70);
        run("(fill-paragraph)", &env, &ctx);
        let filled = text(&ctx);
        assert_eq!(filled, "alpha beta gamma delta");
        assert_eq!(&filled[point(&ctx)..point(&ctx) + 3], "mma");
    }

    #[test]
    fn a_paragraph_filled_already_is_left_unmodified() {
        let (ctx, env) = editor_with("one two\nthree", 0, 7);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "one two\nthree");
        assert!(!ctx.with_current_buffer(|b| b.is_modified));
    }

    #[test]
    fn one_undo_takes_a_fill_back() {
        let (ctx, env) = editor_with("a\nb\nc", 0, 70);
        run(
            "(undo-boundary) (fill-paragraph) (undo-boundary)",
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "a b c");
        run("(undo)", &env, &ctx);
        assert_eq!(text(&ctx), "a\nb\nc");
    }

    // ----------------------------------------------------------------
    // Comments
    // ----------------------------------------------------------------

    #[test]
    fn in_code_the_comment_is_filled_behind_its_opener() {
        let source = "    // one two\n    // three four five\nlet x = 1;\n";
        let (ctx, env) = editor_with(source, 6, 22);
        as_code(&env, &ctx);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(
            text(&ctx),
            "    // one two three\n    // four five\nlet x = 1;\n"
        );
    }

    #[test]
    fn code_is_not_filled_and_says_why() {
        let (ctx, env) = editor_with("let a = 1;\nlet b = 2;\n", 0, 70);
        as_code(&env, &ctx);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "let a = 1;\nlet b = 2;\n");
        assert!(ctx.get_echo_message().contains("Not in a comment"));
    }

    #[test]
    fn two_kinds_of_comment_and_an_empty_one_end_a_paragraph() {
        let source = "/// doc\n/// more\n// plain\n//\n// after\n";
        let (ctx, env) = editor_with(source, 0, 70);
        as_code(&env, &ctx);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "/// doc more\n// plain\n//\n// after\n");
    }

    #[test]
    fn a_continuation_keeps_the_indentation_of_the_second_line() {
        let source = "// - one two three\n//   four\n";
        let (ctx, env) = editor_with(source, 0, 14);
        as_code(&env, &ctx);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "// - one two\n//   three\n//   four\n");
    }

    // ----------------------------------------------------------------
    // fill-region
    // ----------------------------------------------------------------

    #[test]
    fn fill_region_fills_every_paragraph_it_touches() {
        let source = "a\nb\n\nc\nd\n\ne\nf\n";
        let (ctx, env) = editor_with(source, 0, 70);
        // From the a to inside the c: the second paragraph is touched, the
        // third is not.
        let changed = run("(fill-region 0 6)", &env, &ctx);
        assert_eq!(changed, LispExp::number(2.0));
        assert_eq!(text(&ctx), "a b\n\nc d\n\ne\nf\n");
    }

    // ----------------------------------------------------------------
    // The column
    // ----------------------------------------------------------------

    #[test]
    fn set_fill_column_is_the_buffers_own() {
        let (ctx, env) = editor_with("aa bb cc", 0, 70);
        assert_eq!(run("(set-fill-column 5)", &env, &ctx), LispExp::number(5.0));
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "aa bb\ncc");
        assert_eq!(
            run("(buffer-get 'fill-column)", &env, &ctx),
            LispExp::number(5.0)
        );
        run(r#"(switch-to-buffer (buffer-create "other"))"#, &env, &ctx);
        set_text(&ctx, "aa bb cc", 0);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(
            text(&ctx),
            "aa bb cc",
            "another buffer keeps the variable's"
        );
    }

    #[test]
    fn set_fill_column_with_no_number_takes_points_column() {
        let (ctx, env) = editor_with("aa bb cc dd", 5, 70);
        assert_eq!(
            run("(set-fill-column nil)", &env, &ctx),
            LispExp::number(5.0)
        );
        assert!(ctx.get_echo_message().starts_with("Fill column set to 5"));
    }

    #[test]
    fn a_modes_own_column_wins_over_the_variable() {
        let (ctx, env) = editor_with("aa bb cc", 0, 70);
        run("(put 'fundamental-mode 'fill-column 5)", &env, &ctx);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "aa bb\ncc");
    }

    // ----------------------------------------------------------------
    // Breaking as you type
    // ----------------------------------------------------------------

    #[test]
    fn do_auto_fill_breaks_at_the_last_blank_that_fits() {
        let (ctx, env) = editor_with("  aa bb cc ", 11, 8);
        assert!(run("(do-auto-fill)", &env, &ctx).is_truthy());
        assert_eq!(text(&ctx), "  aa bb\n  cc ");
        assert_eq!(point(&ctx), text(&ctx).chars().count());
    }

    #[test]
    fn do_auto_fill_leaves_a_line_that_fits() {
        let (ctx, env) = editor_with("aa bb ", 6, 5);
        assert!(run("(do-auto-fill)", &env, &ctx).is_nil(), "bb ends at 5");
        assert_eq!(text(&ctx), "aa bb ");
    }

    #[test]
    fn in_a_comment_the_new_line_carries_the_opener_and_in_code_nothing_breaks() {
        let (ctx, env) = editor_with("// aa bb cc ", 12, 9);
        as_code(&env, &ctx);
        assert!(run("(do-auto-fill)", &env, &ctx).is_truthy());
        assert_eq!(text(&ctx), "// aa bb\n// cc ");
        set_text(&ctx, "let aa = bb + cc ", 17);
        assert!(run("(do-auto-fill)", &env, &ctx).is_nil());
    }

    #[test]
    fn auto_fill_mode_breaks_lines_as_a_space_is_typed() {
        let (ctx, env) = editor_with("", 0, 10);
        with_fill_module(&env, &ctx);
        run("(auto-fill-mode)", &env, &ctx);
        type_keys("one two three four ", &env, &ctx);
        assert_eq!(text(&ctx), "one two\nthree four ");
        run("(auto-fill-mode)", &env, &ctx);
        type_keys("five six seven ", &env, &ctx);
        assert_eq!(
            text(&ctx),
            "one two\nthree four five six seven ",
            "off again"
        );
    }

    #[test]
    fn minor_mode_enable_turns_auto_fill_on_in_new_buffers_of_the_mode() {
        let (ctx, env) = editor_with("", 0, 10);
        with_fill_module(&env, &ctx);
        run(
            "(minor-mode-enable 'auto-fill-mode 'fundamental-mode)",
            &env,
            &ctx,
        );
        run(r#"(switch-to-buffer (buffer-create "notes"))"#, &env, &ctx);
        assert!(run("(minor-mode-p 'auto-fill-mode)", &env, &ctx).is_truthy());
    }

    #[test]
    fn list_markers_require_a_separator_and_accept_unicode_bullets() {
        for (source, marker) in [("- text", "- "), ("• text", "• "), ("12)\ttext", "12)\t")] {
            assert_eq!(list_marker(source), Some(marker));
        }
        for source in ["-word", "3.14 pi", "heading", "---", "1."] {
            assert_eq!(list_marker(source), None, "{source}");
        }
    }

    #[test]
    fn fill_region_keeps_bullets_numbers_and_nested_items_separate() {
        let source = "Intro\n- one two three four\n- five six\n  + nested item words\n1. seven eight nine ten\n2) eleven twelve\n";
        let (ctx, env) = editor_with(source, 0, 14);
        run("(fill-region (point-min) (point-max))", &env, &ctx);
        assert_eq!(
            text(&ctx),
            "Intro\n- one two\n  three four\n- five six\n  + nested\n    item words\n1. seven eight\n   nine ten\n2) eleven\n   twelve\n"
        );
        // A second fill must retain the same paragraph boundaries.
        run("(fill-region (point-min) (point-max))", &env, &ctx);
        assert_eq!(
            text(&ctx),
            "Intro\n- one two\n  three four\n- five six\n  + nested\n    item words\n1. seven eight\n   nine ten\n2) eleven\n   twelve\n"
        );
    }

    #[test]
    fn filling_from_a_continuation_finds_the_item_and_keeps_point_on_its_word() {
        let source = "heading\n• uno due\n  città tre quattro\n• altro\n";
        let at = source[..source.find("ttà").unwrap()].chars().count();
        let (ctx, env) = editor_with(source, at, 16);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(
            text(&ctx),
            "heading\n• uno due città\n  tre quattro\n• altro\n"
        );
        assert_eq!(
            text(&ctx)
                .chars()
                .skip(point(&ctx))
                .take(3)
                .collect::<String>(),
            "ttà"
        );
    }

    #[test]
    fn list_continuations_match_indentation_in_display_cells() {
        let (ctx, env) = editor_with("123. aa\n\t bb cc dd\n2. ee\n", 9, 11);
        run("(setq tab-width 4) (fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "123. aa bb\n     cc dd\n2. ee\n");
    }

    #[test]
    fn list_separation_can_be_disabled_without_disabling_comments() {
        let source = "- aa\n- bb\n";
        let (ctx, env) = editor_with(source, 0, 70);
        run(
            "(setq fill-separate-list-items nil) (fill-paragraph)",
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "- aa - bb\n");
        as_code(&env, &ctx);
        set_text(&ctx, "// one\n// two\n", 0);
        run("(fill-paragraph)", &env, &ctx);
        assert_eq!(text(&ctx), "// one two\n");
    }

    #[test]
    fn comment_items_have_hanging_prefixes_even_without_a_second_line() {
        let source = "// intro\n// - one two three four\n// - five six\nlet x = 1;\n";
        let (ctx, env) = editor_with(source, 0, 16);
        as_code(&env, &ctx);
        run("(fill-region (point-min) (point-max))", &env, &ctx);
        assert_eq!(
            text(&ctx),
            "// intro\n// - one two\n//   three four\n// - five six\nlet x = 1;\n"
        );
    }

    #[test]
    fn comment_lines_can_be_filled_separately_and_comment_filling_can_be_disabled() {
        let source = "// one two three\n// four five six\n";
        let (ctx, env) = editor_with(source, 0, 12);
        as_code(&env, &ctx);
        run(
            "(setq fill-separate-comments t) (fill-paragraph)",
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "// one two\n// three\n// four five six\n");
        set_text(&ctx, source, 0);
        run(
            "(setq fill-comments nil) (fill-region (point-min) (point-max))",
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), source);
        set_text(&ctx, "// one two three ", 17);
        assert!(run("(do-auto-fill)", &env, &ctx).is_nil());
        assert_eq!(text(&ctx), "// one two three ");
    }

    #[test]
    fn the_two_separation_options_are_independent() {
        let (ctx, env) = editor_with("// - one\n// - two\n", 0, 70);
        as_code(&env, &ctx);
        run(
            "(setq fill-separate-list-items nil) (setq fill-separate-comments t) (fill-paragraph)",
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "// - one\n// - two\n");
        run(
            "(setq fill-separate-comments nil) (fill-paragraph)",
            &env,
            &ctx,
        );
        assert_eq!(text(&ctx), "// - one - two\n");
    }

    #[test]
    fn return_fills_all_of_the_previous_line_and_one_undo_restores_it() {
        let source = "aa bb cc dd ee ff";
        let (ctx, env) = editor_with(source, source.len(), 5);
        with_fill_module(&env, &ctx);
        run(
            "(define-key nil \"<ret>\" 'newline) (auto-fill-mode 1)",
            &env,
            &ctx,
        );
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
        assert_eq!(text(&ctx), "aa bb\ncc dd\nee ff\n");
        assert_eq!(point(&ctx), text(&ctx).chars().count());
        run("(undo-boundary) (undo)", &env, &ctx);
        assert_eq!(text(&ctx), source);
        assert_eq!(point(&ctx), source.len());
    }

    #[test]
    fn return_in_the_middle_preserves_the_following_text_and_cursor() {
        let source = "aa bb cc tail";
        let (ctx, env) = editor_with(source, 8, 5);
        with_fill_module(&env, &ctx);
        run(
            "(define-key nil \"<ret>\" 'newline) (auto-fill-mode 1)",
            &env,
            &ctx,
        );
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
        assert_eq!(text(&ctx), "aa bb\ncc\n tail");
        assert_eq!(point(&ctx), 9);
    }

    #[test]
    fn newline_hook_fills_comments_and_leaves_code_and_disabled_modes_alone() {
        let (ctx, env) = editor_with("// aa bb cc dd", 14, 8);
        as_code(&env, &ctx);
        with_fill_module(&env, &ctx);
        run(
            "(define-key nil \"<ret>\" 'newline) (auto-fill-mode 1)",
            &env,
            &ctx,
        );
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
        assert_eq!(text(&ctx), "// aa bb\n// cc dd\n");
        set_text(&ctx, "let aa = bb + cc;", 17);
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
        assert_eq!(text(&ctx), "let aa = bb + cc;\n");
        run("(auto-fill-mode 0)", &env, &ctx);
        set_text(&ctx, "// aa bb cc dd", 14);
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
        assert_eq!(text(&ctx), "// aa bb cc dd\n");
    }

    #[test]
    fn auto_fill_keeps_markers_once_and_measures_tabs_and_unicode_cells() {
        let (ctx, env) = editor_with("\t• 漢字 aa bb cc ", 15, 12);
        run("(setq tab-width 4)", &env, &ctx);
        assert!(run("(do-auto-fill)", &env, &ctx).is_truthy());
        assert_eq!(text(&ctx), "\t• 漢字\n\t  aa bb\n\t  cc ");
        assert_eq!(point(&ctx), text(&ctx).chars().count());
    }

    #[test]
    fn return_that_fits_or_a_literal_insert_newline_does_not_refill() {
        let (ctx, env) = editor_with("aa bb", 5, 5);
        with_fill_module(&env, &ctx);
        run(
            "(define-key nil \"<ret>\" 'newline) (auto-fill-mode 1)",
            &env,
            &ctx,
        );
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
        assert_eq!(text(&ctx), "aa bb\n");
        run("(define-key nil \"<ret>\" 'insert-newline)", &env, &ctx);
        set_text(&ctx, "aa bb cc", 8);
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
        assert_eq!(text(&ctx), "aa bb cc\n");
    }

    #[test]
    fn auto_fill_column_fills_on_its_own_with_or_without_the_guide() {
        for show in ["t", "nil"] {
            let source = "aa bb cc dd";
            let (ctx, env) = editor_with(source, source.len(), 5);
            with_fill_module(&env, &ctx);
            run(
                &format!(
                    "(define-key nil \"<ret>\" 'newline) (auto-fill-column 1) (setq auto-fill-column-show-guide {show})"
                ),
                &env,
                &ctx,
            );
            ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
            assert_eq!(text(&ctx), "aa bb\ncc dd\n");
            run("(auto-fill-mode 1)", &env, &ctx);
            set_text(&ctx, source, source.len());
            ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
            assert_eq!(text(&ctx), "aa bb\ncc dd\n");
            run("(auto-fill-mode 0) (auto-fill-column 0)", &env, &ctx);
            set_text(&ctx, source, source.len());
            ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
            assert_eq!(text(&ctx), "aa bb cc dd\n");
        }
    }

    #[test]
    fn newline_filling_cooperates_with_electric_indent_in_either_hook_order() {
        for order in [
            "(electric-indent-mode 1) (auto-fill-mode 1)",
            "(auto-fill-mode 1) (electric-indent-mode 1)",
        ] {
            let source = "  aa bb cc dd";
            let (ctx, env) = editor_with(source, source.len(), 7);
            with_fill_module(&env, &ctx);
            eval_str(include_str!("../../lisp/base/indent.lisp"), &env, &ctx).unwrap();
            run(
                "(define-key nil \"<ret>\" 'newline) (electric-indent-mode 0)",
                &env,
                &ctx,
            );
            run(order, &env, &ctx);
            ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
            assert_eq!(text(&ctx), "  aa bb\n  cc dd\n  ", "{order}");
            assert_eq!(point(&ctx), text(&ctx).chars().count());
        }
    }

    #[test]
    fn read_only_buffers_refuse_filling_and_newline() {
        let source = "- one two three";
        let (ctx, env) = editor_with(source, source.len(), 8);
        with_fill_module(&env, &ctx);
        run(
            "(define-key nil \"<ret>\" 'newline) (auto-fill-mode 1)",
            &env,
            &ctx,
        );
        ctx.with_current_buffer_mut(|buf| buf.set_read_only(true));
        run("(fill-paragraph) (do-auto-fill)", &env, &ctx);
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
        assert_eq!(text(&ctx), source);
    }

    #[test]
    fn a_block_only_mode_never_fills_code_as_plain_text() {
        let source = "let a = 1; let b = 2;";
        let (ctx, env) = editor_with(source, source.len(), 8);
        run(
            "(make-mode 'block-mode) (set-comment-syntax 'block-mode '((\"/*\" \"*/\"))) (set-major-mode 'block-mode)",
            &env,
            &ctx,
        );
        run("(fill-paragraph) (do-auto-fill)", &env, &ctx);
        assert_eq!(text(&ctx), source);
    }

    #[test]
    fn guide_is_optional_per_buffer_and_does_not_edit_text() {
        let (ctx, env) = editor_with("abc", 0, 5);
        with_fill_module(&env, &ctx);
        assert_eq!(ctx.snapshot(&env, 20, 8).views[0].lines[0], "abc");
        run("(auto-fill-column 1)", &env, &ctx);
        let frame = ctx.snapshot(&env, 20, 8);
        let view = &frame.views[0];
        assert_eq!(view.lines[0], "abc  │");
        assert_eq!(view.lines.last().unwrap(), "     │");
        assert_eq!(view.lines.len(), view.rect.height);
        assert!(
            view.highlights
                .iter()
                .any(|h| h.face == Face::FILL_COLUMN_INDICATOR && h.start_col == 5)
        );
        assert_eq!(text(&ctx), "abc");
        assert!(!ctx.with_current_buffer(|b| b.is_modified));
        assert!(run("(minor-mode-p 'auto-fill-mode)", &env, &ctx).is_nil());
        run("(switch-to-buffer (buffer-create \"other\"))", &env, &ctx);
        assert!(
            !ctx.snapshot(&env, 20, 8).views[0]
                .lines
                .iter()
                .any(|s| s.contains('│'))
        );
        run(
            "(switch-to-buffer \"*scratch*\") (setq auto-fill-column-show-guide nil)",
            &env,
            &ctx,
        );
        assert_eq!(ctx.snapshot(&env, 20, 8).views[0].lines[0], "abc");
    }

    #[test]
    fn guide_and_fill_share_global_mode_and_buffer_columns() {
        let (ctx, env) = editor_with("a", 0, 5);
        with_fill_module(&env, &ctx);
        run(
            "(auto-fill-mode 1) (setq auto-fill-column-character \"|\")",
            &env,
            &ctx,
        );
        assert_eq!(ctx.snapshot(&env, 30, 8).views[0].lines[0], "a    |");
        run("(put 'fundamental-mode 'fill-column 8)", &env, &ctx);
        assert_eq!(ctx.snapshot(&env, 30, 8).views[0].lines[0], "a       |");
        run("(set-fill-column 4)", &env, &ctx);
        assert_eq!(ctx.snapshot(&env, 30, 8).views[0].lines[0], "a   |");
        for value in [
            "\"\"",
            "\"ab\"",
            "\"漢\"",
            "\"\u{0301}\"",
            "\"\\t\"",
            "\" \"",
            "nil",
            "42",
        ] {
            run(
                &format!("(setq auto-fill-column-character {value})"),
                &env,
                &ctx,
            );
            assert_eq!(
                ctx.snapshot(&env, 30, 8).views[0].lines[0],
                "a   │",
                "{value}"
            );
        }
    }

    #[test]
    fn guide_uses_display_cells_and_keeps_characters_at_the_column() {
        let (ctx, env) = editor_with("漢字 x\n123456789\n漢漢漢", 0, 5);
        with_fill_module(&env, &ctx);
        run("(auto-fill-column 1)", &env, &ctx);
        let view = &ctx.snapshot(&env, 20, 8).views[0];
        assert_eq!(view.lines[0], "漢字 x");
        assert_eq!(view.lines[1], "123456789");
        assert_eq!(view.lines[2], "漢漢漢");
        set_text(&ctx, "漢字  x", 0);
        let frame = ctx.snapshot(&env, 20, 8);
        assert_eq!(frame.views[0].lines[0], "漢字 │x");
        assert_eq!(cells(&frame.views[0].lines[0]), 7);
    }

    #[test]
    fn guide_respects_gutter_horizontal_scroll_and_window_clipping() {
        let source = "01234567890123456789";
        let (ctx, env) = editor_with(source, 18, 5);
        with_fill_module(&env, &ctx);
        run(
            "(define-minor-mode display-line-numbers-mode \"Num\" \"Numbers.\") (display-line-numbers-mode 1) (auto-fill-column 1)",
            &env,
            &ctx,
        );
        let frame = ctx.snapshot(&env, 15, 8);
        assert!(frame.views[0].gutter_width > 0);
        assert!(
            !frame.views[0].lines.iter().any(|s| s.contains('│')),
            "column scrolled out"
        );
        set_text(&ctx, "abc", 0);
        run("(set-fill-column 5)", &env, &ctx);
        let frame = ctx.snapshot(&env, 20, 8);
        assert_eq!(frame.views[0].lines[0], "abc  │");
        assert!(frame.views[0].gutter.iter().all(|c| !c.text.contains('│')));
        run("(set-fill-column 1000000)", &env, &ctx);
        assert_eq!(ctx.snapshot(&env, 20, 8).views[0].lines[0], "abc");
    }
}
