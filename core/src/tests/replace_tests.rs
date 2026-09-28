//! Replace: what gets changed, and how a view drives it.
//!
//! # What is worth testing here
//!
//! A replace is a loop over positions that the loop itself keeps moving. Every
//! interesting failure is in that arithmetic and every one of them is quiet:
//!
//! - A resume point that does not advance past an empty match, and the command
//!   never returns.
//! - A resume point past the *match* rather than past the replacement, and a
//!   replacement containing what it replaced is replaced again, forever.
//! - A region limit that is not moved when the text under it grows, and a
//!   replace confined to the region creeps past its end.
//!
//! None of those show up in a test that replaces `cat` with `dog` once. So the
//! first half below is about the shape of the loop, and the second is about
//! the split the whole feature is built on: the controller decides what
//! changes, the view only decides how you are asked, and the controller has to
//! be drivable with no view at all.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::search::Casing;
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

    fn try_run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        try_run(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn text(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|buf| buf.text.to_string())
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

    /// The question the built-in view is showing, or empty.
    fn question(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
        ctx.snapshot(env, 80, 24).prompt
    }

    /// The spans the current buffer has marked, by face.
    fn marked(ctx: &Ctx) -> Vec<(usize, usize)> {
        ctx.with_current_buffer(|buf| {
            buf.overlays
                .iter()
                .filter(|o| o.face == crate::ui::Face::REPLACE_MATCH)
                .map(|o| (o.start, o.end))
                .collect()
        })
    }

    // ----------------------------------------------------------------
    // Casing, on its own
    // ----------------------------------------------------------------

    #[test]
    fn casing_is_only_read_from_text_that_agrees_with_itself() {
        assert_eq!(Casing::of("FOO"), Casing::Upper);
        assert_eq!(Casing::of("Foo"), Casing::Capitalised);
        assert_eq!(Casing::of("foo"), Casing::AsWritten);
        // Somebody's identifier. Re-casing it would be vandalism.
        assert_eq!(Casing::of("fooBar"), Casing::AsWritten);
        assert_eq!(Casing::of("FooBar"), Casing::AsWritten);
        // No letters to have a case, which is most matches in a regexp
        // replace.
        assert_eq!(Casing::of("123"), Casing::AsWritten);
        assert_eq!(Casing::of(""), Casing::AsWritten);
    }

    #[test]
    fn a_single_capital_reads_as_capitalised_rather_than_shouted() {
        // `I` is one letter and upper. Reading it as `Upper` and reading it as
        // `Capitalised` give the same answer for a one-letter replacement and
        // different ones for a longer, so it has to be decided rather than
        // left to fall out.
        assert_eq!(Casing::of("I"), Casing::Capitalised);
        assert_eq!(Casing::Capitalised.apply("one"), "One");
    }

    // ----------------------------------------------------------------
    // The loop's arithmetic
    // ----------------------------------------------------------------

    #[test]
    fn every_occurrence_is_replaced() {
        let (ctx, env) = editor("cat cat cat");
        assert_eq!(
            run(r#"(replace-string "cat" "dog")"#, &env, &ctx),
            LispExp::Number(3.0)
        );
        assert_eq!(text(&ctx), "dog dog dog");
    }

    #[test]
    fn a_replacement_containing_what_it_replaced_is_not_replaced_again() {
        // The resume point goes past the *replacement*, not past the match.
        // Past the match, `cat` -> `cat cat` finds the copy it just wrote and
        // never stops.
        let (ctx, env) = editor("cat");
        assert_eq!(
            run(r#"(replace-string "cat" "cat cat")"#, &env, &ctx),
            LispExp::Number(1.0)
        );
        assert_eq!(text(&ctx), "cat cat");
    }

    #[test]
    fn a_pattern_that_can_match_nothing_still_terminates() {
        // `x*` matches the empty string at every position. Without the
        // resume point being forced forward, this is an editor that never
        // comes back.
        let (ctx, env) = editor("abc");
        run(r#"(replace-regexp "x*" "-")"#, &env, &ctx);
        assert!(
            text(&ctx).len() < 40,
            "it terminated instead of growing forever: {:?}",
            text(&ctx)
        );
    }

    #[test]
    fn skipping_an_empty_match_moves_on_from_it() {
        // Declining has to force the scan forward for the same reason
        // accepting does, and it is a separate piece of arithmetic: an empty
        // match that is skipped and then found again at the same place is a
        // question that can be answered forever without ever being finished.
        let (ctx, env) = editor("ab");
        run(r#"(query-replace-regexp "x*" "-")"#, &env, &ctx);
        let offered = |ctx: &Ctx, env: &Arc<Env<Ctx>>| match run("(replace-match)", env, ctx) {
            LispExp::Cons(cell) => match &cell.car {
                LispExp::Number(start) => *start as usize,
                other => panic!("a match should start at a number: {other:?}"),
            },
            other => panic!("there should be a match: {other:?}"),
        };
        assert_eq!(offered(&ctx, &env), 0);
        run("(replace-skip)", &env, &ctx);
        assert_eq!(offered(&ctx, &env), 1);
        run("(replace-skip)", &env, &ctx);
        assert_eq!(offered(&ctx, &env), 2);
        run("(replace-abandon)", &env, &ctx);
        assert_eq!(text(&ctx), "ab");
    }

    #[test]
    fn a_shorter_replacement_does_not_skip_what_follows() {
        let (ctx, env) = editor("aaa");
        assert_eq!(
            run(r#"(replace-string "a" "")"#, &env, &ctx),
            LispExp::Number(3.0)
        );
        assert_eq!(text(&ctx), "");
    }

    #[test]
    fn replacing_starts_at_point_and_leaves_what_is_behind_it_alone() {
        let (ctx, env) = editor("cat cat");
        ctx.with_current_buffer_mut(|buf| buf.text.cursor_move(0, 4));
        run(r#"(replace-string "cat" "dog")"#, &env, &ctx);
        assert_eq!(text(&ctx), "cat dog");
    }

    // ----------------------------------------------------------------
    // The region
    // ----------------------------------------------------------------

    #[test]
    fn an_active_region_confines_the_replace_to_itself() {
        let (ctx, env) = editor("cat cat cat");
        ctx.with_current_buffer_mut(|buf| {
            buf.mark = Some(crate::buffer::Mark::new(0));
            buf.text.cursor_move(0, 7);
        });
        assert_eq!(
            run(r#"(replace-string "cat" "dog")"#, &env, &ctx),
            LispExp::Number(2.0)
        );
        assert_eq!(text(&ctx), "dog dog cat");
    }

    #[test]
    fn a_growing_replacement_does_not_let_the_region_creep_past_its_end() {
        // The limit has to move with the text under it. Left where it was, a
        // replacement longer than what it replaced pushes the third `cat`
        // inside the region and it gets replaced too.
        let (ctx, env) = editor("cat cat cat");
        ctx.with_current_buffer_mut(|buf| {
            buf.mark = Some(crate::buffer::Mark::new(0));
            buf.text.cursor_move(0, 7);
        });
        assert_eq!(
            run(r#"(replace-string "cat" "weasel")"#, &env, &ctx),
            LispExp::Number(2.0),
            "exactly the two that were in the region"
        );
        assert_eq!(text(&ctx), "weasel weasel cat");
    }

    // ----------------------------------------------------------------
    // Groups and casing, through the whole thing
    // ----------------------------------------------------------------

    #[test]
    fn groups_are_filled_in_per_match() {
        let (ctx, env) = editor("Smith, John");
        run(r#"(replace-regexp "(\\w+), (\\w+)" "\\2 \\1")"#, &env, &ctx);
        assert_eq!(text(&ctx), "John Smith");
    }

    #[test]
    fn a_group_reference_under_a_literal_pattern_expands_to_nothing() {
        // A literal pattern has no groups. Expanding to *something* would
        // mean inventing it.
        let (ctx, env) = editor("ab");
        run(r#"(replace-string "ab" "[\\1]")"#, &env, &ctx);
        assert_eq!(text(&ctx), "[]");
    }

    #[test]
    fn the_whole_match_is_available_under_a_literal_pattern_too() {
        // `\&` is taken from the text rather than from group zero, which a
        // literal match does not record. Taking it from the group would have
        // made this work under a regexp and silently do nothing here.
        let (ctx, env) = editor("cat");
        run(r#"(replace-string "cat" "<\\&>")"#, &env, &ctx);
        assert_eq!(text(&ctx), "<cat>");
    }

    #[test]
    fn a_replacement_takes_the_casing_of_what_it_replaced() {
        // The thing people most notice missing: replacing `colour` with
        // `color` must not turn `Colour` at the start of a sentence into
        // `color`.
        let (ctx, env) = editor("colour Colour COLOUR");
        run("(setq case-fold-search t)", &env, &ctx);
        run(r#"(replace-string "colour" "color")"#, &env, &ctx);
        assert_eq!(text(&ctx), "color Color COLOR");
    }

    #[test]
    fn case_replace_nil_puts_the_replacement_in_exactly_as_written() {
        let (ctx, env) = editor("Colour");
        run(
            "(setq case-fold-search t) (setq case-replace nil)",
            &env,
            &ctx,
        );
        run(r#"(replace-string "colour" "color")"#, &env, &ctx);
        assert_eq!(text(&ctx), "color");
    }

    #[test]
    fn the_search_follows_case_fold_search_like_every_other_search() {
        // `case-fold-search' is the editor's one answer to whether searching
        // ignores case, and the incremental search already reads it. A replace
        // that answered it differently would be the surprise worth avoiding,
        // not the folding itself -- so both directions are pinned here.
        let (ctx, env) = editor("cat Cat");
        run("(setq case-fold-search nil)", &env, &ctx);
        assert_eq!(
            run(r#"(replace-string "cat" "dog")"#, &env, &ctx),
            LispExp::Number(1.0)
        );
        assert_eq!(text(&ctx), "dog Cat");

        let (ctx, env) = editor("cat Cat");
        run("(setq case-fold-search t)", &env, &ctx);
        assert_eq!(
            run(r#"(replace-string "cat" "dog")"#, &env, &ctx),
            LispExp::Number(2.0)
        );
        // The second one keeps its capital: that is `case-replace', which is
        // the whole reason folding here is not destructive.
        assert_eq!(text(&ctx), "dog Dog");
    }

    // ----------------------------------------------------------------
    // The controller, driven with no view at all
    // ----------------------------------------------------------------

    #[test]
    fn the_verbs_drive_a_replace_without_anything_asking() {
        // The whole point of the split: a view is optional. Anything that can
        // call these -- a script, a window with buttons, a test -- drives a
        // replace without a single key being pressed.
        let (ctx, env) = editor("cat cat cat cat");
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        run("(replace-this)", &env, &ctx);
        run("(replace-skip)", &env, &ctx);
        run("(replace-this)", &env, &ctx);
        // Still running: there is a fourth match to be offered, so the count
        // is readable mid-session and `replace-done' has something to stop.
        assert_eq!(run("(replace-count)", &env, &ctx), LispExp::Number(2.0));
        assert_eq!(run("(replace-done)", &env, &ctx), LispExp::Number(2.0));
        assert_eq!(text(&ctx), "dog cat dog cat");
    }

    #[test]
    fn the_session_finishes_itself_when_the_last_match_is_answered() {
        // Answering the last one leaves nothing to ask about, so the session
        // ends there rather than sitting open with no question in it -- which
        // is why the count comes back from the verb and not from a later
        // `replace-count'.
        let (ctx, env) = editor("cat cat");
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        run("(replace-this)", &env, &ctx);
        assert_eq!(run("(replace-this)", &env, &ctx), LispExp::Number(2.0));
        assert_eq!(run("(replace-count)", &env, &ctx), LispExp::Number(0.0));
        assert_eq!(text(&ctx), "dog dog");
    }

    #[test]
    fn replace_match_says_what_is_offered_and_what_it_would_become() {
        // What a view needs and the whole of what it needs: it should not
        // have to work out the expansion, or it would work it out differently
        // from the thing that will actually do it.
        let (ctx, env) = editor("Colour");
        run("(setq case-fold-search t)", &env, &ctx);
        run(r#"(query-replace "colour" "color")"#, &env, &ctx);
        assert_eq!(
            run("(replace-match)", &env, &ctx),
            run(r#"'(0 6 "Colour" "Color")"#, &env, &ctx)
        );
    }

    #[test]
    fn the_offered_match_is_marked_so_it_can_be_seen() {
        let (ctx, env) = editor("one cat two");
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        assert_eq!(marked(&ctx), vec![(4, 7)]);
        assert_eq!(
            ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()),
            4,
            "and point is on it"
        );
    }

    #[test]
    fn finishing_clears_the_mark() {
        // Left behind, it would sit there highlighting text nothing is doing
        // anything to.
        let (ctx, env) = editor("cat");
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        run("(replace-done)", &env, &ctx);
        assert!(marked(&ctx).is_empty());
    }

    #[test]
    fn replace_rest_finishes_the_session() {
        let (ctx, env) = editor("cat cat cat");
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        run("(replace-skip)", &env, &ctx);
        assert_eq!(run("(replace-rest)", &env, &ctx), LispExp::Number(2.0));
        assert_eq!(text(&ctx), "cat dog dog");
        assert!(marked(&ctx).is_empty(), "and it is over");
    }

    #[test]
    fn stepping_back_puts_the_text_and_the_scan_back_together() {
        // Two halves that have to agree. Undoing alone leaves the scan past a
        // match that is there again; rewinding the scan alone offers a match
        // that has already been changed.
        let (ctx, env) = editor("cat cat");
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        run("(replace-this)", &env, &ctx);
        assert_eq!(text(&ctx), "dog cat");
        assert_eq!(run("(replace-count)", &env, &ctx), LispExp::Number(1.0));

        run("(replace-back)", &env, &ctx);
        assert_eq!(text(&ctx), "cat cat", "the text is back");
        assert_eq!(run("(replace-count)", &env, &ctx), LispExp::Number(0.0));
        assert_eq!(
            run("(replace-match)", &env, &ctx),
            run(r#"'(0 3 "cat" "dog")"#, &env, &ctx),
            "and the first one is offered again"
        );
    }

    #[test]
    fn there_is_nothing_to_step_back_to_at_the_start() {
        let (ctx, env) = editor("cat");
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        assert!(run("(replace-back)", &env, &ctx).is_nil());
        assert_eq!(text(&ctx), "cat");
    }

    #[test]
    fn abandoning_puts_point_back_and_keeps_what_was_replaced() {
        // What has been written is written; `undo' is the way to take that
        // back. All this undoes is being in the middle of a replace.
        let (ctx, env) = editor("aaa cat cat");
        ctx.with_current_buffer_mut(|buf| buf.text.cursor_move(0, 4));
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        run("(replace-this)", &env, &ctx);
        run("(replace-abandon)", &env, &ctx);
        assert_eq!(text(&ctx), "aaa dog cat");
        assert_eq!(ctx.with_current_buffer(|buf| buf.text.cursor_pos_1d()), 4);
    }

    #[test]
    fn a_replace_with_no_match_says_so_and_starts_nothing() {
        let (ctx, env) = editor("cat");
        assert!(run(r#"(query-replace "weasel" "dog")"#, &env, &ctx).is_nil());
        assert!(run("(replace-match)", &env, &ctx).is_nil());
        assert!(ctx.get_echo_message().contains("No match"));
    }

    #[test]
    fn a_bad_regexp_is_reported_rather_than_starting_a_session() {
        let (ctx, env) = editor("cat");
        assert!(run(r#"(query-replace-regexp "(" "x")"#, &env, &ctx).is_nil());
        assert!(run("(replace-match)", &env, &ctx).is_nil());
        assert_eq!(text(&ctx), "cat");
    }

    // ----------------------------------------------------------------
    // The built-in view
    // ----------------------------------------------------------------

    #[test]
    fn the_built_in_view_answers_with_single_keys() {
        let (ctx, env) = editor("cat cat cat");
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        assert!(
            question(&ctx, &env).contains("Replace"),
            "it is asking: {:?}",
            question(&ctx, &env)
        );
        press('y', &ctx, &env);
        press('n', &ctx, &env);
        press('y', &ctx, &env);
        assert_eq!(text(&ctx), "dog cat dog");
        assert_eq!(question(&ctx, &env), "", "and it is over");
    }

    #[test]
    fn any_other_key_ends_the_replace_and_then_does_its_own_job() {
        // `OnUnbound::Release`: the convention every editor with this feature
        // shares, and the one thing about it nobody has to be taught.
        let (ctx, env) = editor("cat cat");
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        press('z', &ctx, &env);
        assert_eq!(question(&ctx, &env), "", "the replace is over");
        assert_eq!(text(&ctx), "zcat cat", "and the key was typed");
    }

    #[test]
    fn a_view_named_by_the_variable_is_used_instead_of_the_built_in_one() {
        // The split the whole feature is built on. Nothing below the variable
        // knows this happened.
        let (ctx, env) = editor("cat cat");
        run(
            r#"(setq asked 0)
               (setq *replace-read-function*
                     (lambda () (setq asked (+ asked 1)) (replace-this)))"#,
            &env,
            &ctx,
        );
        run(r#"(query-replace "cat" "dog")"#, &env, &ctx);
        assert_eq!(text(&ctx), "dog dog", "the view drove it to the end");
        assert_eq!(
            question(&ctx, &env),
            "",
            "and the built-in keys were never installed"
        );
        assert_eq!(run("asked", &env, &ctx), LispExp::Number(2.0));
    }

    #[test]
    fn the_commands_are_registered_so_they_can_be_run_and_rebound() {
        let (ctx, env) = editor("");
        for name in [
            "query-replace",
            "query-replace-regexp",
            "replace-string",
            "replace-regexp",
        ] {
            assert_eq!(
                run(&format!("(commandp '{name})"), &env, &ctx),
                LispExp::t(),
                "{name}"
            );
        }
    }
}
