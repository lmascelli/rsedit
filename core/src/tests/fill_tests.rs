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
    use crate::text::fill::{Measure, break_at, comment_prefix, lay_out};
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
    fn auto_fill_enable_turns_it_on_in_new_buffers_of_the_mode() {
        let (ctx, env) = editor_with("", 0, 10);
        with_fill_module(&env, &ctx);
        run("(auto-fill-enable 'fundamental-mode)", &env, &ctx);
        run(r#"(switch-to-buffer (buffer-create "notes"))"#, &env, &ctx);
        assert!(run("(auto-fill-mode-p)", &env, &ctx).is_truthy());
    }
}
