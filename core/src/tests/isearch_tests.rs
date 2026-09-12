//! Incremental search, and the primitives it is built out of.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
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

    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| {
            b.text = GapBuffer::from(text);
            b.text.cursor_move(0, 0);
            b.is_modified = false;
        });
        (ctx, env)
    }

    /// Point in `*scratch*` -- the buffer being searched, which is *not* the
    /// current buffer while a prompt is open.
    fn point(ctx: &Ctx) -> usize {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .text
            .cursor_pos_1d()
    }

    fn mark_of(ctx: &Ctx) -> Option<(usize, bool)> {
        ctx.get_buffer("*scratch*")
            .expect("*scratch*")
            .read()
            .unwrap()
            .mark
            .map(|mark| (mark.at, mark.active))
    }

    fn goto(ctx: &Ctx, offset: usize) {
        ctx.mutate_buffer(ctx.get_buffer("*scratch*").expect("*scratch*"), |b| {
            let (line, col) = b.text.cursor_1d_to_2d(offset);
            b.text.cursor_move(line, col);
        });
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode) {
        ctx.handle_key_event(
            KeyEvent {
                code,
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    fn press_ctrl(ctx: &Ctx, env: &Arc<Env<Ctx>>, c: char) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
            env,
        );
    }

    /// Type into the open prompt, one keystroke at a time -- which is the only
    /// way to exercise this at all, since the search is driven by the hook that
    /// runs *after each command*.
    fn type_pattern(ctx: &Ctx, env: &Arc<Env<Ctx>>, text: &str) {
        for c in text.chars() {
            press(ctx, env, KeyCode::Char(c));
        }
    }

    fn echo(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
        ctx.snapshot(env, W, H).echo_message
    }

    fn start_isearch(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        eval_str("(isearch-forward)", env, ctx).expect("starting the search");
    }

    // -----------------------------------------------------------------------
    // The primitives it is built out of
    // -----------------------------------------------------------------------

    #[test]
    fn point_reports_where_point_is() {
        let (ctx, env) = editor_with("hello world");
        assert_eq!(
            eval_str("(point)", &env, &ctx).expect("point"),
            LispExp::number(0.0)
        );
        goto(&ctx, 6);
        assert_eq!(
            eval_str("(point)", &env, &ctx).expect("point"),
            LispExp::number(6.0)
        );
    }

    #[test]
    fn point_min_and_point_max_name_both_ends() {
        let (ctx, env) = editor_with("abcde");
        assert_eq!(
            eval_str("(point-min)", &env, &ctx).expect("point-min"),
            LispExp::number(0.0)
        );
        assert_eq!(
            eval_str("(point-max)", &env, &ctx).expect("point-max"),
            LispExp::number(5.0)
        );
    }

    #[test]
    fn goto_char_moves_point_and_says_where_it_landed() {
        let (ctx, env) = editor_with("hello world");
        assert_eq!(
            eval_str("(goto-char 6)", &env, &ctx).expect("goto-char"),
            LispExp::number(6.0)
        );
        assert_eq!(point(&ctx), 6);
    }

    /// A position computed before an edit can be outside the buffer after it.
    /// Landing somewhere real beats refusing.
    #[test]
    fn goto_char_clamps_rather_than_failing() {
        let (ctx, env) = editor_with("abc");
        assert_eq!(
            eval_str("(goto-char 999)", &env, &ctx).expect("goto-char"),
            LispExp::number(3.0)
        );
        assert_eq!(point(&ctx), 3);

        assert_eq!(
            eval_str("(goto-char -5)", &env, &ctx).expect("goto-char"),
            LispExp::number(0.0)
        );
        assert_eq!(point(&ctx), 0);
    }

    #[test]
    fn goto_char_and_point_round_trip_through_a_multi_line_buffer() {
        let (ctx, env) = editor_with("one\ntwo\nthree");
        for offset in [0, 3, 4, 7, 8, 13] {
            eval_str(&format!("(goto-char {offset})"), &env, &ctx).expect("goto-char");
            assert_eq!(
                eval_str("(point)", &env, &ctx).expect("point"),
                LispExp::number(offset as f64),
                "offset {offset} must survive the trip through line and column"
            );
        }
    }

    #[test]
    fn with_current_buffer_acts_on_another_buffer_and_comes_back() {
        let (ctx, env) = editor_with("in scratch");
        let got = eval_str(
            r#"(progn (buffer-create "other")
                      (with-current-buffer "other" (lambda () (current-buffer))))"#,
            &env,
            &ctx,
        )
        .expect("with-current-buffer");

        assert_eq!(got, LispExp::string("other".into()));
        assert_eq!(
            eval_str("(current-buffer)", &env, &ctx).expect("current-buffer"),
            LispExp::string("*scratch*".into()),
            "the buffer that was current before must be current again"
        );
    }

    /// An error escaping with the buffer still switched would strand every
    /// later command on whatever buffer this one happened to be visiting.
    #[test]
    fn with_current_buffer_restores_even_when_the_body_fails() {
        let (ctx, env) = editor_with("text");
        let failed = eval_str(
            r#"(progn (buffer-create "other")
                      (with-current-buffer "other" (lambda () (undefined-function-here))))"#,
            &env,
            &ctx,
        );

        assert!(failed.is_err(), "the error must not be swallowed");
        assert_eq!(
            eval_str("(current-buffer)", &env, &ctx).expect("current-buffer"),
            LispExp::string("*scratch*".into())
        );
    }

    #[test]
    fn with_current_buffer_refuses_a_buffer_that_is_not_there() {
        let (ctx, env) = editor_with("text");
        assert!(
            eval_str(
                r#"(with-current-buffer "nope" (lambda () nil))"#,
                &env,
                &ctx
            )
            .is_err()
        );
    }

    /// `set-buffer` semantics, not `switch-to-buffer`: acting on a buffer must
    /// not point the window at it.
    ///
    /// Asked from *inside* the call, because that is the only place the two
    /// differ -- a version that moved the window would move it back on the way
    /// out, and a snapshot taken afterwards would show nothing wrong.
    #[test]
    fn with_current_buffer_does_not_change_what_is_on_screen() {
        let (ctx, env) = editor_with("text");
        let shown = eval_str(
            r#"(progn (buffer-create "other")
                      (with-current-buffer "other" (lambda () (window-buffer))))"#,
            &env,
            &ctx,
        )
        .expect("with-current-buffer");

        assert_eq!(
            shown,
            LispExp::string("*scratch*".into()),
            "the window must still be showing what the user was looking at"
        );
        assert_eq!(
            ctx.snapshot(&env, W, H).views[0].buffer_name,
            "*scratch*",
            "and still be showing it afterwards"
        );
    }

    // -----------------------------------------------------------------------
    // Starting a search
    // -----------------------------------------------------------------------

    #[test]
    fn starting_a_search_opens_a_prompt_in_isearch_mode() {
        let (ctx, env) = editor_with("hello world");
        start_isearch(&ctx, &env);

        assert_eq!(ctx.get_current_buffer_name(), "*Minibuffer*");
        assert_eq!(
            ctx.get_buffer("*Minibuffer*")
                .expect("the prompt")
                .read()
                .unwrap()
                .current_mode,
            "isearch-mode",
            "the prompt must carry the keymap and hook that make it a search"
        );
        assert!(ctx.isearch_active());
    }

    /// The heart of it: the search runs on every keystroke, with no Return.
    #[test]
    fn the_search_runs_as_the_pattern_is_typed() {
        let (ctx, env) = editor_with("alpha beta gamma");
        start_isearch(&ctx, &env);

        type_pattern(&ctx, &env, "b");
        assert_eq!(point(&ctx), 7, "`b' matches the b of beta");

        type_pattern(&ctx, &env, "eta");
        assert_eq!(point(&ctx), 10, "and `beta' matches all of it");
    }

    /// The mistake this design is most exposed to: while the prompt is open the
    /// current buffer is the minibuffer, so anything reaching for "the current
    /// buffer" searches the pattern the user is typing.
    #[test]
    fn an_isearch_searches_the_file_not_the_prompt() {
        let (ctx, env) = editor_with("xyz xyz");
        start_isearch(&ctx, &env);

        // `x` is in the prompt (it is the pattern) and in the file. A search of
        // the prompt would match at its offset 0 and never move the file.
        type_pattern(&ctx, &env, "x");

        assert_eq!(point(&ctx), 1, "the match must be found in *scratch*");
        assert_eq!(
            ctx.get_current_buffer_name(),
            "*Minibuffer*",
            "and the prompt must still be the current buffer while it runs"
        );
    }

    #[test]
    fn the_match_is_selected_so_it_is_highlighted() {
        let (ctx, env) = editor_with("alpha beta");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "beta");

        assert_eq!(mark_of(&ctx), Some((6, true)));
        assert_eq!(point(&ctx), 10);
    }

    /// Backspacing shortens the pattern, and the match must widen back out --
    /// the search is re-run from scratch, not narrowed monotonically.
    #[test]
    fn deleting_a_character_re_searches() {
        let (ctx, env) = editor_with("ab xy abc");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "abc");
        assert_eq!(point(&ctx), 9);

        press(&ctx, &env, KeyCode::Backspace);
        assert_eq!(
            point(&ctx),
            2,
            "with `ab' the first match is the one at the start again"
        );
    }

    /// With nothing to search for there is no match, and leaving point at the
    /// last one would make the search look like it had found something.
    #[test]
    fn emptying_the_pattern_goes_back_to_where_the_search_started() {
        let (ctx, env) = editor_with("hello world");
        goto(&ctx, 3);
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "world");
        assert_eq!(point(&ctx), 11);

        for _ in 0.."world".len() {
            press(&ctx, &env, KeyCode::Backspace);
        }
        assert_eq!(point(&ctx), 3);
    }

    #[test]
    fn the_search_starts_from_point_not_from_the_top() {
        let (ctx, env) = editor_with("one one one");
        goto(&ctx, 5);
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "one");

        assert_eq!(point(&ctx), 11, "the match before point must be skipped");
    }

    // -----------------------------------------------------------------------
    // Repeating, turning around, wrapping
    // -----------------------------------------------------------------------

    #[test]
    fn c_s_goes_to_the_next_match() {
        let (ctx, env) = editor_with("one one one");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "one");
        assert_eq!(point(&ctx), 3);

        press_ctrl(&ctx, &env, 's');
        assert_eq!(point(&ctx), 7);
        press_ctrl(&ctx, &env, 's');
        assert_eq!(point(&ctx), 11);
    }

    /// Turning around must not step over the match on screen: that one is where
    /// the user is looking.
    #[test]
    fn c_r_turns_the_search_around_at_the_current_match() {
        let (ctx, env) = editor_with("one one one");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "one");
        press_ctrl(&ctx, &env, 's');
        assert_eq!(point(&ctx), 7, "at the second match");

        press_ctrl(&ctx, &env, 'r');
        assert_eq!(
            point(&ctx),
            4,
            "backward finds the same match, from its other end"
        );
        press_ctrl(&ctx, &env, 'r');
        assert_eq!(point(&ctx), 0, "and then the one before it");
    }

    #[test]
    fn a_search_that_runs_out_says_so_and_stays_put() {
        let (ctx, env) = editor_with("one two");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "two");
        assert_eq!(point(&ctx), 7);

        press_ctrl(&ctx, &env, 's');
        assert_eq!(point(&ctx), 7, "there is nothing after it");
        assert_eq!(echo(&ctx, &env), "Failing I-search: two");
    }

    /// Emacs' rule, and worth keeping: the "Failing" report is what turns the
    /// second press into a decision rather than an accident.
    #[test]
    fn repeating_a_failing_search_wraps_around() {
        let (ctx, env) = editor_with("two one two");
        goto(&ctx, 4);
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "two");
        assert_eq!(point(&ctx), 11, "the one after point");

        press_ctrl(&ctx, &env, 's');
        assert_eq!(echo(&ctx, &env), "Failing I-search: two");
        assert_eq!(point(&ctx), 11, "the first press does not move");

        press_ctrl(&ctx, &env, 's');
        assert_eq!(point(&ctx), 3, "the second wraps to the one at the top");
        assert_eq!(echo(&ctx, &env), "Wrapped I-search: two");
    }

    #[test]
    fn a_pattern_that_matches_nothing_at_all_reports_without_moving() {
        let (ctx, env) = editor_with("hello");
        goto(&ctx, 2);
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "zzz");

        assert_eq!(point(&ctx), 2);
        assert_eq!(echo(&ctx, &env), "Failing I-search: zzz");
    }

    // -----------------------------------------------------------------------
    // Ending one
    // -----------------------------------------------------------------------

    #[test]
    fn return_stops_the_search_and_leaves_point_at_the_match() {
        let (ctx, env) = editor_with("alpha beta");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "beta");
        press(&ctx, &env, KeyCode::Enter);

        assert_eq!(point(&ctx), 10);
        assert!(!ctx.isearch_active(), "the session is over");
        assert_ne!(
            ctx.get_current_buffer_name(),
            "*Minibuffer*",
            "and the prompt is closed"
        );
        assert_eq!(
            mark_of(&ctx).map(|m| m.1),
            Some(false),
            "the match stops being a selection the next command would act on"
        );
    }

    /// The reason the origin is recorded at all: a search moves point as you
    /// type, so abandoning one has to undo that.
    #[test]
    fn escape_puts_point_back_where_the_search_started() {
        let (ctx, env) = editor_with("alpha beta gamma");
        goto(&ctx, 4);
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "gamma");
        assert_eq!(point(&ctx), 16, "moved while searching");

        press(&ctx, &env, KeyCode::Esc);

        assert_eq!(point(&ctx), 4, "and put back on abandoning it");
        assert!(!ctx.isearch_active());
        assert_eq!(echo(&ctx, &env), "Quit");
    }

    /// `C-g` is bound globally to `keyboard-quit`, which knows nothing about
    /// prompts -- so the search binds it itself.
    #[test]
    fn c_g_abandons_the_search_like_escape() {
        let (ctx, env) = editor_with("alpha beta");
        goto(&ctx, 2);
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "beta");

        press_ctrl(&ctx, &env, 'g');

        assert_eq!(point(&ctx), 2);
        assert!(!ctx.isearch_active());
        assert_ne!(ctx.get_current_buffer_name(), "*Minibuffer*");
    }

    /// Closing the prompt must leave nothing behind that would make the *next*
    /// prompt behave like a search.
    #[test]
    fn the_hook_stops_firing_once_the_search_is_over() {
        let (ctx, env) = editor_with("alpha beta");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "beta");
        press(&ctx, &env, KeyCode::Enter);
        let settled = point(&ctx);

        // An ordinary prompt, and ordinary typing into it.
        eval_str(r#"(minibuffer-read "P:" nil nil nil)"#, &env, &ctx).expect("a plain prompt");
        type_pattern(&ctx, &env, "alpha");

        assert_eq!(
            point(&ctx),
            settled,
            "typing into a plain prompt must not move the buffer"
        );
    }

    /// Two things at once, and both are easy to get wrong: the search has to
    /// remember which buffer it started in -- read *before* the prompt opens,
    /// or it would record the minibuffer -- and closing the prompt has to put
    /// that buffer back, which is `minibuffer-cleanup`'s job and the reason
    /// `isearch-mode` registers it.
    #[test]
    fn a_search_in_another_buffer_stays_in_that_buffer() {
        let (ctx, env) = editor_with("scratch text");
        eval_str(
            r#"(progn (buffer-create "notes") (switch-to-buffer "notes"))"#,
            &env,
            &ctx,
        )
        .expect("open a second buffer");
        ctx.mutate_buffer(ctx.get_buffer("notes").expect("notes"), |b| {
            b.text = GapBuffer::from("find the needle here");
            b.text.cursor_move(0, 0);
        });

        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "needle");
        press(&ctx, &env, KeyCode::Enter);

        assert_eq!(
            ctx.get_current_buffer_name(),
            "notes",
            "the prompt must hand the buffer back, not drop you in *scratch*"
        );
        assert_eq!(
            ctx.get_buffer("notes")
                .expect("notes")
                .read()
                .unwrap()
                .text
                .cursor_pos_1d(),
            15,
            "and the match must have been found in it"
        );
    }

    /// Repeating steps one character on rather than past the whole match, so
    /// overlapping matches are all reachable -- resuming at the match's end
    /// would step over the one at offset 1.
    #[test]
    fn repeating_finds_overlapping_matches() {
        let (ctx, env) = editor_with("aaa");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "aa");
        assert_eq!(point(&ctx), 2);

        press_ctrl(&ctx, &env, 's');
        assert_eq!(point(&ctx), 3, "the match at 1..3 overlaps the first one");
    }

    /// The same rule going the other way, so a backward search is as thorough
    /// as a forward one rather than skipping whatever it is already sitting on.
    #[test]
    fn repeating_backward_finds_overlapping_matches_too() {
        let (ctx, env) = editor_with("aaa");
        goto(&ctx, 3);
        eval_str("(isearch-backward)", &env, &ctx).expect("start");
        type_pattern(&ctx, &env, "aa");
        assert_eq!(point(&ctx), 1, "the last match, from its start");

        press_ctrl(&ctx, &env, 'r');
        assert_eq!(point(&ctx), 0, "the match at 0..2 overlaps it");
    }

    // -----------------------------------------------------------------------
    // Options
    // -----------------------------------------------------------------------

    #[test]
    fn case_is_ignored_unless_case_fold_search_is_nil() {
        let (ctx, env) = editor_with("Hello hello");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "hello");
        assert_eq!(point(&ctx), 5, "the capitalised one matches by default");
        press(&ctx, &env, KeyCode::Esc);

        eval_str("(setq case-fold-search nil)", &env, &ctx).expect("setq");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "hello");
        assert_eq!(point(&ctx), 11, "with folding off, only the exact one");
    }

    #[test]
    fn a_regexp_search_takes_a_regular_expression() {
        let (ctx, env) = editor_with("x = 41\ny = 42\n");
        eval_str("(isearch-forward-regexp)", &env, &ctx).expect("start");
        type_pattern(&ctx, &env, "[0-9]+");

        assert_eq!(point(&ctx), 6);
        assert_eq!(echo(&ctx, &env), "Regexp I-search: [0-9]+");
    }

    /// Half a regexp is what every complete one looks like while it is being
    /// typed, so an invalid one is reported, not signalled.
    #[test]
    fn an_incomplete_regexp_is_reported_rather_than_signalled() {
        let (ctx, env) = editor_with("abc");
        eval_str("(isearch-forward-regexp)", &env, &ctx).expect("start");
        type_pattern(&ctx, &env, "[a-");

        assert_eq!(echo(&ctx, &env), "Failing Regexp I-search: [a-");
        assert!(ctx.isearch_active(), "and the search carries on");

        type_pattern(&ctx, &env, "c]");
        assert_eq!(point(&ctx), 1, "finishing it makes it match");
    }

    #[test]
    fn searching_backward_finds_the_match_before_point() {
        let (ctx, env) = editor_with("one two one");
        goto(&ctx, 11);
        eval_str("(isearch-backward)", &env, &ctx).expect("start");
        type_pattern(&ctx, &env, "one");

        assert_eq!(point(&ctx), 8, "point at the start, going backwards");
        assert_eq!(echo(&ctx, &env), "I-search backward: one");
    }

    /// Offsets stay counted in characters across multi-byte text. Searching
    /// *for* a multi-byte pattern is not tested here because it cannot be
    /// typed: `fill_default_keymaps` binds self-insert for ASCII only, so no
    /// prompt in the editor can receive a character outside it.
    #[test]
    fn searching_past_multi_byte_text_lands_on_the_right_character() {
        let (ctx, env) = editor_with("naïve café naïve");
        start_isearch(&ctx, &env);
        type_pattern(&ctx, &env, "ve");

        assert_eq!(
            point(&ctx),
            5,
            "one multi-byte character before it: bytes would say 6"
        );

        press_ctrl(&ctx, &env, 's');
        assert_eq!(
            point(&ctx),
            16,
            "three before it: bytes would land past the end of the text"
        );
    }
}
