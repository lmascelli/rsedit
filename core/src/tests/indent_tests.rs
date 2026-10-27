//! Indentation: the primitives that read and rewrite it, and what Tab and
//! Enter do with them, on a line and on a region, in a mode with a rule and
//! in one without.
//!
//! Design: `doc/architettura-editor.typ, <espressioni-indentazione>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
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

    /// The editor with `indent.lisp` loaded, and the files it is written
    /// against.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/base/indent.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// A mode whose syntax table is a Lisp's.
    fn with_lisp_mode(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        run(
            "(make-mode 'toy-lisp)
             (set-syntax-pairs 'toy-lisp \"()[]\")
             (set-syntax-entry 'toy-lisp \"'`,\" 'prefix)
             (set-syntax-entry 'toy-lisp \"-*+<>=?!\" 'symbol)
             (set-comment-syntax 'toy-lisp '((\";\")))",
            env,
            ctx,
        );
    }

    fn in_mode(ctx: &Ctx, mode: &str) {
        ctx.with_buffer_mut("*scratch*", |b| b.current_mode = mode.into());
    }

    fn typing(ctx: &Ctx, text: &str, at: usize) {
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(at);
            b.text.cursor_move(line, col);
            b.is_modified = false;
        });
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.with_buffer("*scratch*", |b| b.text.to_string())
            .expect("*scratch*")
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos_1d())
            .expect("*scratch*")
    }

    fn number(exp: &LispExp<Ctx>) -> f64 {
        match exp {
            LispExp::Number(n) => *n,
            other => panic!("expected a number, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Indentation, the pieces
    // -----------------------------------------------------------------------

    #[test]
    fn indentation_is_read_in_characters_and_columns() {
        let (ctx, env) = editor();
        typing(&ctx, "    foo\nbar", 9);
        assert_eq!(number(&run("(current-indentation)", &env, &ctx)), 0.0);
        assert_eq!(number(&run("(current-indentation 1)", &env, &ctx)), 4.0);
        assert_eq!(number(&run("(current-column)", &env, &ctx)), 1.0);
    }

    #[test]
    fn a_blank_line_does_not_reset_the_indentation_above_it() {
        // A blank line between two indented ones is a paragraph break, not a
        // return to the left margin. Reporting 0 would walk a block back to
        // column 0 one blank line at a time.
        let (ctx, env) = editor();
        typing(&ctx, "    foo\n\n\nbar", 12);
        assert_eq!(number(&run("(previous-indentation)", &env, &ctx)), 4.0);
    }

    #[test]
    fn a_tab_counts_as_indentation_even_though_it_is_not_a_space() {
        // `indent-line-to` only ever writes spaces, but a file opened from
        // disk may already be indented with tabs. Reading one as "no
        // indentation" would make Tab put spaces in *front* of it rather than
        // replace it, and the line would drift right every time it was pressed.
        let (ctx, env) = editor();
        typing(&ctx, "\tfoo\nbar", 5);
        assert_eq!(number(&run("(current-indentation 1)", &env, &ctx)), 1.0);
        assert_eq!(number(&run("(previous-indentation)", &env, &ctx)), 1.0);
    }

    #[test]
    fn the_first_line_has_nothing_above_it() {
        let (ctx, env) = editor();
        typing(&ctx, "foo", 1);
        assert_eq!(number(&run("(previous-indentation)", &env, &ctx)), 0.0);
    }

    #[test]
    fn indenting_a_line_moves_its_text_and_its_point() {
        let (ctx, env) = editor();
        typing(&ctx, "  foo", 4);
        assert_eq!(run("(indent-line-to 6)", &env, &ctx), LispExp::t());
        assert_eq!(contents(&ctx), "      foo");
        assert_eq!(point(&ctx), 8, "point keeps the character it was on");
    }

    #[test]
    fn point_inside_the_indentation_lands_at_the_end_of_it() {
        let (ctx, env) = editor();
        typing(&ctx, "      foo", 2);
        run("(indent-line-to 2)", &env, &ctx);
        assert_eq!(contents(&ctx), "  foo");
        assert_eq!(point(&ctx), 2);
    }

    #[test]
    fn indenting_to_where_it_already_is_reports_nothing_done() {
        // What `indent-for-tab-command` reads to choose its third case.
        let (ctx, env) = editor();
        typing(&ctx, "    foo", 6);
        assert_eq!(run("(indent-line-to 4)", &env, &ctx), LispExp::nil());
        assert_eq!(contents(&ctx), "    foo");
    }

    #[test]
    fn indenting_is_one_undo_step_and_a_read_only_buffer_refuses_it() {
        let (ctx, env) = editor();
        typing(&ctx, "foo", 0);
        run("(indent-line-to 4)", &env, &ctx);
        assert_eq!(contents(&ctx), "    foo");
        run("(undo)", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "foo",
            "the delete and the insert undo together"
        );

        run("(set-buffer-read-only t)", &env, &ctx);
        run("(indent-line-to 4)", &env, &ctx);
        assert_eq!(contents(&ctx), "foo");
    }

    // -----------------------------------------------------------------------
    // Tab's three cases
    // -----------------------------------------------------------------------

    #[test]
    fn tab_indents_a_line_to_match_the_one_above() {
        let (ctx, env) = editor();
        typing(&ctx, "    foo\nbar", 8);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "    foo\n    bar");
    }

    #[test]
    fn tab_on_an_already_indented_line_inserts_spaces() {
        let (ctx, env) = editor();
        typing(&ctx, "    foo\n    bar", 15);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "    foo\n    bar    ");
    }

    #[test]
    fn tab_always_indent_decides_the_third_case() {
        let (ctx, env) = editor();
        typing(&ctx, "    foo\n    bar", 15);
        run("(setq tab-always-indent t)", &env, &ctx);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "    foo\n    bar",
            "t means Tab only ever indents"
        );
    }

    #[test]
    fn the_tab_width_can_be_set_per_mode() {
        let (ctx, env) = editor();
        run("(make-mode 'wide) (put 'wide 'tab-width 8)", &env, &ctx);
        in_mode(&ctx, "wide");
        typing(&ctx, "    foo\n    bar", 15);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "    foo\n    bar        ");
    }

    // -----------------------------------------------------------------------
    // Tab on a region
    //
    // Two meanings, chosen by the mode. With an `indent-function' every line
    // goes where the rule says. Without one the only rule is "match the line
    // above", which applied line by line flattens a block, each line copying
    // the one before; so without a rule the block moves right as a whole.
    // -----------------------------------------------------------------------

    fn select(ctx: &Ctx, env: &Arc<Env<Ctx>>, from: usize, to: usize) {
        run(
            &format!("(goto-char {from}) (set-mark) (goto-char {to})"),
            env,
            ctx,
        );
        assert_eq!(run("(use-region-p)", env, ctx), LispExp::t());
    }

    fn key(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode, shift: bool) {
        ctx.handle_key_event(
            KeyEvent {
                code,
                modifiers: KeyModifiers {
                    shift,
                    ..Default::default()
                },
            },
            env,
        );
    }

    #[test]
    fn tab_on_a_region_with_no_rule_shifts_it_and_keeps_its_shape() {
        let (ctx, env) = editor();
        let text = "        head\na\n    b\nc";
        typing(&ctx, text, 0);
        // From the start of `a' to the middle of `b': two lines.
        select(&ctx, &env, 13, 18);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "        head\n    a\n        b\nc",
            "each line moved by one indent-width; nothing copied the line above"
        );
    }

    #[test]
    fn tab_leaves_the_region_in_place_so_a_second_tab_shifts_again() {
        let (ctx, env) = editor();
        typing(&ctx, "a\nb\nc", 0);
        select(&ctx, &env, 0, 3);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(
            run("(use-region-p)", &env, &ctx),
            LispExp::t(),
            "the region survives"
        );
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "        a\n        b\nc");
    }

    #[test]
    fn a_region_ending_at_the_start_of_a_line_does_not_take_that_line() {
        // Selecting whole lines from the margin ends the region at column 0 of
        // the line after; that line was not meant.
        let (ctx, env) = editor();
        typing(&ctx, "a\nb\nc", 0);
        select(&ctx, &env, 0, 4);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "    a\n    b\nc");
    }

    #[test]
    fn blank_lines_in_a_shifted_region_stay_blank() {
        let (ctx, env) = editor();
        typing(&ctx, "a\n\nb", 0);
        select(&ctx, &env, 0, 4);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "    a\n\n    b", "no trailing whitespace");
    }

    #[test]
    fn a_mode_with_a_rule_reindents_the_region_instead() {
        let (ctx, env) = editor();
        run(
            "(make-mode 'always-three)
             (defun three () 3)
             (put 'always-three 'indent-function 'three)",
            &env,
            &ctx,
        );
        in_mode(&ctx, "always-three");
        typing(&ctx, "a\n        b\nc", 0);
        select(&ctx, &env, 0, 13);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "   a\n   b\n   c");
    }

    #[test]
    fn shift_tab_moves_the_region_left_and_stops_at_the_margin() {
        let (ctx, env) = editor();
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("loading common-keymaps.lisp");
        typing(&ctx, "        a\n  b\n      c", 0);
        select(&ctx, &env, 0, 22);
        key(&ctx, &env, KeyCode::Tab, true);
        assert_eq!(
            contents(&ctx),
            "    a\nb\n  c",
            "the short line stopped at 0 without holding the others back"
        );
        assert_eq!(run("(use-region-p)", &env, &ctx), LispExp::t());
    }

    #[test]
    fn shifting_without_a_region_moves_the_line_and_keeps_point_on_its_text() {
        let (ctx, env) = editor();
        typing(&ctx, "    foo\nbar", 6);
        run("(indent-rigidly-left)", &env, &ctx);
        assert_eq!(contents(&ctx), "foo\nbar");
        assert_eq!(point(&ctx), 2, "still before the second `o'");
        run("(indent-rigidly-right) (indent-rigidly-right)", &env, &ctx);
        assert_eq!(contents(&ctx), "        foo\nbar");
        assert_eq!(point(&ctx), 10);
    }

    #[test]
    fn shifting_a_region_by_key_is_one_undo_step() {
        let (ctx, env) = editor();
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("loading common-keymaps.lisp");
        typing(&ctx, "a\nb\nc", 0);
        select(&ctx, &env, 0, 5);
        key(&ctx, &env, KeyCode::Tab, false);
        assert_eq!(contents(&ctx), "    a\n    b\n    c");
        run("(undo)", &env, &ctx);
        assert_eq!(contents(&ctx), "a\nb\nc", "three lines, one step");
    }

    #[test]
    fn shift_tab_has_a_name_that_survives_the_round_trip() {
        // `S-<tab>' is what a binding writes and what `describe-key' should
        // say back; `<backtab>' is the terminal's name, accepted as the same.
        let shift_tab = KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers {
                shift: true,
                ..Default::default()
            },
        };
        let parse = |spelling| crate::input::parse_key_sequence(spelling);
        assert_eq!(parse("S-<tab>"), Some(vec![shift_tab.clone()]));
        assert_eq!(parse("<backtab>"), Some(vec![shift_tab.clone()]));
        assert_ne!(
            parse("<tab>"),
            Some(vec![shift_tab.clone()]),
            "Tab is not Shift-Tab"
        );
        assert_eq!(crate::input::describe_keys(&[shift_tab]), "S-<tab>");
    }

    // -----------------------------------------------------------------------
    // Enter
    // -----------------------------------------------------------------------

    #[test]
    fn newline_and_indent_matches_the_line_above() {
        let (ctx, env) = editor();
        typing(&ctx, "    foo", 7);
        run("(newline-and-indent)", &env, &ctx);
        assert_eq!(contents(&ctx), "    foo\n    ");
        assert_eq!(point(&ctx), 12, "at the end of the new indentation");
    }

    #[test]
    fn newline_and_indent_asks_the_mode_where_the_line_goes() {
        let (ctx, env) = editor();
        run(
            "(make-mode 'always-three)
             (defun three () 3)
             (put 'always-three 'indent-function 'three)",
            &env,
            &ctx,
        );
        in_mode(&ctx, "always-three");
        typing(&ctx, "foo", 3);
        run("(newline-and-indent)", &env, &ctx);
        assert_eq!(contents(&ctx), "foo\n   ");
    }

    #[test]
    fn newline_and_indent_in_a_lisp_indents_the_body() {
        let (ctx, env) = editor();
        lisp_indenting(&ctx, &env);
        typing(&ctx, "(defun f ()", 11);
        run("(newline-and-indent)", &env, &ctx);
        assert_eq!(contents(&ctx), "(defun f ()\n  ");
    }

    #[test]
    fn newline_and_indent_splits_a_line_without_leaving_whitespace_behind() {
        let (ctx, env) = editor();
        // Point between the spaces and `bar'.
        typing(&ctx, "    foo   bar", 10);
        run("(newline-and-indent)", &env, &ctx);
        assert_eq!(contents(&ctx), "    foo\n    bar");
        assert_eq!(point(&ctx), 12, "before `bar', which came along");
    }

    #[test]
    fn newline_and_indent_on_an_indented_blank_line_leaves_it_empty() {
        let (ctx, env) = editor();
        typing(&ctx, "    foo\n    ", 12);
        run("(newline-and-indent)", &env, &ctx);
        assert_eq!(contents(&ctx), "    foo\n\n    ");
    }

    #[test]
    fn enter_indents_until_electric_indent_mode_is_turned_off() {
        let (ctx, env) = editor();
        eval_str(
            include_str!("../../lisp/base/common-keymaps.lisp"),
            &env,
            &ctx,
        )
        .expect("loading common-keymaps.lisp");
        typing(&ctx, "    foo", 7);
        key(&ctx, &env, KeyCode::Enter, false);
        assert_eq!(
            contents(&ctx),
            "    foo\n    ",
            "on by default, as in Emacs"
        );

        run("(setq electric-indent-mode nil)", &env, &ctx);
        key(&ctx, &env, KeyCode::Enter, false);
        assert_eq!(
            contents(&ctx),
            "    foo\n    \n",
            "off: a plain newline, with no rebinding needed"
        );
    }

    // -----------------------------------------------------------------------
    // indent-region-style
    // -----------------------------------------------------------------------

    fn a_mode_with_a_rule(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        run(
            "(make-mode 'always-three)
             (defun three () 3)
             (put 'always-three 'indent-function 'three)",
            env,
            ctx,
        );
        in_mode(ctx, "always-three");
    }

    #[test]
    fn shift_style_shifts_even_where_the_mode_has_a_rule() {
        let (ctx, env) = editor();
        a_mode_with_a_rule(&ctx, &env);
        run("(setq indent-region-style 'shift)", &env, &ctx);
        typing(&ctx, "a\n  b", 0);
        select(&ctx, &env, 0, 6);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "    a\n      b");
    }

    #[test]
    fn reindent_style_reindents_even_where_the_mode_has_no_rule() {
        // Which, with no rule, means matching the line above -- the flattening
        // `auto' exists to avoid. Asked for, so given.
        let (ctx, env) = editor();
        run("(setq indent-region-style 'reindent)", &env, &ctx);
        typing(&ctx, "    head\na\n        b", 9);
        select(&ctx, &env, 9, 20);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "    head\n    a\n    b");
    }

    #[test]
    fn a_mode_can_choose_its_own_style() {
        let (ctx, env) = editor();
        a_mode_with_a_rule(&ctx, &env);
        run(
            "(put 'always-three 'indent-region-style 'shift)",
            &env,
            &ctx,
        );
        typing(&ctx, "a\n  b", 0);
        select(&ctx, &env, 0, 6);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "    a\n      b",
            "the mode's own choice wins over `auto'"
        );
    }

    #[test]
    fn a_style_nobody_understands_behaves_as_auto() {
        let (ctx, env) = editor();
        run("(setq indent-region-style 'shfit)", &env, &ctx);
        typing(&ctx, "a\n  b", 0);
        select(&ctx, &env, 0, 6);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "    a\n      b", "no rule here, so shifted");
    }

    #[test]
    fn a_mode_can_replace_the_rule_entirely() {
        let (ctx, env) = editor();
        run(
            "(make-mode 'always-three)
             (defun three () 3)
             (put 'always-three 'indent-function 'three)",
            &env,
            &ctx,
        );
        in_mode(&ctx, "always-three");
        typing(&ctx, "foo", 0);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "   foo");
    }

    // -----------------------------------------------------------------------
    // Indenting a Lisp
    // -----------------------------------------------------------------------

    fn lisp_indenting(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        with_lisp_mode(ctx, env);
        run(
            "(put 'toy-lisp 'indent-function 'lisp-indent-line)",
            env,
            ctx,
        );
        in_mode(ctx, "toy-lisp");
    }

    #[test]
    fn a_top_level_form_starts_at_column_zero() {
        let (ctx, env) = editor();
        lisp_indenting(&ctx, &env);
        typing(&ctx, "(a b)\n   (c d)", 9);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "(a b)\n(c d)");
    }

    #[test]
    fn a_body_form_is_indented_two_from_its_delimiter() {
        let (ctx, env) = editor();
        lisp_indenting(&ctx, &env);
        typing(&ctx, "(defun f ()\n(g))", 12);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "(defun f ()\n  (g))");
    }

    #[test]
    fn an_argument_is_lined_up_under_the_first_one() {
        let (ctx, env) = editor();
        lisp_indenting(&ctx, &env);
        typing(&ctx, "(foo bar\nbaz)", 9);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "(foo bar\n     baz)");
    }

    #[test]
    fn a_form_whose_head_is_alone_indents_just_inside_the_delimiter() {
        let (ctx, env) = editor();
        lisp_indenting(&ctx, &env);
        typing(&ctx, "(foo\nbar)", 5);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "(foo\n bar)");
    }

    #[test]
    fn a_line_inside_a_string_is_left_alone() {
        // Its leading whitespace is content, not layout.
        let (ctx, env) = editor();
        lisp_indenting(&ctx, &env);
        typing(&ctx, "(foo \"bar\n    baz\")", 14);
        assert_eq!(
            run("(indent-line)", &env, &ctx),
            LispExp::nil(),
            "the rule reports the indentation the line already has, so nothing moves"
        );
        assert_eq!(contents(&ctx), "(foo \"bar\n    baz\")");
    }

    #[test]
    fn a_quoted_list_indents_against_its_delimiter_not_its_quote() {
        // Why `syntax-ppss` has a fifth element.
        let (ctx, env) = editor();
        lisp_indenting(&ctx, &env);
        typing(&ctx, "'(a\nb)", 4);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "'(a\n  b)");
    }
}
