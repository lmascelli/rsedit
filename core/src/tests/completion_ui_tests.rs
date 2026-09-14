//! `core/lisp/completion.lisp`, and the three mechanisms it is built out of:
//! a split that holds a size, a keymap that is a question, and the seam in
//! `minibuffer-complete` that lets a module take over presenting candidates.
//!
//! # The one thing these must protect above all
//!
//! That the editor works *without* this module. It is optional, and "optional"
//! is a claim that rots the moment something in the base system quietly starts
//! depending on it. So the first block below loads no module at all and checks
//! that Tab still cycles -- if that ever fails, the module has stopped being a
//! module.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 60;
    const H: usize = 24;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// The editor with the standard Lisp layers but *without* the completion
    /// module, which is how it runs for anyone who has not loaded it.
    fn plain() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/minibuffer.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// The same, with the module loaded.
    fn with_module() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = plain();
        eval_str(include_str!("../../lisp/completion.lisp"), &env, &ctx)
            .expect("loading completion.lisp");
        (ctx, env)
    }

    fn text_of(exp: &LispExp<Ctx>) -> String {
        match exp {
            LispExp::String(s) => (**s).clone(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    fn windows(ctx: &Ctx) -> usize {
        ctx.window_count()
    }

    fn strip(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text_of(&run(
            r#"(with-current-buffer "*Completions*" (lambda () (buffer-string)))"#,
            env,
            ctx,
        ))
    }

    /// What the selection covers, as the region in the completions buffer --
    /// which is exactly what the renderer draws highlighted.
    fn selected(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text_of(&run(
            r#"(with-current-buffer "*Completions*"
                 (lambda () (substring (buffer-string) (region-beginning) (region-end))))"#,
            env,
            ctx,
        ))
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

    /// Open a prompt whose Tab offers NAMES.
    fn prompt_offering(names: &[&str], env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        let list = names
            .iter()
            .map(|name| format!(r#""{name}""#))
            .collect::<Vec<_>>()
            .join(" ");
        run(
            &format!(
                r#"(progn (setq *test-answer* nil)
                          (minibuffer-read "Pick:"
                            (lambda (input) (setq *test-answer* input))
                            (lambda (input) (list {list}))
                            nil))"#
            ),
            env,
            ctx,
        );
    }

    fn minibuffer_text(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text_of(&run("(buffer-string)", env, ctx))
    }

    // ---------------- the editor without the module ----------------

    /// The claim the whole design rests on: nothing in the base system reaches
    /// for this module, so an editor that never loads it behaves as it always
    /// did.
    #[test]
    fn without_the_module_tab_still_cycles_in_place() {
        let (ctx, env) = plain();
        prompt_offering(&["alpha", "beta"], &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);
        assert_eq!(minibuffer_text(&env, &ctx), "alpha");
        press(&ctx, &env, KeyCode::Tab);
        assert_eq!(minibuffer_text(&env, &ctx), "beta");
        press(&ctx, &env, KeyCode::Tab);
        assert_eq!(minibuffer_text(&env, &ctx), "alpha", "and wraps round");
    }

    #[test]
    fn without_the_module_no_window_is_opened_by_completing() {
        let (ctx, env) = plain();
        let before = windows(&ctx);
        prompt_offering(&["alpha", "beta"], &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(windows(&ctx), before);
        assert!(ctx.get_buffer("*Completions*").is_none());
    }

    /// And setting the variable back is how it is turned off again -- the
    /// module is not something the editor has to be rebuilt without.
    #[test]
    fn clearing_the_variable_gives_the_cycling_back() {
        let (ctx, env) = with_module();
        run("(setq *completion-read-function* nil)", &env, &ctx);
        prompt_offering(&["alpha", "beta"], &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(minibuffer_text(&env, &ctx), "alpha");
        assert_eq!(windows(&ctx), 1, "no strip");
    }

    // ---------------- the strip ----------------

    #[test]
    fn completing_with_the_module_opens_a_strip_instead_of_cycling() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta", "gamma"], &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(windows(&ctx), 2, "the strip is a window of its own");
        assert!(strip(&env, &ctx).contains("alpha"));
        assert!(strip(&env, &ctx).contains("gamma"));
        assert_eq!(
            minibuffer_text(&env, &ctx),
            "",
            "and nothing was put in the prompt yet"
        );
    }

    /// The property everything else depends on. The strip is shown *while* the
    /// prompt is being filled in, so focus staying put is what keeps the next
    /// keystroke going to the prompt.
    #[test]
    fn the_strip_does_not_take_focus() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        let focused = ctx.get_focused_window_id();

        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(ctx.get_focused_window_id(), focused);
        assert_eq!(ctx.get_current_buffer_name(), "*Minibuffer*");
    }

    #[test]
    fn the_strip_is_exactly_as_tall_as_the_variable_says() {
        let (ctx, env) = with_module();
        run("(setq completion-window-height 4)", &env, &ctx);
        prompt_offering(&["alpha", "beta"], &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);

        let frame = ctx.snapshot(&env, W, H);
        let strip = frame
            .views
            .iter()
            .find(|view| view.buffer_name == "*Completions*")
            .expect("the strip is on screen");
        assert_eq!(strip.rect.height, 4);
        assert_eq!(strip.rect.width, W, "and spans the frame");
    }

    /// Four rows means four rows of candidates. A status line would spend one
    /// of them saying the name of a buffer nobody is editing.
    #[test]
    fn the_strip_has_no_status_line() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        let frame = ctx.snapshot(&env, W, H);
        let strip = frame
            .views
            .iter()
            .find(|view| view.buffer_name == "*Completions*")
            .expect("the strip is on screen");
        assert_eq!(strip.mode_line, None);
        assert_eq!(strip.lines.len(), strip.rect.height);
    }

    /// The strip takes its space from the windows above rather than painting
    /// over them, which is what makes it a split and not a float.
    #[test]
    fn the_windows_above_give_up_the_space() {
        let (ctx, env) = with_module();
        let before = ctx.snapshot(&env, W, H).views[0].rect.height;
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        run("(setq completion-window-height 5)", &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);

        let after = ctx.snapshot(&env, W, H).views[0].rect.height;
        assert_eq!(after, before - 5);
    }

    #[test]
    fn candidates_are_laid_out_in_the_number_of_columns_asked_for() {
        let (ctx, env) = with_module();
        run("(setq completion-columns 2)", &env, &ctx);
        prompt_offering(&["alpha", "beta", "gamma"], &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);

        let shown = strip(&env, &ctx);
        let first = shown.lines().next().expect("a first row");
        assert!(first.starts_with("alpha"), "{first:?}");
        assert!(first.contains("beta"), "two to a row: {first:?}");
        assert!(!first.contains("gamma"), "and no more than two: {first:?}");
        assert!(
            shown
                .lines()
                .nth(1)
                .expect("a second row")
                .contains("gamma"),
            "{shown:?}"
        );
    }

    #[test]
    fn a_description_is_shown_beside_its_value() {
        let (ctx, env) = with_module();
        run(
            r#"(completion--present (list (cons "point" "a position in a buffer"))
                                    (lambda (v) (setq *test-answer* v)))"#,
            &env,
            &ctx,
        );
        // One candidate is taken without asking, so present a second.
        run(
            r#"(completion--present (list (cons "point" "a position") "mark")
                                    (lambda (v) (setq *test-answer* v)))"#,
            &env,
            &ctx,
        );

        assert!(strip(&env, &ctx).contains("point a position"));
    }

    // ---------------- moving and choosing ----------------

    #[test]
    fn the_first_candidate_is_selected_when_the_strip_opens() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(selected(&env, &ctx), "alpha");
    }

    #[test]
    fn n_and_p_walk_the_candidates_and_wrap_round() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta", "gamma"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        press(&ctx, &env, KeyCode::Char('n'));
        assert_eq!(selected(&env, &ctx), "beta");
        press(&ctx, &env, KeyCode::Char('n'));
        assert_eq!(selected(&env, &ctx), "gamma");
        press(&ctx, &env, KeyCode::Char('n'));
        assert_eq!(selected(&env, &ctx), "alpha", "past the end is the start");
        press(&ctx, &env, KeyCode::Char('p'));
        assert_eq!(selected(&env, &ctx), "gamma", "and back the other way");
    }

    /// Only the value is highlighted, not the description beside it: the
    /// highlight says what pressing Return would give you.
    #[test]
    fn the_highlight_covers_the_value_and_not_its_description() {
        let (ctx, env) = with_module();
        run(
            r#"(completion--present (list (cons "point" "a position") "mark")
                                    (lambda (v) (setq *test-answer* v)))"#,
            &env,
            &ctx,
        );

        assert_eq!(selected(&env, &ctx), "point");
    }

    #[test]
    fn return_takes_the_selected_candidate_into_the_prompt() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta", "gamma"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);
        press(&ctx, &env, KeyCode::Char('n'));

        press(&ctx, &env, KeyCode::Enter);

        assert_eq!(minibuffer_text(&env, &ctx), "beta");
        assert_eq!(windows(&ctx), 1, "and the strip is gone");
    }

    /// Return chooses a candidate; it does not also confirm the prompt. The
    /// user may still edit what was completed, and it takes a second Return to
    /// finish -- which is what makes completing a step rather than a decision.
    #[test]
    fn return_chooses_a_candidate_without_confirming_the_prompt() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        press(&ctx, &env, KeyCode::Enter);

        assert_eq!(run("*test-answer*", &env, &ctx), LispExp::nil());
        assert_eq!(ctx.get_current_buffer_name(), "*Minibuffer*");

        press(&ctx, &env, KeyCode::Enter);
        assert_eq!(
            run("*test-answer*", &env, &ctx),
            LispExp::string("alpha".into()),
            "the second Return is the one that answers"
        );
    }

    #[test]
    fn escape_puts_the_strip_away_and_leaves_the_prompt_alone() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        run(r#"(self-insert "x")"#, &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        press(&ctx, &env, KeyCode::Esc);

        assert_eq!(windows(&ctx), 1);
        assert_eq!(
            minibuffer_text(&env, &ctx),
            "x",
            "what was typed is still there"
        );
        assert_eq!(
            ctx.get_current_buffer_name(),
            "*Minibuffer*",
            "and the prompt is still open"
        );
    }

    /// Escape while the strip is up dismisses the strip, not the prompt. The
    /// transient map is consulted first, which is what puts it in front of the
    /// minibuffer's own Escape.
    #[test]
    fn escape_does_not_reach_the_prompt_underneath() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        press(&ctx, &env, KeyCode::Esc);
        assert!(ctx.get_buffer("*Minibuffer*").is_some(), "still prompting");

        press(&ctx, &env, KeyCode::Esc);
        assert!(
            ctx.get_buffer("*Minibuffer*").is_none(),
            "the second Escape is the one the prompt sees"
        );
    }

    /// A modal map has to swallow what it does not bind, or a half-made choice
    /// could be walked away from by pressing something unrelated -- leaving a
    /// strip on screen that nothing is listening to.
    #[test]
    fn a_key_the_strip_does_not_bind_does_nothing_at_all() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        press(&ctx, &env, KeyCode::Char('z'));

        assert_eq!(windows(&ctx), 2, "the strip is still up");
        assert_eq!(minibuffer_text(&env, &ctx), "", "and z was not typed");
    }

    #[test]
    fn c_g_abandons_the_strip_like_escape() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('g'),
                modifiers: KeyModifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
            &env,
        );

        assert_eq!(windows(&ctx), 1);
        assert!(ctx.get_buffer("*Minibuffer*").is_some());
    }

    // ---------------- the cases that are not a choice ----------------

    /// One candidate is not a question. Opening a window to ask it reads as the
    /// editor not knowing what it is doing.
    #[test]
    fn a_single_candidate_is_taken_without_opening_anything() {
        let (ctx, env) = with_module();
        prompt_offering(&["only-one"], &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(windows(&ctx), 1);
        assert_eq!(minibuffer_text(&env, &ctx), "only-one");
    }

    #[test]
    fn no_candidates_says_so_rather_than_opening_an_empty_strip() {
        let (ctx, env) = with_module();
        prompt_offering(&[], &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(windows(&ctx), 1);
        assert_eq!(ctx.get_echo_message(), "No completions");
    }

    // ---------------- more candidates than fit ----------------

    /// The strip is never focused, and only a focused window scrolls to follow
    /// its cursor -- so the strip is re-rendered around the selection instead.
    /// Walking past the bottom has to bring the next page into view.
    #[test]
    fn walking_past_the_end_of_a_page_shows_the_next_one() {
        let (ctx, env) = with_module();
        run(
            "(setq completion-window-height 2)(setq completion-columns 2)",
            &env,
            &ctx,
        );
        // Four to a page, so the fifth is on the second page.
        let names: Vec<String> = (0..6).map(|n| format!("item{n}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        prompt_offering(&refs, &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);
        assert!(strip(&env, &ctx).contains("item0"));
        assert!(!strip(&env, &ctx).contains("item4"), "not yet");

        for _ in 0..4 {
            press(&ctx, &env, KeyCode::Char('n'));
        }

        assert!(
            strip(&env, &ctx).contains("item4"),
            "{:?}",
            strip(&env, &ctx)
        );
        assert!(!strip(&env, &ctx).contains("item0"), "the page turned");
        assert_eq!(selected(&env, &ctx), "item4");
    }

    /// Every cell is padded to the same width even past the end of the list,
    /// because the position of a candidate is computed rather than searched
    /// for -- a short row would put every position below it out by however
    /// many characters were missing.
    #[test]
    fn every_row_is_the_same_length_even_when_the_candidates_run_out() {
        let (ctx, env) = with_module();
        run("(setq completion-columns 3)", &env, &ctx);
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        let shown = strip(&env, &ctx);
        let lengths: Vec<usize> = shown.lines().map(str::len).collect();
        assert!(
            lengths.windows(2).all(|pair| pair[0] == pair[1]),
            "rows differ in length: {lengths:?}"
        );
    }

    /// A candidate longer than its column is cut rather than wrapped: wrapping
    /// would put one candidate in two cells and make every position after it
    /// wrong.
    #[test]
    fn a_candidate_too_wide_for_its_column_is_truncated() {
        let (ctx, env) = with_module();
        run("(setq completion-columns 3)", &env, &ctx);
        let long = "x".repeat(200);
        prompt_offering(&[&long, "short"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        let shown = strip(&env, &ctx);
        let first = shown.lines().next().expect("a row");
        assert_eq!(first.len(), W, "the row is still one frame wide");
        assert!(
            first.contains("short"),
            "and the next cell is where it should be"
        );
    }

    // ---------------- what the module leaves behind ----------------

    #[test]
    fn choosing_clears_the_state_so_a_later_answer_cannot_be_delivered_late() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);
        press(&ctx, &env, KeyCode::Enter);

        assert_eq!(run("*completion--items*", &env, &ctx), LispExp::nil());
        assert_eq!(run("*completion--window*", &env, &ctx), LispExp::nil());
        assert_eq!(run("*completion--on-choose*", &env, &ctx), LispExp::nil());
    }

    /// The strip is taken down *before* the chosen value is handed on, because
    /// what receives it may start another completion -- picking a directory and
    /// then completing inside it is the obvious case. Handing on first would
    /// have the second completion open while the first one's window and state
    /// were still standing, and the close that followed would take down the
    /// second one's window instead of the first one's.
    #[test]
    fn the_strip_is_gone_before_the_chosen_value_is_handed_on() {
        let (ctx, env) = with_module();
        run(
            r#"(progn (setq *windows-when-called* nil)
                      (setq *items-when-called* nil)
                      (completion--present (list "alpha" "beta")
                        (lambda (v)
                          (setq *windows-when-called* (count-windows))
                          (setq *items-when-called* *completion--items*))))"#,
            &env,
            &ctx,
        );

        press(&ctx, &env, KeyCode::Enter);

        assert_eq!(
            run("*windows-when-called*", &env, &ctx),
            LispExp::number(1.0),
            "the strip was already closed when the callback ran"
        );
        assert_eq!(
            run("*items-when-called*", &env, &ctx),
            LispExp::nil(),
            "and its state was already cleared"
        );
    }

    /// The case that ordering is for: choosing one candidate starts another
    /// completion, and exactly one strip must be left standing.
    #[test]
    fn a_completion_started_from_a_choice_leaves_one_strip_not_two() {
        let (ctx, env) = with_module();
        run(
            r#"(progn (setq *test-answer* nil)
                      (completion--present (list "first" "second")
                        (lambda (v)
                          (completion--present (list "inner-a" "inner-b")
                                               (lambda (w) (setq *test-answer* w))))))"#,
            &env,
            &ctx,
        );
        assert_eq!(windows(&ctx), 2);

        press(&ctx, &env, KeyCode::Enter);

        assert_eq!(windows(&ctx), 2, "the second strip, and only it");
        assert!(strip(&env, &ctx).contains("inner-a"));
        press(&ctx, &env, KeyCode::Enter);
        assert_eq!(
            run("*test-answer*", &env, &ctx),
            LispExp::string("inner-a".into()),
            "and the second one still answers"
        );
        assert_eq!(windows(&ctx), 1);
    }

    /// Twice in a row, which is what a user does: complete, edit, complete
    /// again. The second must not find the first one's window still open.
    #[test]
    fn a_second_completion_opens_exactly_one_strip() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);

        press(&ctx, &env, KeyCode::Tab);
        press(&ctx, &env, KeyCode::Esc);
        press(&ctx, &env, KeyCode::Tab);

        assert_eq!(windows(&ctx), 2);
    }

    /// The keyboard is not left captured by a window that is no longer there.
    #[test]
    fn abandoning_the_strip_gives_the_keyboard_back() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);
        press(&ctx, &env, KeyCode::Esc);

        press(&ctx, &env, KeyCode::Char('z'));

        assert_eq!(
            minibuffer_text(&env, &ctx),
            "z",
            "typing reaches the prompt again"
        );
    }
}
