//! The sexp commands as a user reaches them, `recenter`, and what Tab does.
//!
//! The scanner itself is tested in `sexp_tests`, against tables built in Rust.
//! What is left once that passes is everything between the scanner and a
//! keystroke: that a command reaches it, that a kill goes through the two
//! doors, that the same text moves differently in two modes, and that the
//! indentation dispatch picks the right one of its three cases.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
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

    /// The editor with the layers `indent.lisp` is written against.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/indent.lisp"),
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
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| b.current_mode = mode.into());
    }

    fn typing(ctx: &Ctx, text: &str, at: usize) {
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(at);
            b.text.cursor_move(line, col);
            b.is_modified = false;
        });
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .expect("read lock")
            .text
            .to_string()
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .expect("read lock")
            .text
            .cursor_pos_1d()
    }

    fn number(exp: &LispExp<Ctx>) -> f64 {
        match exp {
            LispExp::Number(n) => *n,
            other => panic!("expected a number, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // A kill is an edit like any other
    // -----------------------------------------------------------------------

    #[test]
    fn killing_an_expression_can_be_undone_in_one_step() {
        // It goes through `cut_out`, so undo groups it with the command that
        // made it rather than leaving half of a replacement behind.
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a (b c)) rest", 0);

        run("(kill-sexp)", &env, &ctx);
        assert_eq!(contents(&ctx), " rest");
        run("(undo)", &env, &ctx);
        assert_eq!(contents(&ctx), "(a (b c)) rest");
    }

    #[test]
    fn a_killed_expression_goes_to_the_kill_ring() {
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a b) rest", 0);

        run("(kill-sexp)", &env, &ctx);
        run("(yank)", &env, &ctx);
        assert_eq!(contents(&ctx), "(a b) rest");
    }

    #[test]
    fn a_read_only_buffer_refuses_the_kill() {
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a b) rest", 0);
        run("(set-buffer-read-only t)", &env, &ctx);

        run("(kill-sexp)", &env, &ctx);
        assert_eq!(contents(&ctx), "(a b) rest");
    }

    #[test]
    fn kill_sexp_removes_exactly_what_forward_sexp_traverses() {
        // The property the file claims: the kill asks the same function where
        // the motion ends, so the two cannot drift apart.
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");

        for text in ["(a (b)) rest", "(message \"a )\") rest", "'foo rest"] {
            typing(&ctx, text, 0);
            run("(forward-sexp)", &env, &ctx);
            let traversed = point(&ctx);

            typing(&ctx, text, 0);
            run("(kill-sexp)", &env, &ctx);
            let mut expected = String::new();
            expected.push_str(&text[traversed..]);
            assert_eq!(contents(&ctx), expected, "killing {text:?}");
        }
    }

    // -----------------------------------------------------------------------
    // Reachable from the keyboard
    // -----------------------------------------------------------------------

    #[test]
    fn the_sexp_keys_reach_the_commands() {
        let (ctx, env) = editor();
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("loading common-keymaps.lisp");
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a b) (c)", 0);

        let ctrl_alt = || KeyModifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: ctrl_alt(),
            },
            &env,
        );
        assert_eq!(point(&ctx), 5, "C-M-f steps over the first list");

        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: ctrl_alt(),
            },
            &env,
        );
        assert_eq!(point(&ctx), 0, "C-M-b comes back");

        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: ctrl_alt(),
            },
            &env,
        );
        assert_eq!(contents(&ctx), " (c)", "C-M-k kills it");
    }

    #[test]
    fn the_sexp_commands_are_available_from_m_x() {
        let (ctx, _env) = editor();
        let names = ctx.command_names();
        for name in [
            "forward-sexp",
            "backward-sexp",
            "kill-sexp",
            "backward-kill-sexp",
            "up-list",
            "backward-up-list",
            "down-list",
            "recenter",
            "indent-for-tab-command",
        ] {
            assert!(names.iter().any(|known| known == name), "{name} is missing");
        }
    }

    // -----------------------------------------------------------------------
    // The same text, two modes
    // -----------------------------------------------------------------------

    #[test]
    fn the_same_text_moves_differently_in_two_modes() {
        // The point of the whole design. In a Lisp `;` opens a comment, so the
        // `)` inside it is text and the list runs on; in a mode that never said
        // so it is punctuation, and the list ends at the first `)` it meets.
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        run(
            "(make-mode 'toy-plain) (set-syntax-pairs 'toy-plain \"()\")",
            &env,
            &ctx,
        );

        let text = "(a ; ) not the end\n b) after";

        in_mode(&ctx, "toy-lisp");
        typing(&ctx, text, 0);
        run("(forward-sexp)", &env, &ctx);
        assert_eq!(point(&ctx), 22, "the `)` in the comment is text");

        in_mode(&ctx, "toy-plain");
        typing(&ctx, text, 0);
        run("(forward-sexp)", &env, &ctx);
        assert_eq!(
            point(&ctx),
            6,
            "with no comment syntax it ends at the first `)`"
        );
    }

    // -----------------------------------------------------------------------
    // syntax-ppss
    // -----------------------------------------------------------------------

    #[test]
    fn syntax_ppss_in_a_string_reports_the_string_and_not_a_comment() {
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a \"bc\" d)", 5);

        let state = run("(syntax-ppss)", &env, &ctx);
        let parts: Vec<LispExp<Ctx>> = state.iter().collect();
        assert_eq!(number(&parts[0]), 1.0, "a string does not change the depth");
        assert_eq!(
            number(&parts[2]),
            3.0,
            "element 2 is where the string began"
        );
        assert_eq!(
            parts[3],
            LispExp::nil(),
            "element 3 says it is not a comment"
        );
    }

    #[test]
    fn syntax_ppss_reports_the_delimiter_as_well_as_the_expression() {
        // Element 4. They differ by the prefix, and indentation lines up
        // against the delimiter while motion wants the expression.
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "'(a b)", 3);

        let parts: Vec<LispExp<Ctx>> = run("(syntax-ppss)", &env, &ctx).iter().collect();
        assert_eq!(number(&parts[1]), 0.0, "the expression starts at the quote");
        assert_eq!(
            number(&parts[4]),
            1.0,
            "the delimiter is the paren after it"
        );
    }

    #[test]
    fn bounds_of_enclosing_list_is_the_whole_list() {
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a (b c) d)", 5);

        let parts: Vec<LispExp<Ctx>> = run("(bounds-of-enclosing-list)", &env, &ctx)
            .iter()
            .collect();
        assert_eq!((number(&parts[0]), number(&parts[1])), (3.0, 8.0));

        typing(&ctx, "(a b)", 0);
        assert_eq!(
            run("(bounds-of-enclosing-list)", &env, &ctx),
            LispExp::nil(),
            "nothing encloses the top level"
        );
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

    #[test]
    fn tab_indents_the_region_and_leaves_it_in_place() {
        let (ctx, env) = editor();
        typing(&ctx, "        head\na\nb\nc", 13);
        run("(set-mark) (goto-char 17)", &env, &ctx);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "        head\n        a\n        b\n        c"
        );
        assert_eq!(
            run("(use-region-p)", &env, &ctx),
            LispExp::t(),
            "the region survives, so a second Tab works on the same block"
        );
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
        // The reason `syntax-ppss` grew a fifth element.
        let (ctx, env) = editor();
        lisp_indenting(&ctx, &env);
        typing(&ctx, "'(a\nb)", 4);
        run("(indent-for-tab-command)", &env, &ctx);
        assert_eq!(contents(&ctx), "'(a\n  b)");
    }

    // -----------------------------------------------------------------------
    // recenter
    // -----------------------------------------------------------------------

    /// The lines the focused window is actually showing.
    fn visible(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<String> {
        ctx.snapshot(env, 80, 24)
            .views
            .into_iter()
            .find(|view| view.is_focused)
            .expect("a focused window")
            .lines
    }

    fn hundred_lines(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        let text: String = (0..100).map(|n| format!("line {n}\n")).collect();
        typing(ctx, &text, 0);
        run("(goto-line 50)", env, ctx);
        // Lay the window out once, so it has a height to centre within.
        let _ = ctx.snapshot(env, 80, 24);
    }

    #[test]
    fn recentring_moves_the_text_and_not_the_cursor() {
        let (ctx, env) = editor();
        hundred_lines(&ctx, &env);
        let before = point(&ctx);

        run("(recenter)", &env, &ctx);
        assert_eq!(point(&ctx), before, "the text slides under the cursor");

        let lines = visible(&ctx, &env);
        let middle = lines.len() / 2;
        assert!(
            lines[middle].contains("line 49"),
            "the line point is on should be in the middle, got {:?}",
            lines[middle]
        );
    }

    #[test]
    fn recentring_accepts_where_to_put_the_line() {
        let (ctx, env) = editor();
        hundred_lines(&ctx, &env);

        run("(recenter 0)", &env, &ctx);
        let lines = visible(&ctx, &env);
        assert!(
            lines[0].contains("line 49"),
            "0 puts it on the top row, got {:?}",
            lines[0]
        );

        run("(recenter 1)", &env, &ctx);
        let lines = visible(&ctx, &env);
        let last = lines.len() - 1;
        assert!(
            lines[last].contains("line 49"),
            "1 puts it on the bottom row, got {:?}",
            lines[last]
        );
    }
}
