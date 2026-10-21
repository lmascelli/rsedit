//! Exercise Vim through physical input and the shipped Risp, not a Rust Vim engine.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;
    type Ctx = EditorState<GapBuffer>;

    fn run(source: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let form = Parser::new(&format!("(progn {source})"))
            .next()
            .expect("parse");
        eval(&form, env.clone(), ctx).unwrap_or_else(|e| panic!("{source}: {e:?}"))
    }
    fn editor(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("editor");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for (name, source) in [
            ("commands", include_str!("../../lisp/commands.lisp")),
            ("debug", include_str!("../../lisp/debug.lisp")),
            (
                "common-keymaps",
                include_str!("../../lisp/common-keymaps.lisp"),
            ),
            ("vim", include_str!("../../lisp/vim.lisp")),
        ] {
            let form = Parser::new(&format!("(progn {source})"))
                .next()
                .unwrap_or_else(|e| panic!("{name} parse: {e:?}"));
            eval(&form, env.clone(), &ctx).unwrap_or_else(|e| panic!("{name} load: {e:?}"));
        }
        ctx.with_current_buffer_mut(|b| {
            b.text = GapBuffer::from(text);
            b.text.cursor_move(0, 0);
        });
        run("(vim-mode t)", &env, &ctx);
        (ctx, env)
    }
    fn keys(ctx: &Ctx, env: &Arc<Env<Ctx>>, sequence: &str) {
        for c in sequence.chars() {
            ctx.handle_key_event(KeyEvent::new(KeyCode::Char(c)), env);
            assert!(
                !ctx.get_logs().iter().any(|line| line.contains("Vim: ")),
                "Vim error: {:?}",
                ctx.get_logs()
            );
        }
    }
    fn esc(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        ctx.handle_key_event(KeyEvent::new(KeyCode::Esc), env);
    }
    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|b| b.text.to_string())
    }
    fn point(ctx: &Ctx) -> usize {
        ctx.with_current_buffer(|b| b.text.cursor_pos_1d())
    }

    #[test]
    fn vim_module_loads_and_normal_does_not_insert() {
        let (ctx, env) = editor("abc");
        keys(&ctx, &env, "l");
        assert_eq!(point(&ctx), 1);
        keys(&ctx, &env, "Q");
        assert_eq!(text(&ctx), "abc");
    }
    #[test]
    fn delete_word_and_multiply_counts() {
        let (ctx, env) = editor("one two three four five six seven");
        keys(&ctx, &env, "2d3w");
        assert_eq!(text(&ctx), "seven");
        keys(&ctx, &env, "u");
        assert_eq!(text(&ctx), "one two three four five six seven");
    }
    #[test]
    fn insert_session_is_one_undo_group() {
        let (ctx, env) = editor("abc");
        keys(&ctx, &env, "i012345678901234567890123456789");
        esc(&ctx, &env);
        keys(&ctx, &env, "u");
        assert_eq!(text(&ctx), "abc");
    }
    #[test]
    fn change_word_and_dot() {
        let (ctx, env) = editor("one two three");
        keys(&ctx, &env, "cwX");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "X two three");
        keys(&ctx, &env, "w.");
        assert_eq!(text(&ctx), "X X three");
    }
    #[test]
    fn linewise_register_paste() {
        let (ctx, env) = editor("one\ntwo\nthree");
        keys(&ctx, &env, "yyjp");
        assert_eq!(text(&ctx), "one\ntwo\none\nthree");
    }
    #[test]
    fn visual_selection_is_inclusive() {
        let (ctx, env) = editor("abcde");
        keys(&ctx, &env, "vld");
        assert_eq!(text(&ctx), "cde");
    }
    #[test]
    fn text_object_deletes_inner_pair() {
        let (ctx, env) = editor("(one (two) three)");
        run("(goto-char 7)", &env, &ctx);
        keys(&ctx, &env, "di(");
        assert_eq!(text(&ctx), "(one () three)");
    }
    fn ctrl(ctx: &Ctx, env: &Arc<Env<Ctx>>, c: char) {
        let mut event = KeyEvent::new(KeyCode::Char(c));
        event.modifiers.ctrl = true;
        ctx.handle_key_event(event, env);
    }
    #[test]
    fn repeated_dot_keeps_the_original_change() {
        let (ctx, env) = editor("one two three four");
        keys(&ctx, &env, "cwX");
        esc(&ctx, &env);
        keys(&ctx, &env, "w.w.");
        assert_eq!(text(&ctx), "X X X four");
    }
    #[test]
    fn dot_recomputes_text_objects() {
        let (ctx, env) = editor("(a) (longword)");
        keys(&ctx, &env, "di(");
        keys(&ctx, &env, "w.");
        assert_eq!(text(&ctx), "() ()");
    }
    #[test]
    fn dot_recomputes_end_of_line() {
        let (ctx, env) = editor("abc\nlongword");
        keys(&ctx, &env, "lDj0l.");
        assert_eq!(text(&ctx), "a\nl");
    }
    #[test]
    fn dot_repeats_backward_deletion() {
        let (ctx, env) = editor("abcdef");
        keys(&ctx, &env, "lllX.");
        assert_eq!(text(&ctx), "adef");
    }
    #[test]
    fn counted_insert_and_dot_override() {
        let (ctx, env) = editor("abc");
        keys(&ctx, &env, "3iX");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "XXXabc");
        keys(&ctx, &env, "$2.");
        assert_eq!(text(&ctx), "XXXabXXc");
    }
    #[test]
    fn change_last_character_of_word() {
        let (ctx, env) = editor("a b");
        keys(&ctx, &env, "cwX");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "X b");
    }
    #[test]
    fn cancelling_operator_and_count_preserves_text() {
        let (ctx, env) = editor("abc def");
        keys(&ctx, &env, "12d");
        esc(&ctx, &env);
        keys(&ctx, &env, "l");
        assert_eq!(text(&ctx), "abc def");
        assert_eq!(point(&ctx), 1);
    }
    #[test]
    fn invalid_motion_does_not_leak_pending_operator() {
        let (ctx, env) = editor("abc def");
        keys(&ctx, &env, "dqix");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "xabc def");
    }
    #[test]
    fn vertical_motion_preserves_goal_column() {
        let (ctx, env) = editor("abcdef\nx\nabcdef");
        keys(&ctx, &env, "4ljj");
        assert_eq!(point(&ctx), 13);
    }
    #[test]
    fn word_motions_distinguish_punctuation_and_big_words() {
        let (ctx, env) = editor("foo.bar baz");
        keys(&ctx, &env, "w");
        assert_eq!(point(&ctx), 3);
        keys(&ctx, &env, "w");
        assert_eq!(point(&ctx), 4);
        keys(&ctx, &env, "0W");
        assert_eq!(point(&ctx), 8);
        keys(&ctx, &env, "B");
        assert_eq!(point(&ctx), 0);
    }
    #[test]
    fn goto_last_line_and_counted_goto() {
        let (ctx, env) = editor("one\ntwo\nthree");
        keys(&ctx, &env, "G");
        assert_eq!(point(&ctx), 8);
        keys(&ctx, &env, "gg");
        assert_eq!(point(&ctx), 0);
        keys(&ctx, &env, "2G");
        assert_eq!(point(&ctx), 4);
    }
    #[test]
    fn find_till_and_repeat() {
        let (ctx, env) = editor("a,b,c,d");
        keys(&ctx, &env, "t,;");
        assert_eq!(point(&ctx), 2);
        keys(&ctx, &env, ",");
        assert_eq!(point(&ctx), 2); // backward till of the previous comma
        keys(&ctx, &env, "0df,");
        assert_eq!(text(&ctx), "b,c,d");
    }
    #[test]
    fn failed_find_does_not_delete() {
        let (ctx, env) = editor("abc\nz");
        keys(&ctx, &env, "dfz");
        assert_eq!(text(&ctx), "abc\nz");
        keys(&ctx, &env, "l");
        assert_eq!(point(&ctx), 1);
    }
    #[test]
    fn line_delete_at_eof_and_undo() {
        let (ctx, env) = editor("one\ntwo");
        keys(&ctx, &env, "Gdd");
        assert_eq!(text(&ctx), "one");
        keys(&ctx, &env, "u");
        assert_eq!(text(&ctx), "one\ntwo");
    }
    #[test]
    fn change_line_keeps_next_line_separate() {
        let (ctx, env) = editor("one\ntwo");
        keys(&ctx, &env, "ccX");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "X\ntwo");
        keys(&ctx, &env, "u");
        assert_eq!(text(&ctx), "one\ntwo");
    }
    #[test]
    fn change_to_end_keeps_newline() {
        let (ctx, env) = editor("abc\ndef");
        keys(&ctx, &env, "lCX");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "aX\ndef");
    }
    #[test]
    fn visual_backward_includes_both_endpoints() {
        let (ctx, env) = editor("abcde");
        keys(&ctx, &env, "llvhd");
        assert_eq!(text(&ctx), "ade");
    }
    #[test]
    fn visual_single_character_region_is_nonempty() {
        let (ctx, env) = editor("abc");
        keys(&ctx, &env, "v");
        assert_eq!(run("(region-end)", &env, &ctx), LispExp::number(1.0));
        keys(&ctx, &env, "d");
        assert_eq!(text(&ctx), "bc");
    }
    #[test]
    fn visual_line_deletes_whole_lines() {
        let (ctx, env) = editor("abc\ndef\nghi");
        keys(&ctx, &env, "lVjd");
        assert_eq!(text(&ctx), "ghi");
    }
    #[test]
    fn visual_block_deletes_a_rectangle() {
        let (ctx, env) = editor("abcd\nefgh\nijkl");
        keys(&ctx, &env, "l");
        ctrl(&ctx, &env, 'v');
        keys(&ctx, &env, "jld");
        assert_eq!(text(&ctx), "ad\neh\nijkl");
    }
    #[test]
    fn block_yank_and_paste() {
        let (ctx, env) = editor("abcd\nefgh\nijkl");
        keys(&ctx, &env, "l");
        ctrl(&ctx, &env, 'v');
        keys(&ctx, &env, "jlyG0P");
        assert_eq!(text(&ctx), "abcd\nefgh\nbcijkl\nfg");
    }
    #[test]
    fn named_registers_append_and_blackhole() {
        let (ctx, env) = editor("one two three");
        keys(&ctx, &env, "\"ayiw w\"Ayiw");
        assert_eq!(
            run("(car (vim--register-value \"a\"))", &env, &ctx),
            LispExp::string("onetwo".into())
        );
        keys(&ctx, &env, "\"_dw");
        assert_eq!(
            run("(car (vim--register-value \"a\"))", &env, &ctx),
            LispExp::string("onetwo".into())
        );
    }
    #[test]
    fn yank_zero_survives_delete() {
        let (ctx, env) = editor("one two");
        keys(&ctx, &env, "yiwwdw");
        assert_eq!(
            run("(car (vim--register-value \"0\"))", &env, &ctx),
            LispExp::string("one".into())
        );
    }
    #[test]
    fn quote_word_and_around_word_objects() {
        let (ctx, env) = editor("\"alpha beta\" tail");
        keys(&ctx, &env, "ldi\"");
        assert_eq!(text(&ctx), "\"\" tail");
        keys(&ctx, &env, "wdiw");
        assert_eq!(text(&ctx), "\"\" ");
    }
    #[test]
    fn replace_character_and_dot() {
        let (ctx, env) = editor("abcdef");
        keys(&ctx, &env, "2rXll.");
        assert_eq!(text(&ctx), "XXcXXf");
    }
    #[test]
    fn shift_operators_apply_to_counted_lines() {
        let (ctx, env) = editor("a\nb\nc");
        keys(&ctx, &env, "2>>");
        assert_eq!(text(&ctx), "    a\n    b\nc");
        keys(&ctx, &env, "2<<");
        assert_eq!(text(&ctx), "a\nb\nc");
    }
    #[test]
    fn case_operators_and_pair_motion() {
        let (ctx, env) = editor("(foo bar)");
        keys(&ctx, &env, "gUi(");
        assert_eq!(text(&ctx), "(FOO BAR)");
        keys(&ctx, &env, "0%");
        assert_eq!(point(&ctx), 8);
    }
    #[test]
    fn macro_register_records_and_replays_insert_and_escape() {
        let (ctx, env) = editor("abc");
        keys(&ctx, &env, "qaiX");
        esc(&ctx, &env);
        keys(&ctx, &env, "q$a");
        esc(&ctx, &env);
        keys(&ctx, &env, "@a");
        assert_eq!(text(&ctx), "XabXc");
    }
    #[test]
    fn a_count_before_at_is_how_many_times_the_macro_plays_and_no_more() {
        let (ctx, env) = editor("abcdefghij");
        keys(&ctx, &env, "qaxq");
        assert_eq!(text(&ctx), "bcdefghij");
        keys(&ctx, &env, "3@a");
        assert_eq!(text(&ctx), "efghij");
        keys(&ctx, &env, "2@@");
        assert_eq!(text(&ctx), "ghij");
    }
    #[test]
    fn a_count_before_gg_or_ge_is_spent_by_it() {
        let (ctx, env) = editor("1\n2\n3\n4\n5\n6");
        keys(&ctx, &env, "4ggdd");
        assert_eq!(text(&ctx), "1\n2\n3\n5\n6");
        let (ctx, env) = editor("one two three four");
        run("(goto-char 15)", &env, &ctx);
        keys(&ctx, &env, "2gex");
        assert_eq!(text(&ctx), "one tw three four");
    }
    #[test]
    fn ge_goes_to_the_end_of_the_previous_word_from_its_first_character() {
        let (ctx, env) = editor("one two three four");
        run("(goto-char 8)", &env, &ctx);
        keys(&ctx, &env, "ge");
        assert_eq!(point(&ctx), 6);
        run("(goto-char 14)", &env, &ctx);
        keys(&ctx, &env, "2ge");
        assert_eq!(point(&ctx), 6);
        run("(goto-char 8)", &env, &ctx);
        keys(&ctx, &env, "dge");
        assert_eq!(text(&ctx), "one twhree four");
    }
    #[test]
    fn ge_from_white_space_or_after_punctuation_stops_at_the_nearest_end() {
        let (ctx, env) = editor("one  two foo.bar");
        run("(goto-char 4)", &env, &ctx);
        keys(&ctx, &env, "ge");
        assert_eq!(point(&ctx), 2);
        run("(goto-char 13)", &env, &ctx);
        keys(&ctx, &env, "ge");
        assert_eq!(point(&ctx), 12);
    }
    #[test]
    fn ap_takes_the_blank_lines_after_the_paragraph_and_ip_leaves_them() {
        let (ctx, env) = editor("a\nb\n\n\nc");
        keys(&ctx, &env, "dap");
        assert_eq!(text(&ctx), "c");
        keys(&ctx, &env, "u");
        keys(&ctx, &env, "dip");
        assert_eq!(text(&ctx), "\n\nc");
        let (ctx, env) = editor("a\n  \nb");
        keys(&ctx, &env, "dap");
        assert_eq!(text(&ctx), "b");
    }
    #[test]
    fn ap_on_the_last_paragraph_takes_the_blank_lines_before_it() {
        let (ctx, env) = editor("a\n\nb\nc");
        run("(goto-char 3)", &env, &ctx);
        keys(&ctx, &env, "dip");
        assert_eq!(text(&ctx), "a\n");
        keys(&ctx, &env, "u");
        run("(goto-char 3)", &env, &ctx);
        keys(&ctx, &env, "dap");
        assert_eq!(text(&ctx), "a");
    }
    #[test]
    fn paragraph_objects_count_a_run_of_blank_lines_as_vim_does() {
        let (ctx, env) = editor("a\n\n\nb\n\nc");
        run("(goto-char 2)", &env, &ctx);
        keys(&ctx, &env, "dip");
        assert_eq!(text(&ctx), "a\nb\n\nc");
        keys(&ctx, &env, "u");
        run("(goto-char 2)", &env, &ctx);
        keys(&ctx, &env, "dap");
        assert_eq!(text(&ctx), "a\n\nc");
        let (ctx, env) = editor("a\n\nb\n\nc");
        keys(&ctx, &env, "d2ip");
        assert_eq!(text(&ctx), "b\n\nc");
        keys(&ctx, &env, "u");
        keys(&ctx, &env, "d2ap");
        assert_eq!(text(&ctx), "c");
    }
    #[test]
    fn regex_search_wraps_and_repeats_in_both_directions() {
        let (ctx, env) = editor("foo bar foo");
        keys(&ctx, &env, "/");
        keys(&ctx, &env, "foo");
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
        assert_eq!(point(&ctx), 8);
        keys(&ctx, &env, "n");
        assert_eq!(point(&ctx), 0);
        keys(&ctx, &env, "N");
        assert_eq!(point(&ctx), 8);
    }
    #[test]
    fn marks_are_local_and_exact_jumps_work() {
        let (ctx, env) = editor("abc\ndef");
        keys(&ctx, &env, "lmagg`a");
        assert_eq!(point(&ctx), 1);
        keys(&ctx, &env, "'a");
        assert_eq!(point(&ctx), 0);
    }
    #[test]
    fn vim_state_is_buffer_local_and_keeps_major_mode() {
        let (ctx, env) = editor("abc");
        run(
            "(make-mode 'test-lang) (buffer-create \"other\" 'test-lang) (switch-to-buffer \"other\")",
            &env,
            &ctx,
        );
        keys(&ctx, &env, "i");
        assert_eq!(text(&ctx), "i");
        run("(vim-mode t)", &env, &ctx);
        assert_eq!(
            run("(major-mode)", &env, &ctx),
            LispExp::symbol("test-lang".into())
        );
        run("(vim-mode-disable)", &env, &ctx);
        keys(&ctx, &env, "x");
        assert_eq!(text(&ctx), "xi");
    }
    #[test]
    fn read_only_buffers_allow_yank_but_refuse_edits() {
        let (ctx, env) = editor("abc");
        run("(set-buffer-read-only t)", &env, &ctx);
        keys(&ctx, &env, "ddixrp");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "abc");
        keys(&ctx, &env, "yy");
        assert!(!run("(vim--register-value \"0\")", &env, &ctx).is_nil());
    }
    #[test]
    fn ctrl_u_belongs_to_vim_instead_of_emacs_argument_reader() {
        let (ctx, env) = editor("abc");
        keys(&ctx, &env, "Axyz");
        ctrl(&ctx, &env, 'u');
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "");
    }
    #[test]
    fn paste_in_insert_is_part_of_undo_and_dot() {
        let (ctx, env) = editor("abc");
        keys(&ctx, &env, "i");
        ctx.handle_paste("é🌍".into(), &env);
        esc(&ctx, &env);
        keys(&ctx, &env, "$.");
        assert_eq!(text(&ctx), "é🌍abé🌍c");
        keys(&ctx, &env, "u");
        assert_eq!(text(&ctx), "é🌍abc");
        keys(&ctx, &env, "u");
        assert_eq!(text(&ctx), "abc");
    }
    fn ex(ctx: &Ctx, env: &Arc<Env<Ctx>>, command: &str) {
        keys(ctx, env, ":");
        keys(ctx, env, command);
        ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), env);
        assert!(
            !ctx.get_logs().iter().any(|line| line.contains("Vim: ")),
            "Vim error: {:?}",
            ctx.get_logs()
        );
    }
    #[test]
    fn replace_mode_restores_overwritten_text_on_backspace() {
        let (ctx, env) = editor("abcde");
        keys(&ctx, &env, "RXY");
        ctx.handle_key_event(KeyEvent::new(KeyCode::Backspace), &env);
        keys(&ctx, &env, "Z");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "XZcde");
        keys(&ctx, &env, "u");
        assert_eq!(text(&ctx), "abcde");
        ctrl(&ctx, &env, 'r');
        assert_eq!(text(&ctx), "XZcde");
    }
    #[test]
    fn replace_mode_dot_replays_overwrite() {
        let (ctx, env) = editor("abcdef");
        keys(&ctx, &env, "RXY");
        esc(&ctx, &env);
        keys(&ctx, &env, "ll.");
        assert_eq!(text(&ctx), "XYcXYf");
    }
    #[test]
    fn normal_insert_replace_and_find_handle_unicode() {
        let (ctx, env) = editor("aéb🌍c");
        keys(&ctx, &env, "f🌍");
        assert_eq!(point(&ctx), 3);
        keys(&ctx, &env, "RΩ");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "aébΩc");
        keys(&ctx, &env, "0iλ");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "λaébΩc");
        keys(&ctx, &env, "Ω");
        assert_eq!(text(&ctx), "λaébΩc");
    }
    #[test]
    fn ex_substitution_replaces_once_per_line_and_globally() {
        let (ctx, env) = editor("one one\none one");
        ex(&ctx, &env, "s/one/X/");
        assert_eq!(text(&ctx), "X one\none one");
        ex(&ctx, &env, "%s/one/Y/g");
        assert_eq!(text(&ctx), "X Y\nY Y");
        keys(&ctx, &env, "u");
        assert_eq!(text(&ctx), "X one\none one");
    }
    #[test]
    fn ex_substitution_supports_ranges_and_captures() {
        let (ctx, env) = editor("a1\nb2\nc3");
        ex(&ctx, &env, "2,3s/([a-z])([0-9])/\\2-\\1-&/");
        assert_eq!(text(&ctx), "a1\n2-b-b2\n3-c-c3");
    }
    #[test]
    fn ex_substitution_with_escaped_delimiter_and_case_flag() {
        let (ctx, env) = editor("A/B a/b");
        ex(&ctx, &env, "s/a\\/b/X/gi");
        assert_eq!(text(&ctx), "X X");
    }
    #[test]
    fn ex_goto_and_settings() {
        let (ctx, env) = editor("one\ntwo\nthree");
        ex(&ctx, &env, "2");
        assert_eq!(point(&ctx), 4);
        ex(&ctx, &env, "set relativenumber");
        assert_eq!(
            env.get_variable("display-line-numbers").unwrap(),
            LispExp::symbol("relative".into())
        );
    }
    #[test]
    fn word_search_uses_boundaries() {
        let (ctx, env) = editor("foo foobar foo");
        keys(&ctx, &env, "*");
        assert_eq!(point(&ctx), 11);
    }
    #[test]
    fn visual_line_creates_overlay_and_escape_removes_it() {
        let (ctx, env) = editor("abc\ndef");
        keys(&ctx, &env, "V");
        assert!(!run("(overlays-at 2)", &env, &ctx).is_nil());
        esc(&ctx, &env);
        assert!(run("(overlays-at 2)", &env, &ctx).is_nil());
    }
    #[test]
    fn restoring_a_previous_local_map_preserves_language_bindings() {
        let (ctx, env) = editor("abc");
        run(
            "(vim-mode-disable) (make-mode 'prior) (define-key 'prior \"x\" '(insert \"!\")) (set-local-keymap 'prior) (vim-mode t) (vim-mode-disable)",
            &env,
            &ctx,
        );
        assert_eq!(
            run("(buffer-get 'vim-saved-keymap)", &env, &ctx),
            LispExp::symbol("prior".into())
        );
        assert_eq!(
            run("(buffer-get 'local-keymap)", &env, &ctx),
            LispExp::symbol("prior".into())
        );
        keys(&ctx, &env, "x");
        assert_eq!(text(&ctx), "!abc");
    }
    #[test]
    fn key_binding_reports_local_unicode_fallback_and_ctrl_u() {
        let (ctx, env) = editor("abc");
        assert_eq!(
            run("(nth 2 (key-binding \"Ω\"))", &env, &ctx),
            LispExp::string("local".into())
        );
        assert_eq!(
            run("(nth 2 (key-binding \"C-u\"))", &env, &ctx),
            LispExp::string("local".into())
        );
    }

    #[test]
    fn block_change_inserts_on_each_selected_row_and_repeats() {
        let (ctx, env) = editor("abcd\nefgh\nijkl\nmnop");
        keys(&ctx, &env, "l");
        ctrl(&ctx, &env, 'v');
        keys(&ctx, &env, "jlcX");
        esc(&ctx, &env);
        assert_eq!(text(&ctx), "aXd\neXh\nijkl\nmnop");
        keys(&ctx, &env, "jj.");
        assert_eq!(text(&ctx), "aXd\neXh\niXl\nmXp");
    }
    #[test]
    fn case_operators_repeat_linewise_and_visual_u_changes_case() {
        let (ctx, env) = editor("Abc\nDef");
        keys(&ctx, &env, "gUU");
        assert_eq!(text(&ctx), "ABC\nDef");
        keys(&ctx, &env, "j0vlu");
        assert_eq!(text(&ctx), "ABC\ndef");
    }
    #[test]
    fn an_operator_with_unset_mark_preserves_text() {
        let (ctx, env) = editor("abc\ndef");
        keys(&ctx, &env, "d'z");
        assert_eq!(text(&ctx), "abc\ndef");
    }
    #[test]
    fn g_ignores_a_trailing_newline_as_an_extra_physical_line() {
        let (ctx, env) = editor("abc\ndef\n");
        keys(&ctx, &env, "G");
        assert_eq!(point(&ctx), 4);
    }
    #[test]
    fn mode_line_exposes_the_vim_state() {
        let (ctx, env) = editor("abc");
        run("(setq mode-line-format \"%s\")", &env, &ctx);
        let line = ctx
            .snapshot(&env, 80, 24)
            .views
            .into_iter()
            .find(|v| v.buffer_name == "*scratch*")
            .unwrap()
            .mode_line
            .unwrap();
        assert_eq!(line, "NORMAL");
        keys(&ctx, &env, "i");
        let line = ctx
            .snapshot(&env, 80, 24)
            .views
            .into_iter()
            .find(|v| v.buffer_name == "*scratch*")
            .unwrap()
            .mode_line
            .unwrap();
        assert_eq!(line, "INSERT");
    }

    #[test]
    fn substitution_captures_honor_ignorecase() {
        let (ctx, env) = editor("ABC");
        ex(&ctx, &env, "s/(abc)/[\\1]-&/i");
        assert_eq!(text(&ctx), "[ABC]-ABC");
    }
    #[test]
    fn empty_line_dollar_delete_and_boundary_delete_preserve_text_and_dot() {
        let (ctx, env) = editor("\nabc");
        keys(&ctx, &env, "d$");
        assert_eq!(text(&ctx), "\nabc");
        keys(&ctx, &env, "jx0dh.");
        assert_eq!(text(&ctx), "\nc");
    }
    #[test]
    fn disabling_an_inactive_mode_preserves_another_keymap() {
        let (ctx, env) = editor("abc");
        run(
            "(vim-mode-disable) (make-mode 'other) (set-local-keymap 'other) (vim-mode-disable)",
            &env,
            &ctx,
        );
        assert_eq!(
            run("(buffer-get 'local-keymap)", &env, &ctx),
            LispExp::symbol("other".into())
        );
    }
    #[test]
    fn visual_ex_substitution_leaves_normal_state() {
        let (ctx, env) = editor("a\na\na");
        keys(&ctx, &env, "Vj");
        ex(&ctx, &env, "'<,'>s/a/X/g");
        assert_eq!(text(&ctx), "X\nX\na");
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("normal".into())
        );
    }

    #[test]
    fn listed_bindings_agree_with_the_character_fallback() {
        let (ctx, env) = editor("abc");
        run(
            "(vim-mode-disable) (make-mode 'local) (define-key 'local \"<char>\" 'self-insert) (set-local-keymap 'local)",
            &env,
            &ctx,
        );
        assert_eq!(
            run("(nth 2 (assoc \"x\" (keymap-bindings)))", &env, &ctx),
            LispExp::string("local".into())
        );
    }

    #[test]
    fn negative_upward_motions_work_for_k_and_the_up_arrow_with_counts() {
        for code in [KeyCode::Char('k'), KeyCode::Up] {
            let (ctx, env) = editor("zero\none\ntwo\nthree");
            keys(&ctx, &env, "Gll2");
            ctx.handle_key_event(KeyEvent::new(code), &env);
            assert_eq!(point(&ctx), 7, "two lines up, preserving column two");
        }
    }
    #[test]
    fn negative_page_motions_move_up_and_positive_ones_move_down() {
        let content = (1..=60)
            .map(|i| format!("line{i:02}\n"))
            .collect::<String>();
        for (key, start, count, expected) in [
            ('b', 40, 1, 18),
            ('b', 50, 2, 6),
            ('u', 40, 1, 28),
            ('u', 40, 2, 16),
            ('f', 10, 1, 32),
            ('d', 10, 1, 22),
        ] {
            let (ctx, env) = editor(&content);
            run(&format!("(goto-line {start})"), &env, &ctx);
            if count > 1 {
                keys(&ctx, &env, &count.to_string());
            }
            ctrl(&ctx, &env, key);
            assert_eq!(
                run("(line-number-at-point)", &env, &ctx),
                LispExp::number(expected as f64),
                "C-{key}, count {count}"
            );
        }
    }
    #[test]
    fn negative_upward_motion_preserves_goal_column_across_a_short_line() {
        let (ctx, env) = editor("abcdef\nx\nabcdef");
        keys(&ctx, &env, "G4lkk");
        assert_eq!(point(&ctx), 4);
        keys(&ctx, &env, "jj");
        assert_eq!(point(&ctx), 13);
    }
    #[test]
    fn negative_upward_motion_composes_with_operators_and_visual_selection() {
        let (ctx, env) = editor("a\nb\nc\nd");
        keys(&ctx, &env, "Gdk");
        assert_eq!(text(&ctx), "a\nb");
        let (ctx, env) = editor("a\nb\nc\nd");
        keys(&ctx, &env, "Gvk");
        assert_eq!(run("(region-beginning)", &env, &ctx), LispExp::number(4.0));
        assert_eq!(run("(region-end)", &env, &ctx), LispExp::number(7.0));
    }

    #[test]
    fn global_vim_applies_to_existing_and_future_buffers_without_moving_focus() {
        let (ctx, env) = editor("abc");
        run(
            r#"(vim-mode-auto)
               (buffer-create "existing")
               (global-vim-mode t)
               (buffer-create "future")"#,
            &env,
            &ctx,
        );
        for name in ["*scratch*", "existing", "future"] {
            assert_eq!(
                run(&format!("(buffer-get 'vim-state {name:?})"), &env, &ctx),
                LispExp::symbol("normal".into())
            );
        }
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        assert_eq!(ctx.focused_window_buffer().as_deref(), Some("*scratch*"));
        keys(&ctx, &env, "l");
        assert_eq!(point(&ctx), 1);
        assert_eq!(text(&ctx), "abc");
        run("(global-vim-mode -1)", &env, &ctx);
        for name in ["*scratch*", "existing", "future"] {
            assert!(run(&format!("(buffer-get 'vim-state {name:?})"), &env, &ctx).is_nil());
        }
    }

    #[test]
    fn automatic_vim_excludes_service_modes_and_internal_buffers() {
        let (ctx, env) = editor("");
        run(
            r#"(make-mode 'help-mode)
               (make-mode 'dired-mode)
               (global-vim-mode t)
               (buffer-create "directory" 'dired-mode)
               (buffer-create "help" 'help-mode)
               (buffer-create "*internal*")
               (buffer-create "notes")"#,
            &env,
            &ctx,
        );
        for name in ["directory", "help", "*internal*"] {
            assert!(run(&format!("(buffer-get 'vim-state {name:?})"), &env, &ctx).is_nil());
        }
        assert_eq!(
            run("(buffer-get 'vim-state \"notes\")", &env, &ctx),
            LispExp::symbol("normal".into())
        );
        // Even a custom prompt mode must keep its own typing map.
        run(
            r#"(setq vim-mode-excluded-modes nil vim-mode-predicate nil)
               (make-floating-window "*Minibuffer*" 0 0 30 5 "Prompt" 'fundamental-mode)"#,
            &env,
            &ctx,
        );
        assert!(run("(vim--state)", &env, &ctx).is_nil());
        keys(&ctx, &env, "hello");
        assert_eq!(text(&ctx), "hello");
    }

    #[test]
    fn explicit_buffer_choices_override_global_policy_until_returned_to_auto() {
        let (ctx, env) = editor("abc");
        // The initial buffer was enabled manually by editor().
        run(
            r#"(global-vim-mode t)
               (buffer-create "auto")
               (switch-to-buffer "auto")
               (vim-mode-disable)
               (vim-mode-refresh)"#,
            &env,
            &ctx,
        );
        assert!(run("(vim--state)", &env, &ctx).is_nil());
        run("(vim-mode-auto)", &env, &ctx);
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("normal".into())
        );
        run("(global-vim-mode -1)", &env, &ctx);
        assert!(run("(vim--state)", &env, &ctx).is_nil());
        run(r#"(switch-to-buffer "*scratch*")"#, &env, &ctx);
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("normal".into())
        );
        run("(vim-mode-auto)", &env, &ctx);
        assert!(run("(vim--state)", &env, &ctx).is_nil());
    }

    #[test]
    fn mode_policy_tracks_major_mode_changes_and_restores_previous_local_map() {
        let (ctx, env) = editor("");
        run(
            r#"(make-mode 'code-mode)
               (make-mode 'previous-map)
               (setq vim-mode-modes '(code-mode))
               (buffer-create "code")
               (switch-to-buffer "code")
               (set-local-keymap 'previous-map)
               (set-major-mode 'code-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("normal".into())
        );
        keys(&ctx, &env, "iabc");
        run("(set-major-mode 'code-mode) (vim-mode-refresh)", &env, &ctx);
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("insert".into())
        );
        run("(set-major-mode 'fundamental-mode)", &env, &ctx);
        assert!(run("(vim--state)", &env, &ctx).is_nil());
        assert_eq!(
            run("(buffer-get 'local-keymap)", &env, &ctx),
            LispExp::symbol("previous-map".into())
        );
        assert_eq!(text(&ctx), "abc");
        run("(set-major-mode 'code-mode)", &env, &ctx);
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("normal".into())
        );
    }

    #[test]
    fn global_refresh_and_reenable_preserve_insert_visual_and_pending_operator() {
        let (ctx, env) = editor("one two three");
        keys(&ctx, &env, "iX");
        run(
            r#"(global-vim-mode t) (vim-mode-enable) (buffer-create "background")
               (switch-to-buffer "background") (switch-to-buffer "*scratch*")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("insert".into())
        );
        keys(&ctx, &env, "Y");
        esc(&ctx, &env);
        keys(&ctx, &env, "0vl");
        let region = run("(list (region-beginning) (region-end))", &env, &ctx);
        run("(vim-mode-refresh)", &env, &ctx);
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("visual".into())
        );
        assert_eq!(
            run("(list (region-beginning) (region-end))", &env, &ctx),
            region
        );
        esc(&ctx, &env);
        keys(&ctx, &env, "0d");
        run(r#"(buffer-create "another-background")"#, &env, &ctx);
        keys(&ctx, &env, "w");
        assert_eq!(text(&ctx), "two three");
    }

    #[test]
    fn installing_a_hidden_vim_map_preserves_the_focused_buffers_key_prefix() {
        let (ctx, env) = editor("abc");
        run(
            r#"(global-vim-mode t)
               (define-key nil "C-x z" '(buffer-put 'prefix-ran t))"#,
            &env,
            &ctx,
        );
        ctrl(&ctx, &env, 'x');
        run(r#"(buffer-create "hidden")"#, &env, &ctx);
        keys(&ctx, &env, "z");
        assert_eq!(run("(buffer-get 'prefix-ran)", &env, &ctx), LispExp::t());
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn custom_predicate_can_restrict_automatic_vim_to_file_buffers() {
        let (ctx, env) = editor("");
        let path =
            std::env::temp_dir().join(format!("rsedit-vim-policy-{}.new", std::process::id()));
        run(
            r#"(vim-mode-auto)
               (setq vim-mode-predicate (lambda () (buffer-file-name)))
               (global-vim-mode t)
               (buffer-create "notes")"#,
            &env,
            &ctx,
        );
        assert!(run("(buffer-get 'vim-state \"*scratch*\")", &env, &ctx).is_nil());
        assert!(run("(buffer-get 'vim-state \"notes\")", &env, &ctx).is_nil());
        run(
            &format!("(find-file {:?})", path.to_str().unwrap()),
            &env,
            &ctx,
        );
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("normal".into())
        );
        keys(&ctx, &env, "iabc");
        run(
            &format!("(find-file {:?})", path.to_str().unwrap()),
            &env,
            &ctx,
        );
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("insert".into())
        );
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn per_mode_policy_survives_global_disable_and_can_be_refreshed() {
        let (ctx, env) = editor("");
        run(
            r#"(make-mode 'code-mode)
               (setq vim-mode-modes '(code-mode))
               (buffer-create "code" 'code-mode)
               (global-vim-mode t) (global-vim-mode 0)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(buffer-get 'vim-state \"code\")", &env, &ctx),
            LispExp::symbol("normal".into())
        );
        run("(setq vim-mode-modes nil) (vim-mode-refresh)", &env, &ctx);
        assert!(run("(buffer-get 'vim-state \"code\")", &env, &ctx).is_nil());
        run("(vim-mode 0)", &env, &ctx);
        assert!(run("(vim--state)", &env, &ctx).is_nil());
    }

    #[test]
    fn replacement_scratch_receives_automatic_vim_after_last_buffer_is_closed() {
        let (ctx, env) = editor("");
        run(
            r#"(global-vim-mode t) (close-buffer "*scratch*")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("normal".into())
        );
        keys(&ctx, &env, "iabc");
        assert_eq!(text(&ctx), "abc");
    }

    #[test]
    fn visiting_a_file_after_save_as_refreshes_policy_without_changing_major_mode() {
        let (ctx, env) = editor("abc");
        let path = std::env::temp_dir().join(format!(
            "rsedit-vim-save-policy-{}.unknown",
            std::process::id()
        ));
        run(
            r#"(vim-mode-auto)
               (setq vim-mode-predicate (lambda () (buffer-file-name)))
               (global-vim-mode t)"#,
            &env,
            &ctx,
        );
        assert!(run("(vim--state)", &env, &ctx).is_nil());
        run(
            &format!("(write-file {:?})", path.to_str().unwrap()),
            &env,
            &ctx,
        );
        assert_eq!(
            run("(major-mode)", &env, &ctx),
            LispExp::symbol("fundamental-mode".into())
        );
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("normal".into())
        );
        keys(&ctx, &env, "iX");
        run(
            &format!("(write-file {:?})", path.to_str().unwrap()),
            &env,
            &ctx,
        );
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("insert".into())
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn major_mode_hooks_can_replace_the_map_under_an_active_vim_session() {
        let (ctx, env) = editor("abc");
        run(
            r#"(make-mode 'code-mode)
               (make-mode 'code-map)
               (defun code-setup () (set-local-keymap 'code-map))
               (add-hook 'code-mode "major-mode-hook" 'code-setup)"#,
            &env,
            &ctx,
        );
        keys(&ctx, &env, "iX");
        run("(set-major-mode 'code-mode)", &env, &ctx);
        assert_eq!(
            run("(vim--state)", &env, &ctx),
            LispExp::symbol("insert".into())
        );
        assert_eq!(
            run("(buffer-get 'local-keymap)", &env, &ctx),
            LispExp::symbol("vim-insert-map".into())
        );
        esc(&ctx, &env);
        run("(vim-mode-disable)", &env, &ctx);
        assert_eq!(
            run("(buffer-get 'local-keymap)", &env, &ctx),
            LispExp::symbol("code-map".into())
        );
    }

    #[test]
    fn interactive_global_toggle_honors_negative_prefix_argument() {
        let (ctx, env) = editor("");
        run(
            r#"(vim-mode-auto)
               (define-key nil "M-v" 'global-vim-mode)"#,
            &env,
            &ctx,
        );
        let mut toggle = KeyEvent::new(KeyCode::Char('v'));
        toggle.modifiers.alt = true;
        ctx.handle_key_event(toggle.clone(), &env);
        assert_eq!(run("global-vim-mode", &env, &ctx), LispExp::t());
        assert!(!run("(vim--state)", &env, &ctx).is_nil());
        // Where vim is on, C-u is one of its keys, so the prefix is typed in a
        // buffer vim does not suit. C-u - turns the mode off, and keeps it off.
        run(
            r#"(buffer-create "*elsewhere*") (switch-to-buffer "*elsewhere*")"#,
            &env,
            &ctx,
        );
        for _ in 0..2 {
            ctrl(&ctx, &env, 'u');
            keys(&ctx, &env, "-");
            ctx.handle_key_event(toggle.clone(), &env);
            assert!(run("global-vim-mode", &env, &ctx).is_nil());
        }
        let scratch = r#"(with-current-buffer "*scratch*" (lambda () (vim--state)))"#;
        assert!(run(scratch, &env, &ctx).is_nil());
    }
    #[test]
    fn negative_prefix_argument_turns_vim_mode_off() {
        let (ctx, env) = editor("abc");
        run(
            r#"(vim-mode-disable) (define-key nil "C-c v" 'vim-mode)"#,
            &env,
            &ctx,
        );
        ctrl(&ctx, &env, 'u');
        keys(&ctx, &env, "-");
        ctrl(&ctx, &env, 'c');
        keys(&ctx, &env, "v");
        assert!(run("(vim--state)", &env, &ctx).is_nil());
        assert!(run("(vim-mode t) (vim-mode '-) (vim--state)", &env, &ctx).is_nil());
    }
    #[test]
    fn hidden_initialization_and_global_refresh_preserve_buffer_selection_history() {
        let (ctx, env) = editor("");
        run(
            r#"(buffer-create "z-last-visited")
               (switch-to-buffer "z-last-visited")
               (switch-to-buffer "*scratch*")
               (buffer-create "a-hidden")
               (global-vim-mode t) (vim-mode-refresh)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            ctx.most_recent_buffer("*scratch*").as_deref(),
            Some("z-last-visited")
        );
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        run(r#"(close-buffer "*scratch*")"#, &env, &ctx);
        assert_eq!(ctx.get_current_buffer_name(), "z-last-visited");
        assert_eq!(
            ctx.focused_window_buffer().as_deref(),
            Some("z-last-visited")
        );
    }
}
