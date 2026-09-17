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

    /// `C-n`, `C-p` -- the keys the strip binds now that bare letters have to
    /// reach the buffer.
    fn press_ctrl(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode) {
        ctx.handle_key_event(
            KeyEvent {
                code,
                modifiers: KeyModifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
            env,
        );
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

    /// A prompt whose candidates actually depend on what has been typed --
    /// which is what a real one does, and what `prompt_offering` deliberately
    /// does not, so that the two cases can be told apart.
    fn prompt_filtering(names: &[&str], env: &Arc<Env<Ctx>>, ctx: &Ctx) {
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
                            (lambda (input) (fuzzy-filter input (list {list})))
                            nil))"#
            ),
            env,
            ctx,
        );
    }

    // ---------------- narrowing as you type ----------------

    #[test]
    fn typing_narrows_the_strip() {
        let (ctx, env) = with_module();
        prompt_filtering(&["alpha", "beta", "gamma"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);
        assert!(strip(&env, &ctx).contains("alpha"));

        press(&ctx, &env, KeyCode::Char('b'));

        let shown = strip(&env, &ctx);
        assert!(shown.contains("beta"), "beta should survive, got {shown:?}");
        assert!(!shown.contains("alpha"), "alpha should be gone, got {shown:?}");
    }

    #[test]
    fn backspacing_widens_the_list_again() {
        // Why the unnarrowed list is kept: filtering the filtered list could
        // only ever make it smaller, and backspace has to make it bigger.
        let (ctx, env) = with_module();
        prompt_filtering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);
        press(&ctx, &env, KeyCode::Char('b'));
        assert!(!strip(&env, &ctx).contains("alpha"));

        press(&ctx, &env, KeyCode::Backspace);

        assert!(strip(&env, &ctx).contains("alpha"), "alpha should be back");
    }

    #[test]
    fn moving_the_selection_does_not_re_narrow() {
        // The refresh runs after every command, `C-n` included. Without the
        // guard that compares against the last pattern, moving the selection
        // would reset it to the first candidate and the key would appear dead.
        let (ctx, env) = with_module();
        prompt_filtering(&["alpha", "beta", "gamma"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        press_ctrl(&ctx, &env, KeyCode::Char('n'));
        press(&ctx, &env, KeyCode::Enter);

        // Into the prompt, not confirming it -- see
        // `return_chooses_a_candidate_without_confirming_the_prompt`.
        assert_eq!(minibuffer_text(&env, &ctx), "beta");
    }

    #[test]
    fn typing_puts_the_selection_back_on_the_first_candidate() {
        // The one that was selected may not even be in the list any more, so
        // keeping the index would select something arbitrary.
        let (ctx, env) = with_module();
        prompt_filtering(&["alpha", "aardvark", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);
        press_ctrl(&ctx, &env, KeyCode::Char('n'));

        press(&ctx, &env, KeyCode::Char('a'));
        press(&ctx, &env, KeyCode::Enter);

        let chosen = minibuffer_text(&env, &ctx);
        assert!(
            chosen.starts_with('a'),
            "the first of what matches `a`, got {chosen:?}"
        );
    }

    #[test]
    fn nothing_matching_leaves_the_strip_up_saying_so() {
        let (ctx, env) = with_module();
        prompt_filtering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        press(&ctx, &env, KeyCode::Char('z'));

        assert_eq!(windows(&ctx), 2, "the strip stays");
        assert!(strip(&env, &ctx).contains("No matches"));
    }

    #[test]
    fn choosing_with_nothing_matching_takes_nothing() {
        let (ctx, env) = with_module();
        prompt_filtering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);
        press(&ctx, &env, KeyCode::Char('z'));

        press(&ctx, &env, KeyCode::Enter);

        // The `z` is still in the prompt and nothing replaced it, because
        // there was nothing to replace it with.
        assert_eq!(minibuffer_text(&env, &ctx), "z");
    }

    #[test]
    fn a_strip_nobody_is_typing_into_is_left_alone() {
        // A module may call the presenter directly, with no prompt and no
        // buffer position behind it. There is no text to narrow by, and the
        // refresh must leave such a strip standing rather than closing it --
        // which is exactly what a `completion-choose' callback opening a
        // second strip relies on.
        let (ctx, env) = with_module();
        run(
            r#"(completion--present (list "one" "two")
                                    (lambda (v) (setq *test-answer* v)))"#,
            &env,
            &ctx,
        );
        assert_eq!(windows(&ctx), 2, "the strip opened");
        run("(next-line)", &env, &ctx);
        assert_eq!(windows(&ctx), 2, "and is still there");
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

        press_ctrl(&ctx, &env, KeyCode::Char('n'));
        assert_eq!(selected(&env, &ctx), "beta");
        press_ctrl(&ctx, &env, KeyCode::Char('n'));
        assert_eq!(selected(&env, &ctx), "gamma");
        press_ctrl(&ctx, &env, KeyCode::Char('n'));
        assert_eq!(selected(&env, &ctx), "alpha", "past the end is the start");
        press_ctrl(&ctx, &env, KeyCode::Char('p'));
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
        press_ctrl(&ctx, &env, KeyCode::Char('n'));

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

    /// The strip used to swallow every key it did not bind, so a list of forty
    /// candidates could only be walked through one `n` at a time. It passes
    /// them on now -- and stays up, which is the part neither of the other two
    /// `OnUnbound` answers could give: `Refuse` could not be typed at, and
    /// `Release` would vanish at the first letter.
    #[test]
    fn a_key_the_strip_does_not_bind_reaches_the_prompt_and_leaves_the_strip_up() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        press(&ctx, &env, KeyCode::Char('z'));

        assert_eq!(windows(&ctx), 2, "the strip is still up");
        assert_eq!(minibuffer_text(&env, &ctx), "z", "and z was typed");
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
            press_ctrl(&ctx, &env, KeyCode::Char('n'));
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

    // ---------------- the prompt and the strip share the frame ----------------

    /// The bug: the prompt used to be docked along the bottom edge, which is
    /// exactly where the completion strip goes. Two things drawn in one place
    /// is not a layout.
    ///
    /// Checked at several sizes, because one size proves nothing here -- the
    /// answer depends on how much of the frame the strip is taking.
    #[test]
    fn on_a_frame_with_room_the_prompt_and_the_strip_do_not_overlap() {
        for (width, height) in [(80, 24), (100, 40), (60, 20)] {
            let (ctx, env) = with_module();
            prompt_offering(&["alpha", "beta", "gamma"], &env, &ctx);
            press(&ctx, &env, KeyCode::Tab);

            let frame = ctx.snapshot(&env, width, height);
            let overlap = shared_rows(&frame);
            assert!(
                overlap.is_empty(),
                "at {width}x{height} the prompt and the strip share rows {overlap:?}"
            );
        }
    }

    /// And on a frame too small to hold both, they do overlap -- there is
    /// nowhere else for a *centred* prompt to go when a six-row strip is most
    /// of the screen. What matters then is which one wins: a float is drawn
    /// after the tiled windows, so the prompt is on top and can still be read
    /// and typed into. Losing sight of some candidates is survivable; losing
    /// the prompt is not, because a prompt is how you would run the command
    /// that made the strip smaller.
    #[test]
    fn on_a_frame_too_small_for_both_the_prompt_is_drawn_over_the_strip() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha", "beta", "gamma"], &env, &ctx);
        press(&ctx, &env, KeyCode::Tab);

        let frame = ctx.snapshot(&env, 40, 12);
        assert!(
            !shared_rows(&frame).is_empty(),
            "this size is chosen because they *do* collide; if they no longer \
             do, this test is measuring nothing"
        );

        let position = |name: &str| {
            frame
                .views
                .iter()
                .position(|view| view.buffer_name == name)
                .unwrap_or_else(|| panic!("{name} is on screen"))
        };
        assert!(
            position("*Minibuffer*") > position("*Completions*"),
            "the prompt is drawn later, so it is the one that is legible"
        );
    }

    /// Rows covered by both the prompt and the completion strip.
    fn shared_rows(frame: &crate::ui::FrameSnapshot) -> Vec<isize> {
        let rect_of = |name: &str| {
            frame
                .views
                .iter()
                .find(|view| view.buffer_name == name)
                .map(|view| view.rect.clone())
                .unwrap_or_else(|| panic!("{name} is on screen"))
        };
        let prompt = rect_of("*Minibuffer*");
        let strip = rect_of("*Completions*");
        let rows = |rect: &crate::ui::Rect| rect.y..(rect.y + rect.height as isize);
        rows(&prompt)
            .filter(|row| rows(&strip).contains(row))
            .collect()
    }

    #[test]
    fn the_prompt_sits_in_the_middle_of_the_frame() {
        let (ctx, env) = with_module();
        prompt_offering(&["alpha"], &env, &ctx);

        let frame = ctx.snapshot(&env, W, H);
        let prompt = frame
            .views
            .iter()
            .find(|view| view.buffer_name == "*Minibuffer*")
            .expect("the prompt is on screen");

        let left = prompt.rect.x;
        let right = W as isize - (prompt.rect.x + prompt.rect.width as isize);
        assert!(
            (left - right).abs() <= 1,
            "margins differ: {left} left, {right} right"
        );
        let above = prompt.rect.y;
        // The echo area owns the bottom row, so the space centred in is one
        // shorter than the frame.
        let below = (H - 1) as isize - (prompt.rect.y + prompt.rect.height as isize);
        assert!(
            (above - below).abs() <= 1,
            "margins differ: {above} above, {below} below"
        );
    }

    #[test]
    fn the_prompt_is_the_size_the_variables_ask_for() {
        let (ctx, env) = with_module();
        run(
            "(setq minibuffer-width 30)(setq minibuffer-height 5)",
            &env,
            &ctx,
        );
        prompt_offering(&["alpha"], &env, &ctx);

        let frame = ctx.snapshot(&env, W, H);
        let prompt = frame
            .views
            .iter()
            .find(|view| view.buffer_name == "*Minibuffer*")
            .expect("the prompt is on screen");

        assert_eq!(prompt.rect.width, 30);
        assert_eq!(prompt.rect.height, 5);
    }

    /// A prompt must always be openable -- it is how `M-x` works, and how you
    /// would run the command that fixed the setting -- so a size the terminal
    /// cannot hold is clamped rather than refused.
    #[test]
    fn a_prompt_larger_than_the_frame_is_cut_down_to_fit() {
        let (ctx, env) = with_module();
        run(
            "(setq minibuffer-width 9999)(setq minibuffer-height 9999)",
            &env,
            &ctx,
        );
        prompt_offering(&["alpha"], &env, &ctx);

        let frame = ctx.snapshot(&env, W, H);
        let prompt = frame
            .views
            .iter()
            .find(|view| view.buffer_name == "*Minibuffer*")
            .expect("the prompt is still openable");

        assert!(prompt.rect.x >= 0);
        assert!(prompt.rect.y >= 0);
        assert!(
            prompt.rect.x + prompt.rect.width as isize <= W as isize,
            "{:?}",
            prompt.rect
        );
        assert!(
            prompt.rect.y + prompt.rect.height as isize <= H as isize,
            "{:?}",
            prompt.rect
        );
    }

    /// A nonsense size falls back to the *default*, not merely to something
    /// legal. One column wide is technically a window and is no more use than
    /// none: the point of falling back is that the prompt stays usable, and a
    /// prompt that cannot open -- or cannot be read -- leaves no way to run the
    /// command that would put the setting right.
    #[test]
    fn a_nonsense_size_falls_back_to_the_default() {
        for (setting, width, height) in [
            (r#"(setq minibuffer-width nil)"#, None, None),
            (r#"(setq minibuffer-width "wide")"#, None, None),
            (r#"(setq minibuffer-width 0)"#, None, None),
            (r#"(setq minibuffer-height -4)"#, None, None),
            // A good value beside a bad one is still honoured: they fall back
            // one at a time, not together.
            (
                r#"(setq minibuffer-width 30)(setq minibuffer-height nil)"#,
                Some(30),
                None,
            ),
        ] {
            let (ctx, env) = with_module();
            // Wide enough that the default is not clamped, so what is measured
            // is the fallback and not the frame.
            run("(setq frame-width 100)", &env, &ctx);
            run(setting, &env, &ctx);
            prompt_offering(&["alpha"], &env, &ctx);

            let frame = ctx.snapshot(&env, 100, H);
            let prompt = frame
                .views
                .iter()
                .find(|view| view.buffer_name == "*Minibuffer*")
                .unwrap_or_else(|| panic!("still openable after {setting}"));

            let expected_width = width.unwrap_or(crate::editor::DEFAULT_MINIBUFFER_WIDTH as usize);
            let expected_height =
                height.unwrap_or(crate::editor::DEFAULT_MINIBUFFER_HEIGHT as usize);
            assert_eq!(prompt.rect.width, expected_width, "width after {setting}");
            assert_eq!(
                prompt.rect.height, expected_height,
                "height after {setting}"
            );
        }
    }

    /// The echo area owns the bottom row, so a prompt is centred in what is
    /// left rather than in the whole frame. Only a tall prompt can tell the
    /// difference -- and a tall prompt whose last row sat under a message
    /// would hide the line being typed.
    #[test]
    fn a_prompt_as_tall_as_the_frame_still_leaves_the_echo_row_clear() {
        let (ctx, env) = with_module();
        run(&format!("(setq minibuffer-height {H})"), &env, &ctx);
        prompt_offering(&["alpha"], &env, &ctx);

        let frame = ctx.snapshot(&env, W, H);
        let prompt = frame
            .views
            .iter()
            .find(|view| view.buffer_name == "*Minibuffer*")
            .expect("the prompt is on screen");

        let echo_row = (H - 1) as isize;
        assert!(
            prompt.rect.y + prompt.rect.height as isize <= echo_row,
            "the prompt at {:?} reaches the echo row {echo_row}",
            prompt.rect
        );
    }

    /// Both are set at boot, so a configuration adjusts a value that already
    /// exists rather than bringing the setting into being.
    #[test]
    fn the_sizes_are_set_without_anyone_configuring_them() {
        let (ctx, env) = with_module();

        assert_eq!(
            run("minibuffer-width", &env, &ctx),
            LispExp::number(crate::editor::DEFAULT_MINIBUFFER_WIDTH)
        );
        assert_eq!(
            run("minibuffer-height", &env, &ctx),
            LispExp::number(crate::editor::DEFAULT_MINIBUFFER_HEIGHT)
        );
    }
}
