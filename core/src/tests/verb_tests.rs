//! The small verbs: what each one changes, and where it leaves point.
//!
//! # What is worth testing here
//!
//! Two things, and they are not the obvious one. Whether `M-u` upcases a word
//! is not in much doubt; where it leaves point is, and that is half of what
//! makes the key usable -- `M-u M-u M-u` walks up three words only because
//! each press ends after the word it changed. So every test below says where
//! point ended.
//!
//! The other is the edges: the first line has nothing above it, the end of a
//! line has no character after it, and a buffer can end without a newline.
//! Each of those is where a verb either says so or corrupts something, and
//! none of them comes up while trying the feature out by hand.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, Mark, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, LispExp, Parser, eval};
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

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|buf| buf.text.to_string())
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d())
    }

    fn at(ctx: &Ctx, offset: usize) {
        ctx.with_current_buffer_mut(|buf| {
            let (line, column) = buf.text.cursor_1d_to_2d(offset);
            buf.text.cursor_move(line, column);
        });
    }

    fn select(ctx: &Ctx, mark: usize, offset: usize) {
        at(ctx, offset);
        ctx.with_current_buffer_mut(|buf| buf.mark = Some(Mark::new(mark)));
    }

    fn modified(ctx: &Ctx) -> bool {
        ctx.with_current_buffer(|buf| buf.is_modified)
    }

    fn press(ch: char, ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    // ----------------------------------------------------------------
    // Case
    // ----------------------------------------------------------------

    #[test]
    fn upcase_word_takes_the_word_after_point_and_steps_over_it() {
        let (ctx, env) = editor("one two three");
        run("(upcase-word)", &env, &ctx);
        assert_eq!(text(&ctx), "ONE two three");
        assert_eq!(point(&ctx), 3, "point should be after the word it changed");
        run("(upcase-word)", &env, &ctx);
        assert_eq!(text(&ctx), "ONE TWO three");
        assert_eq!(point(&ctx), 7);
    }

    #[test]
    fn a_word_is_what_forward_word_would_have_stepped_over() {
        // Same two functions as `forward-word`, so `M-t` and `M-f` agree
        // about the underscore. A separate notion of a word written here
        // would only ever show up as "sometimes it eats the underscore".
        let (ctx, env) = editor("some_name other");
        run("(upcase-word)", &env, &ctx);
        assert_eq!(text(&ctx), "SOME_NAME other");
    }

    #[test]
    fn downcase_and_capitalize_do_what_they_say() {
        let (ctx, env) = editor("SHOUTING words");
        run("(downcase-word)", &env, &ctx);
        assert_eq!(text(&ctx), "shouting words");
        at(&ctx, 0);
        run("(capitalize-word)", &env, &ctx);
        assert_eq!(text(&ctx), "Shouting words");
    }

    #[test]
    fn capitalize_lowercases_the_rest_of_the_word() {
        let (ctx, env) = editor("mIxEd");
        run("(capitalize-word)", &env, &ctx);
        assert_eq!(text(&ctx), "Mixed");
    }

    #[test]
    fn a_region_is_taken_instead_of_a_word_and_point_stays_put() {
        let (ctx, env) = editor("one two three");
        select(&ctx, 0, 7);
        run("(upcase-word)", &env, &ctx);
        assert_eq!(text(&ctx), "ONE TWO three");
        assert_eq!(point(&ctx), 7, "point should not have moved");
    }

    #[test]
    fn capitalizing_a_region_capitalizes_every_word_in_it() {
        // The same thing six presses would have done, which is what somebody
        // selecting six words is asking for.
        let (ctx, env) = editor("one two three");
        select(&ctx, 0, 13);
        run("(capitalize-word)", &env, &ctx);
        assert_eq!(text(&ctx), "One Two Three");
    }

    #[test]
    fn recasing_what_is_already_cased_that_way_changes_nothing_at_all() {
        // Not an optimisation: an edit that changed no characters would still
        // mark the buffer modified, and a file you only looked at would need
        // saving.
        let (ctx, env) = editor("ALREADY loud");
        assert_eq!(run("(upcase-word)", &env, &ctx), LispExp::nil());
        assert!(!modified(&ctx), "the buffer should not be modified");
        assert_eq!(point(&ctx), 7, "point should still step over the word");
    }

    // ----------------------------------------------------------------
    // Whitespace
    // ----------------------------------------------------------------

    #[test]
    fn delete_horizontal_space_takes_both_sides() {
        let (ctx, env) = editor("one    two");
        at(&ctx, 5);
        run("(delete-horizontal-space)", &env, &ctx);
        assert_eq!(text(&ctx), "onetwo");
        assert_eq!(point(&ctx), 3);
    }

    #[test]
    fn delete_horizontal_space_cannot_join_two_lines() {
        // Newlines are not horizontal space. A verb that removed one would
        // join lines by accident on the key people press to tidy up.
        let (ctx, env) = editor("one   \n   two");
        at(&ctx, 6);
        run("(delete-horizontal-space)", &env, &ctx);
        assert_eq!(text(&ctx), "one\n   two");
    }

    #[test]
    fn just_one_space_leaves_exactly_one() {
        let (ctx, env) = editor("one    two");
        at(&ctx, 5);
        run("(just-one-space)", &env, &ctx);
        assert_eq!(text(&ctx), "one two");
        assert_eq!(point(&ctx), 4, "point should be after the space");
    }

    #[test]
    fn just_one_space_puts_one_in_where_there_was_none() {
        let (ctx, env) = editor("onetwo");
        at(&ctx, 3);
        run("(just-one-space)", &env, &ctx);
        assert_eq!(text(&ctx), "one two");
    }

    #[test]
    fn delete_blank_lines_leaves_one_of_a_run() {
        let (ctx, env) = editor("one\n\n\n\n\ntwo\n");
        at(&ctx, 6); // inside the run
        run("(delete-blank-lines)", &env, &ctx);
        assert_eq!(text(&ctx), "one\n\ntwo\n");
    }

    #[test]
    fn delete_blank_lines_removes_a_lone_one() {
        let (ctx, env) = editor("one\n\ntwo\n");
        at(&ctx, 4);
        run("(delete-blank-lines)", &env, &ctx);
        assert_eq!(text(&ctx), "one\ntwo\n");
    }

    #[test]
    fn delete_blank_lines_on_a_line_with_text_clears_what_follows_it() {
        let (ctx, env) = editor("one\n\n\n\ntwo\n");
        at(&ctx, 1);
        run("(delete-blank-lines)", &env, &ctx);
        assert_eq!(text(&ctx), "one\ntwo\n");
        assert_eq!(point(&ctx), 1, "point should not have moved");
    }

    #[test]
    fn delete_blank_lines_on_a_line_with_nothing_after_it_does_nothing() {
        let (ctx, env) = editor("one\ntwo\n");
        at(&ctx, 1);
        assert_eq!(run("(delete-blank-lines)", &env, &ctx), LispExp::nil());
        assert_eq!(text(&ctx), "one\ntwo\n");
    }

    #[test]
    fn back_to_indentation_goes_to_the_code_rather_than_the_margin() {
        let (ctx, env) = editor("    deep();\n");
        at(&ctx, 9);
        run("(back-to-indentation)", &env, &ctx);
        assert_eq!(point(&ctx), 4);
    }

    #[test]
    fn back_to_indentation_on_a_blank_line_stops_at_its_end() {
        let (ctx, env) = editor("    \nx\n");
        at(&ctx, 0);
        run("(back-to-indentation)", &env, &ctx);
        assert_eq!(point(&ctx), 4);
    }

    // ----------------------------------------------------------------
    // Lines
    // ----------------------------------------------------------------

    #[test]
    fn join_line_pulls_this_line_up_with_one_space() {
        let (ctx, env) = editor("one\n    two\n");
        at(&ctx, 8);
        run("(join-line)", &env, &ctx);
        assert_eq!(text(&ctx), "one two\n");
        assert_eq!(point(&ctx), 4, "point should be at the join");
    }

    #[test]
    fn join_line_leaves_no_space_before_a_closing_delimiter() {
        // Asked of the mode's syntax table rather than a list of brackets
        // kept here: which characters close something is a property of the
        // language, and the editor already knows it.
        let (ctx, env) = editor("foo(\n)\n");
        run(
            r#"(make-mode 'probe-mode)
               (set-syntax-pairs 'probe-mode "()")"#,
            &env,
            &ctx,
        );
        ctx.with_current_buffer_mut(|buf| buf.current_mode = "probe-mode".to_string());
        at(&ctx, 5);
        run("(join-line)", &env, &ctx);
        assert_eq!(text(&ctx), "foo()\n");
    }

    #[test]
    fn join_line_on_the_first_line_says_there_is_nothing_above() {
        let (ctx, env) = editor("one\ntwo\n");
        at(&ctx, 1);
        assert_eq!(run("(join-line)", &env, &ctx), LispExp::nil());
        assert_eq!(text(&ctx), "one\ntwo\n");
        assert!(
            ctx.snapshot(&env, 80, 24).echo_message.contains("join"),
            "the user should be told: {:?}",
            ctx.snapshot(&env, 80, 24).echo_message
        );
    }

    #[test]
    fn duplicate_line_puts_the_copy_underneath() {
        let (ctx, env) = editor("one\ntwo\n");
        at(&ctx, 1);
        run("(duplicate-line)", &env, &ctx);
        assert_eq!(text(&ctx), "one\none\ntwo\n");
        assert_eq!(point(&ctx), 1, "point should not have moved");
    }

    #[test]
    fn duplicate_line_works_on_a_last_line_with_no_newline() {
        // A buffer that does not end in a newline is the case that turns a
        // copy into a line and a half.
        let (ctx, env) = editor("one\ntwo");
        at(&ctx, 5);
        run("(duplicate-line)", &env, &ctx);
        assert_eq!(text(&ctx), "one\ntwo\ntwo");
    }

    #[test]
    fn transpose_lines_swaps_this_one_with_the_one_above() {
        let (ctx, env) = editor("one\ntwo\nthree\n");
        at(&ctx, 5); // on `two`
        run("(transpose-lines)", &env, &ctx);
        assert_eq!(text(&ctx), "two\none\nthree\n");
        assert_eq!(point(&ctx), 1, "point should follow the text it was in");
    }

    #[test]
    fn transpose_lines_on_the_first_line_says_there_is_nothing_above() {
        let (ctx, env) = editor("one\ntwo\n");
        at(&ctx, 0);
        assert_eq!(run("(transpose-lines)", &env, &ctx), LispExp::nil());
        assert_eq!(text(&ctx), "one\ntwo\n");
    }

    // ----------------------------------------------------------------
    // Transposition
    // ----------------------------------------------------------------

    #[test]
    fn transpose_chars_swaps_around_point_and_steps_on() {
        let (ctx, env) = editor("abcd");
        at(&ctx, 2);
        run("(transpose-chars)", &env, &ctx);
        assert_eq!(text(&ctx), "acbd");
        assert_eq!(point(&ctx), 3);
    }

    #[test]
    fn transpose_chars_at_the_end_of_a_line_swaps_the_two_before_it() {
        // Which is what makes it the fix for a typo you have just finished
        // typing: there is nothing after point to swap with.
        let (ctx, env) = editor("teh\nnext\n");
        at(&ctx, 3);
        run("(transpose-chars)", &env, &ctx);
        assert_eq!(text(&ctx), "the\nnext\n");
        assert_eq!(point(&ctx), 3);
    }

    #[test]
    fn transpose_chars_at_the_start_of_a_line_has_nothing_to_swap() {
        let (ctx, env) = editor("one\ntwo\n");
        at(&ctx, 4);
        assert_eq!(run("(transpose-chars)", &env, &ctx), LispExp::nil());
        assert_eq!(text(&ctx), "one\ntwo\n");
    }

    #[test]
    fn transpose_words_swaps_across_whatever_is_between_them() {
        let (ctx, env) = editor("alpha, beta");
        at(&ctx, 5); // just after `alpha`
        run("(transpose-words)", &env, &ctx);
        assert_eq!(text(&ctx), "beta, alpha");
        assert_eq!(point(&ctx), 11, "point should be after the second word");
    }

    #[test]
    fn transpose_words_from_inside_the_first_word() {
        let (ctx, env) = editor("alpha beta");
        at(&ctx, 6);
        run("(transpose-words)", &env, &ctx);
        assert_eq!(text(&ctx), "beta alpha");
    }

    #[test]
    fn transpose_words_with_no_word_behind_does_nothing() {
        let (ctx, env) = editor("alpha beta");
        at(&ctx, 0);
        assert_eq!(run("(transpose-words)", &env, &ctx), LispExp::nil());
        assert_eq!(text(&ctx), "alpha beta");
    }

    // ----------------------------------------------------------------
    // Zapping
    // ----------------------------------------------------------------

    #[test]
    fn zap_to_char_kills_up_to_and_including_the_character() {
        let (ctx, env) = editor("one, two, three");
        run("(zap-to-char)", &env, &ctx);
        press(',', &ctx, &env);
        assert_eq!(text(&ctx), " two, three");
        assert_eq!(point(&ctx), 0);
    }

    #[test]
    fn what_zap_killed_can_be_put_back() {
        let (ctx, env) = editor("one, two");
        run("(zap-to-char)", &env, &ctx);
        press(',', &ctx, &env);
        assert_eq!(
            run("(current-kill 0)", &env, &ctx),
            LispExp::string("one,".into())
        );
    }

    #[test]
    fn zap_to_char_says_so_when_the_character_is_not_there() {
        let (ctx, env) = editor("one two");
        run("(zap-to-char)", &env, &ctx);
        press(';', &ctx, &env);
        assert_eq!(text(&ctx), "one two");
        assert!(
            ctx.snapshot(&env, 80, 24).echo_message.contains(';'),
            "the user should be told which character: {:?}",
            ctx.snapshot(&env, 80, 24).echo_message
        );
    }

    #[test]
    fn zap_to_char_refuses_a_key_that_is_not_a_character() {
        // The capture reports `C-x` as a key sequence, not a character to
        // zap to, and a verb that took its first letter would zap to the `C`
        // of `Cat` -- which is why there is one in the buffer.
        let (ctx, env) = editor("a Cat x two");
        run("(zap-to-char)", &env, &ctx);
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('x'),
                modifiers: KeyModifiers {
                    ctrl: true,
                    ..KeyModifiers::default()
                },
            },
            &env,
        );
        assert_eq!(text(&ctx), "a Cat x two");
    }

    #[test]
    fn zapping_does_not_leave_the_next_keystroke_captured() {
        let (ctx, env) = editor("one, two");
        run("(zap-to-char)", &env, &ctx);
        press(',', &ctx, &env);
        let before = text(&ctx);
        press('z', &ctx, &env);
        assert_ne!(text(&ctx), before, "typing should work again");
    }

    // ----------------------------------------------------------------
    // The family
    // ----------------------------------------------------------------

    #[test]
    fn every_verb_is_a_command_anybody_can_run_by_name() {
        let (ctx, env) = editor("");
        for name in [
            "upcase-word",
            "downcase-word",
            "capitalize-word",
            "delete-horizontal-space",
            "just-one-space",
            "delete-blank-lines",
            "back-to-indentation",
            "join-line",
            "duplicate-line",
            "transpose-lines",
            "transpose-chars",
            "transpose-words",
            "zap-to-char",
        ] {
            assert!(
                !run(&format!("(commandp '{name})"), &env, &ctx).is_nil(),
                "{name} should be a command"
            );
        }
    }

    #[test]
    fn the_keys_the_verbs_are_bound_to_are_spelt_in_a_way_the_parser_accepts() {
        // `M-\\' and `M-<space>' are the two spellings most likely to be
        // written in a way `define-key' cannot read -- and a binding it
        // cannot read is dropped with a line in the log nobody sees, leaving
        // a key that does nothing at all the day somebody presses it.
        let (ctx, env) = editor("");
        for (name, source) in [
            ("commands", include_str!("../../lisp/commands.lisp")),
            (
                "common-keymaps",
                include_str!("../../lisp/common-keymaps.lisp"),
            ),
        ] {
            let ast = Parser::new(&format!("(progn {source})"))
                .next()
                .unwrap_or_else(|why| panic!("{name}.lisp must parse: {why:?}"));
            eval(&ast, env.clone(), &ctx)
                .unwrap_or_else(|why| panic!("loading {name}.lisp: {why:?}"));
        }
        for (keys, command) in [
            ("M-u", "upcase-word"),
            ("M-l", "downcase-word"),
            ("M-c", "capitalize-word"),
            // Two backslashes here so the Lisp source the test builds reads
            // `"M-\\"`, which is how a lone backslash is written in a string.
            ("M-\\\\", "delete-horizontal-space"),
            ("M-<space>", "just-one-space"),
            ("C-x C-o", "delete-blank-lines"),
            ("M-m", "back-to-indentation"),
            ("M-^", "join-line"),
            ("C-t", "transpose-chars"),
            ("M-t", "transpose-words"),
            ("C-x C-t", "transpose-lines"),
            ("M-z", "zap-to-char"),
        ] {
            let found = run(&format!(r#"(key-binding "{keys}")"#), &env, &ctx);
            let printed = format!("{found:?}");
            assert!(
                printed.contains(command),
                "{keys} should run {command}, got {printed}"
            );
        }
    }

    #[test]
    fn a_read_only_buffer_refuses_all_of_them() {
        let (ctx, env) = editor("one two\nthree\n");
        run("(set-buffer-read-only t)", &env, &ctx);
        at(&ctx, 8);
        for verb in [
            "(upcase-word)",
            "(just-one-space)",
            "(join-line)",
            "(duplicate-line)",
            "(transpose-lines)",
            "(transpose-chars)",
            "(transpose-words)",
        ] {
            run(verb, &env, &ctx);
        }
        assert_eq!(text(&ctx), "one two\nthree\n");
    }
}
