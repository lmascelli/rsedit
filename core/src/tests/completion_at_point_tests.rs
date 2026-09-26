//! Completion at point: the source protocol, the merge, and the five sources
//! `completion-at-point.lisp` ships.
//!
//! # What these are really protecting
//!
//! Two things that are easy to get wrong and silent when they are.
//!
//! The first is the **merge**. Sources that claim the same text contribute to
//! one list; sources that claim different text are passed over. Get that
//! backwards and completion still appears to work -- it just quietly offers
//! half of what it should, or replaces the wrong characters.
//!
//! The second is that **this works with nothing loaded**. No presenter, no
//! module, no sources: every one of those is optional, and each has a test
//! here that runs without it.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
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

    /// An editor with the base Lisp layers but **no completion sources**, which
    /// is how the editor comes up before any module is loaded.
    fn plain() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// The same, with the shipped sources.
    fn with_sources() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = plain();
        eval_str(
            include_str!("../../lisp/completion-at-point.lisp"),
            &env,
            &ctx,
        )
        .expect("loading completion-at-point.lisp");
        (ctx, env)
    }

    /// Put `text` in *scratch* with point at offset `at`.
    fn typing(ctx: &Ctx, text: &str, at: usize) {
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(at);
            b.text.cursor_move(line, col);
            b.is_modified = false;
        });
    }

    /// Put `text` in *scratch* with point at its end -- the usual case.
    fn typed(ctx: &Ctx, text: &str) {
        typing(ctx, text, text.chars().count());
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.with_buffer("*scratch*", |b| b.text.to_string())
            .expect("*scratch*")
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos_1d())
            .expect("*scratch*")
    }

    fn echo(ctx: &Ctx) -> String {
        ctx.get_echo_message()
    }

    fn strings(exp: &LispExp<Ctx>) -> Vec<String> {
        exp.iter()
            .map(|item| match item {
                LispExp::String(s) => (*s).clone(),
                LispExp::Symbol(s) => (*s).clone(),
                LispExp::Cons(cell) => match &cell.car {
                    LispExp::String(s) | LispExp::Symbol(s) => s.to_string(),
                    other => panic!("expected a string, got {other:?}"),
                },
                other => panic!("expected a string, got {other:?}"),
            })
            .collect()
    }

    /// A source that always claims `start..end` and offers `values`.
    fn define_source(
        env: &Arc<Env<Ctx>>,
        ctx: &Ctx,
        name: &str,
        start: usize,
        end: usize,
        values: &[&str],
    ) {
        let items = values
            .iter()
            .map(|v| format!("\"{v}\""))
            .collect::<Vec<_>>()
            .join(" ");
        run(
            &format!("(defun {name} () (list {start} {end} (list {items})))"),
            env,
            ctx,
        );
    }

    fn use_sources(env: &Arc<Env<Ctx>>, ctx: &Ctx, names: &[&str]) {
        let quoted = names.join(" ");
        run(
            &format!("(set-completion-functions nil '({quoted}))"),
            env,
            ctx,
        );
    }

    // -----------------------------------------------------------------------
    // The protocol
    // -----------------------------------------------------------------------

    #[test]
    fn a_source_that_answers_nil_is_passed_over() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        run("(defun capf-silent () nil)", &env, &ctx);
        define_source(&env, &ctx, "capf-loud", 0, 2, &["abcdef"]);
        use_sources(&env, &ctx, &["capf-silent", "capf-loud"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "abcdef");
    }

    #[test]
    fn nothing_to_complete_is_reported_rather_than_silent() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        run("(defun capf-silent () nil)", &env, &ctx);
        use_sources(&env, &ctx, &["capf-silent"]);

        assert_eq!(run("(completion-at-point)", &env, &ctx), LispExp::nil());
        assert!(
            echo(&ctx).contains("No completion source"),
            "got {:?}",
            echo(&ctx)
        );
    }

    #[test]
    fn the_editor_completes_with_no_sources_configured_at_all() {
        // The command is built in and the sources are a module. Coming up with
        // an empty list has to be an ordinary state, not a crash.
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        assert_eq!(run("(completion-functions)", &env, &ctx), LispExp::nil());
        assert_eq!(run("(completion-at-point)", &env, &ctx), LispExp::nil());
        assert_eq!(contents(&ctx), "ab");
    }

    #[test]
    fn one_candidate_is_simply_inserted() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-one", 0, 2, &["abcdef"]);
        use_sources(&env, &ctx, &["capf-one"]);

        assert_eq!(run("(completion-at-point)", &env, &ctx), LispExp::t());
        assert_eq!(contents(&ctx), "abcdef");
        assert_eq!(point(&ctx), 6, "point follows the inserted text");
    }

    #[test]
    fn candidates_are_filtered_against_the_text_in_the_region() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-many", 0, 2, &["abcdef", "zzz", "abzz"]);
        use_sources(&env, &ctx, &["capf-many"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(
            echo(&ctx),
            "2 completions",
            "`zzz' does not start with `ab'"
        );
        assert_eq!(contents(&ctx), "ab", "and they share nothing beyond `ab'");
    }

    #[test]
    fn several_candidates_fill_in_as_far_as_they_agree() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-many", 0, 2, &["abcdef", "abcdzz"]);
        use_sources(&env, &ctx, &["capf-many"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "abcd");
        assert_eq!(echo(&ctx), "2 completions");
    }

    #[test]
    fn no_match_is_reported_and_changes_nothing() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-wrong", 0, 2, &["zzz"]);
        use_sources(&env, &ctx, &["capf-wrong"]);

        assert_eq!(run("(completion-at-point)", &env, &ctx), LispExp::nil());
        assert_eq!(contents(&ctx), "ab");
        assert!(echo(&ctx).contains("No completion"), "got {:?}", echo(&ctx));
    }

    // -----------------------------------------------------------------------
    // The merge -- sources agreeing, and sources not
    // -----------------------------------------------------------------------

    #[test]
    fn sources_claiming_the_same_text_are_merged() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-a", 0, 2, &["abcd"]);
        define_source(&env, &ctx, "capf-b", 0, 2, &["abce"]);
        use_sources(&env, &ctx, &["capf-a", "capf-b"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(echo(&ctx), "2 completions", "both sources contributed");
        assert_eq!(contents(&ctx), "abc");
    }

    #[test]
    fn a_source_claiming_different_text_is_passed_over() {
        // Two sources completing different spans cannot both be right, and
        // taking the second one's candidates would replace text the first
        // never offered to touch.
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-whole", 0, 2, &["abcd"]);
        define_source(&env, &ctx, "capf-part", 1, 2, &["abce", "abcf"]);
        use_sources(&env, &ctx, &["capf-whole", "capf-part"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "abcd", "only the first source's candidate");
    }

    #[test]
    fn the_first_source_to_answer_fixes_the_region() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-part", 1, 2, &["bxx"]);
        define_source(&env, &ctx, "capf-whole", 0, 2, &["abcd"]);
        use_sources(&env, &ctx, &["capf-part", "capf-whole"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "abxx",
            "the region was 1..2, so only `b' was replaced"
        );
    }

    #[test]
    fn a_repeated_candidate_is_offered_once() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-a", 0, 2, &["abcd"]);
        define_source(&env, &ctx, "capf-b", 0, 2, &["abcd"]);
        use_sources(&env, &ctx, &["capf-a", "capf-b"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "abcd",
            "one candidate, so it is simply inserted"
        );
    }

    #[test]
    fn the_first_source_to_offer_a_name_keeps_its_description() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        run(
            "(defun capf-a () (list 0 2 (list (cons \"abcd\" \"first\"))))\
             (defun capf-b () (list 0 2 (list (cons \"abcd\" \"second\") \"abce\")))\
             (setq *seen* nil)\
             (defun present (items choose) (setq *seen* items))\
             (setq *completion-read-function* 'present)",
            &env,
            &ctx,
        );
        use_sources(&env, &ctx, &["capf-a", "capf-b"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(
            run("(cdr (car *seen*))", &env, &ctx),
            LispExp::string("first".into())
        );
        // And the repeat is gone rather than merely ranked behind it. Only a
        // presenter can see this: with the duplicate left in, the candidates
        // still share the same prefix, so the text put in the buffer is the
        // same either way and the list is silently one longer than it should
        // be.
        assert_eq!(
            run("(length *seen*)", &env, &ctx),
            LispExp::number(2.0),
            "two distinct names were offered between them, not three"
        );
    }

    #[test]
    fn a_malformed_answer_costs_that_source_and_not_the_command() {
        // A completion list is configuration in the way a grammar is: one
        // broken entry must not be the difference between completing and not.
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        run("(defun capf-broken () (list 1))", &env, &ctx);
        define_source(&env, &ctx, "capf-good", 0, 2, &["abcd"]);
        use_sources(&env, &ctx, &["capf-broken", "capf-good"]);

        assert_eq!(run("(completion-at-point)", &env, &ctx), LispExp::t());
        assert_eq!(contents(&ctx), "abcd");
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("capf-broken")),
            "the broken source should be named in the diagnostics"
        );
    }

    #[test]
    fn an_end_before_its_start_is_refused_rather_than_used() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        run(
            "(defun capf-backwards () (list 2 0 (list \"abcd\")))",
            &env,
            &ctx,
        );
        use_sources(&env, &ctx, &["capf-backwards"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "ab");
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("capf-backwards"))
        );
    }

    // -----------------------------------------------------------------------
    // Filtering is one decision, in one place
    // -----------------------------------------------------------------------

    #[test]
    fn the_match_can_be_replaced_wholesale() {
        // The point of filtering centrally: one variable makes every source
        // match differently, without any source being touched.
        let (ctx, env) = plain();
        typed(&ctx, "bc");
        define_source(&env, &ctx, "capf-many", 0, 2, &["abcdef", "zzz"]);
        use_sources(&env, &ctx, &["capf-many"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "bc", "a prefix match finds nothing");

        run(
            "(defun contains-filter (pattern items) (match-list items pattern))\
             (setq *completion-filter-function* 'contains-filter)",
            &env,
            &ctx,
        );
        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "abcdef", "matching on containment finds it");
    }

    // -----------------------------------------------------------------------
    // Handing over to a presenter
    // -----------------------------------------------------------------------

    #[test]
    fn a_presenter_is_given_the_candidates_and_a_way_to_answer() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-many", 0, 2, &["abcd", "abce"]);
        use_sources(&env, &ctx, &["capf-many"]);
        run(
            "(setq *seen* nil) (setq *choose* nil)\
             (defun present (items choose) (setq *seen* items) (setq *choose* choose))\
             (setq *completion-read-function* 'present)",
            &env,
            &ctx,
        );

        assert_eq!(run("(completion-at-point)", &env, &ctx), LispExp::t());
        assert_eq!(strings(&run("*seen*", &env, &ctx)), vec!["abcd", "abce"]);
        assert_eq!(
            run("*choose*", &env, &ctx),
            LispExp::symbol("completion-at-point-choose".into())
        );
        assert_eq!(
            contents(&ctx),
            "ab",
            "nothing is inserted until one is chosen"
        );

        run("(funcall *choose* \"abce\")", &env, &ctx);
        assert_eq!(contents(&ctx), "abce");
        assert_eq!(point(&ctx), 4);
    }

    #[test]
    fn choosing_with_nothing_in_progress_does_not_edit_the_buffer() {
        // The presenter is a module and may answer late -- after the user gave
        // up, or twice. Inserting at point anyway would edit a buffer nobody
        // asked to edit.
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        assert_eq!(
            run("(completion-at-point-choose \"zzz\")", &env, &ctx),
            LispExp::nil()
        );
        assert_eq!(contents(&ctx), "ab");
    }

    #[test]
    fn choosing_twice_only_completes_once() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-many", 0, 2, &["abcd", "abce"]);
        use_sources(&env, &ctx, &["capf-many"]);
        run(
            "(defun present (items choose) nil) (setq *completion-read-function* 'present)",
            &env,
            &ctx,
        );
        run("(completion-at-point)", &env, &ctx);
        run("(completion-at-point-choose \"abcd\")", &env, &ctx);
        assert_eq!(contents(&ctx), "abcd");
        run("(completion-at-point-choose \"abce\")", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "abcd",
            "the region was consumed by the first answer"
        );
    }

    // -----------------------------------------------------------------------
    // It is an edit like any other
    // -----------------------------------------------------------------------

    #[test]
    fn completing_is_one_undo_step() {
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-one", 0, 2, &["abcdef"]);
        use_sources(&env, &ctx, &["capf-one"]);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "abcdef");
        run("(undo)", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "ab",
            "one undo, not one per half of the replacement"
        );
    }

    #[test]
    fn a_read_only_buffer_is_not_completed_into() {
        let (ctx, env) = plain();
        typing(&ctx, "ab", 1);
        define_source(&env, &ctx, "capf-one", 0, 2, &["abcdef"]);
        use_sources(&env, &ctx, &["capf-one"]);
        run("(set-buffer-read-only t)", &env, &ctx);

        assert_eq!(run("(completion-at-point)", &env, &ctx), LispExp::nil());
        assert_eq!(contents(&ctx), "ab");
        assert_eq!(
            point(&ctx),
            1,
            "point must not travel to the end of text that was never inserted"
        );
        assert_eq!(
            echo(&ctx),
            "Buffer is read-only",
            "and the refusal says so, through the one message every refused edit uses"
        );
    }

    #[test]
    fn a_completion_that_would_change_nothing_changes_nothing() {
        // The candidates agree on no more than what is already written. Going
        // ahead and "replacing" the region with itself would mark the buffer
        // modified and add an undo entry for an edit that never happened.
        let (ctx, env) = plain();
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-many", 0, 2, &["abcd", "abzz"]);
        use_sources(&env, &ctx, &["capf-many"]);
        let modified = || {
            ctx.with_buffer("*scratch*", |b| b.is_modified)
                .expect("*scratch*")
        };
        assert!(!modified(), "the harness starts unmodified");

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "ab");
        assert_eq!(echo(&ctx), "2 completions");
        assert!(!modified(), "nothing was inserted, so nothing was edited");
    }

    #[test]
    fn completion_at_point_is_available_from_m_x_and_from_the_keyboard() {
        let (ctx, env) = plain();
        assert!(
            ctx.command_names()
                .iter()
                .any(|name| name == "completion-at-point"),
            "M-x should reach it"
        );
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("loading common-keymaps.lisp");
        typed(&ctx, "ab");
        define_source(&env, &ctx, "capf-one", 0, 2, &["abcdef"]);
        use_sources(&env, &ctx, &["capf-one"]);

        use crate::input::{KeyCode, KeyEvent, KeyModifiers};
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Tab,
                modifiers: KeyModifiers {
                    alt: true,
                    ..Default::default()
                },
            },
            &env,
        );
        assert_eq!(contents(&ctx), "abcdef", "M-<tab> completes");
    }

    // -----------------------------------------------------------------------
    // The registry: per mode, and global
    // -----------------------------------------------------------------------

    #[test]
    fn a_modes_sources_are_tried_before_the_global_ones() {
        // The reason the split exists: a mode knows something the editor does
        // not, and has to be able to say it first.
        let (ctx, env) = plain();
        run("(make-mode 'toy)", &env, &ctx);
        ctx.with_buffer_mut("*scratch*", |b| b.current_mode = "toy".into());
        typed(&ctx, "ab");

        run(
            "(defun capf-mode () (list 0 2 (list (cons \"abcd\" \"from the mode\"))))\
             (defun capf-global () (list 0 2 (list (cons \"abcd\" \"from everywhere\") \"abce\")))\
             (setq *seen* nil)\
             (defun present (items choose) (setq *seen* items))\
             (setq *completion-read-function* 'present)",
            &env,
            &ctx,
        );
        run("(add-completion-function 'toy 'capf-mode)", &env, &ctx);
        run("(add-completion-function nil 'capf-global)", &env, &ctx);

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(
            run("(cdr (car *seen*))", &env, &ctx),
            LispExp::string("from the mode".into())
        );
    }

    #[test]
    fn a_modes_sources_do_not_leak_into_another_mode() {
        let (ctx, env) = plain();
        run(
            "(make-mode 'toy) (defun capf-mode () (list 0 2 (list \"abcd\")))",
            &env,
            &ctx,
        );
        run("(add-completion-function 'toy 'capf-mode)", &env, &ctx);
        typed(&ctx, "ab");

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "ab", "*scratch* is not in toy mode");

        ctx.with_buffer_mut("*scratch*", |b| b.current_mode = "toy".into());
        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "abcd");
    }

    #[test]
    fn adding_a_source_to_an_unknown_mode_is_reported_not_signalled() {
        let (ctx, env) = plain();
        assert_eq!(
            run(
                "(add-completion-function 'no-such-mode 'capf-x)",
                &env,
                &ctx
            ),
            LispExp::nil()
        );
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("no-such-mode"))
        );
    }

    #[test]
    fn a_source_can_be_removed_and_the_order_changed() {
        let (ctx, env) = plain();
        run("(defun capf-a () nil) (defun capf-b () nil)", &env, &ctx);
        run("(add-completion-function nil 'capf-a)", &env, &ctx);
        run("(add-completion-function nil 'capf-b)", &env, &ctx);
        assert_eq!(
            strings(&run("(completion-functions)", &env, &ctx)),
            vec!["capf-a", "capf-b"]
        );

        run("(set-completion-functions nil '(capf-b))", &env, &ctx);
        assert_eq!(
            strings(&run("(completion-functions)", &env, &ctx)),
            vec!["capf-b"]
        );
    }

    #[test]
    fn a_modes_list_is_replaced_too_and_not_added_to() {
        // The same promise for a mode's list as for the global one. They are
        // two different stores behind one primitive, so "replace" has to be
        // true of both or the word means nothing.
        let (ctx, env) = plain();
        run(
            "(make-mode 'toy) (defun capf-a () nil) (defun capf-b () nil)",
            &env,
            &ctx,
        );
        run("(add-completion-function 'toy 'capf-a)", &env, &ctx);
        assert_eq!(
            strings(&run("(completion-functions 'toy)", &env, &ctx)),
            vec!["capf-a"]
        );

        run("(set-completion-functions 'toy '(capf-b))", &env, &ctx);
        assert_eq!(
            strings(&run("(completion-functions 'toy)", &env, &ctx)),
            vec!["capf-b"],
            "`capf-a' should be gone, not still there in front"
        );
    }

    #[test]
    fn completion_functions_reports_one_list_unmerged() {
        let (ctx, env) = plain();
        run(
            "(make-mode 'toy) (defun capf-a () nil) (defun capf-b () nil)",
            &env,
            &ctx,
        );
        run("(add-completion-function 'toy 'capf-a)", &env, &ctx);
        run("(add-completion-function nil 'capf-b)", &env, &ctx);

        assert_eq!(
            strings(&run("(completion-functions 'toy)", &env, &ctx)),
            vec!["capf-a"]
        );
        assert_eq!(
            strings(&run("(completion-functions)", &env, &ctx)),
            vec!["capf-b"]
        );
        assert_eq!(
            run("(completion-functions 'no-such-mode)", &env, &ctx),
            LispExp::nil()
        );
    }

    // -----------------------------------------------------------------------
    // A language's own words
    // -----------------------------------------------------------------------
    //
    // A mode's vocabulary is no longer something the editor knows about. A
    // language module puts a list on its own symbol and `capf-mode-keywords`
    // reads it back, so what these check is a convention between two Lisp
    // modules rather than a primitive -- there is no primitive left to check.
    //
    // One consequence worth seeing written down: `put` does not care whether
    // the symbol names a mode that exists. It is a property of a symbol, and a
    // symbol needs no permission to carry one. The old `set-mode-keywords`
    // refused an unknown mode; nothing refuses now.

    #[test]
    fn a_mode_can_declare_its_vocabulary() {
        let (ctx, env) = plain();
        run("(put 'toy 'keywords '(\"begin\" \"end\"))", &env, &ctx);
        assert_eq!(
            strings(&run("(get 'toy 'keywords)", &env, &ctx)),
            vec!["begin", "end"]
        );
    }

    #[test]
    fn declaring_a_vocabulary_replaces_rather_than_accumulates() {
        let (ctx, env) = plain();
        run("(put 'toy 'keywords '(\"begin\"))", &env, &ctx);
        run("(put 'toy 'keywords '(\"end\"))", &env, &ctx);
        assert_eq!(
            strings(&run("(get 'toy 'keywords)", &env, &ctx)),
            vec!["end"]
        );
    }

    #[test]
    fn the_vocabulary_read_is_the_one_for_this_buffers_mode() {
        let (ctx, env) = plain();
        run(
            "(make-mode 'toy) (put 'toy 'keywords '(\"begin\"))",
            &env,
            &ctx,
        );
        assert_eq!(
            run("(get (major-mode) 'keywords)", &env, &ctx),
            LispExp::nil(),
            "*scratch* is not in toy mode"
        );

        ctx.with_buffer_mut("*scratch*", |b| b.current_mode = "toy".into());
        assert_eq!(
            strings(&run("(get (major-mode) 'keywords)", &env, &ctx)),
            vec!["begin"]
        );
    }

    #[test]
    fn a_mode_that_declared_nothing_has_no_vocabulary() {
        let (ctx, env) = plain();
        run("(make-mode 'toy)", &env, &ctx);
        assert_eq!(run("(get 'toy 'keywords)", &env, &ctx), LispExp::nil());
    }

    // -----------------------------------------------------------------------
    // Which mode a buffer is in
    // -----------------------------------------------------------------------

    #[test]
    fn a_buffer_reports_the_mode_it_is_in() {
        let (ctx, env) = plain();
        assert_eq!(
            run("(major-mode)", &env, &ctx),
            LispExp::symbol("fundamental-mode".into()),
            "a buffer nobody gave a mode is in the default one"
        );

        ctx.with_buffer_mut("*scratch*", |b| b.current_mode = "toy".into());
        assert_eq!(
            run("(major-mode)", &env, &ctx),
            LispExp::symbol("toy".into())
        );
    }

    #[test]
    fn the_mode_comes_back_as_a_symbol_and_not_a_string() {
        // It is handed straight to `get`, which takes either -- so nothing
        // fails if this regresses to a string. What breaks instead is `eq`
        // against a mode written in source, which is how any dispatch on mode
        // would be written.
        let (ctx, env) = plain();
        assert_eq!(
            run("(eq (major-mode) 'fundamental-mode)", &env, &ctx),
            LispExp::t()
        );
    }

    #[test]
    fn another_buffer_can_be_asked_about() {
        let (ctx, env) = plain();
        run("(buffer-create \"other\" 'toy)", &env, &ctx);
        assert_eq!(
            run("(major-mode \"other\")", &env, &ctx),
            LispExp::symbol("toy".into())
        );
        assert_eq!(
            run("(major-mode)", &env, &ctx),
            LispExp::symbol("fundamental-mode".into()),
            "asking about another buffer does not change which one is current"
        );
    }

    #[test]
    fn a_buffer_that_does_not_exist_has_no_mode() {
        let (ctx, env) = plain();
        assert_eq!(
            run("(major-mode \"no-such-buffer\")", &env, &ctx),
            LispExp::nil()
        );
    }

    // -----------------------------------------------------------------------
    // bounds-of-thing-at-point
    // -----------------------------------------------------------------------

    #[test]
    fn a_symbol_is_bounded_by_what_is_not_one() {
        let (ctx, env) = plain();
        typing(&ctx, "(foo-bar baz)", 8);
        assert_eq!(
            strings(&run(
                "(mapcar 'number-to-string (bounds-of-thing-at-point 'symbol))",
                &env,
                &ctx
            )),
            vec!["1", "8"]
        );
    }

    #[test]
    fn the_whole_symbol_is_claimed_from_inside_it() {
        // Completing in the middle of a word replaces the word, rather than
        // leaving the tail of the old one stranded after the new one.
        let (ctx, env) = plain();
        typing(&ctx, "forward", 3);
        assert_eq!(
            strings(&run(
                "(mapcar 'number-to-string (bounds-of-thing-at-point))",
                &env,
                &ctx
            )),
            vec!["0", "7"]
        );
    }

    #[test]
    fn there_is_no_symbol_where_there_is_no_symbol() {
        let (ctx, env) = plain();
        typing(&ctx, "( )", 1);
        assert_eq!(
            run("(bounds-of-thing-at-point 'symbol)", &env, &ctx),
            LispExp::nil()
        );
    }

    #[test]
    fn stars_and_hyphens_are_part_of_a_name_here() {
        // `*completion-read-function*` is one identifier, not four.
        let (ctx, env) = plain();
        typed(&ctx, "(setq *some-name*");
        assert_eq!(
            strings(&run(
                "(mapcar 'number-to-string (bounds-of-thing-at-point))",
                &env,
                &ctx
            )),
            vec!["6", "17"]
        );
    }

    #[test]
    fn a_file_name_claims_more_than_the_symbol_inside_it() {
        // Which is the whole reason a path source and a symbol source do not
        // merge: they are not completing the same text.
        let (ctx, env) = plain();
        typed(&ctx, "  /usr/lo");
        assert_eq!(
            strings(&run(
                "(mapcar 'number-to-string (bounds-of-thing-at-point 'filename))",
                &env,
                &ctx
            )),
            vec!["2", "9"]
        );
        assert_eq!(
            strings(&run(
                "(mapcar 'number-to-string (bounds-of-thing-at-point 'symbol))",
                &env,
                &ctx
            )),
            vec!["7", "9"],
            "the symbol is `lo', starting after the slash the path claimed"
        );
    }

    #[test]
    fn an_unknown_kind_of_thing_is_an_error() {
        let (ctx, env) = plain();
        typed(&ctx, "abc");
        assert!(eval_str("(bounds-of-thing-at-point 'sentence)", &env, &ctx).is_err());
    }

    // -----------------------------------------------------------------------
    // buffer-words
    // -----------------------------------------------------------------------

    #[test]
    fn buffer_words_are_distinct_and_sorted() {
        let (ctx, env) = plain();
        typed(&ctx, "beta alpha beta gamma");
        assert_eq!(
            strings(&run("(buffer-words)", &env, &ctx)),
            vec!["alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn short_words_are_not_worth_completing() {
        let (ctx, env) = plain();
        typed(&ctx, "a an and ante");
        assert_eq!(
            strings(&run("(buffer-words)", &env, &ctx)),
            vec!["and", "ante"]
        );
        assert_eq!(
            strings(&run("(buffer-words nil 4)", &env, &ctx)),
            vec!["ante"]
        );
    }

    #[test]
    fn a_word_running_to_the_end_of_the_buffer_is_still_a_word() {
        let (ctx, env) = plain();
        typed(&ctx, "alpha beta");
        assert_eq!(
            strings(&run("(buffer-words)", &env, &ctx)),
            vec!["alpha", "beta"]
        );
    }

    #[test]
    fn another_buffer_can_be_named() {
        let (ctx, env) = plain();
        typed(&ctx, "here");
        run("(buffer-create \"other\")", &env, &ctx);
        run(
            "(with-current-buffer \"other\" (lambda () (insert \"elsewhere\")))",
            &env,
            &ctx,
        );

        assert_eq!(
            strings(&run("(buffer-words \"other\")", &env, &ctx)),
            vec!["elsewhere"]
        );
        assert_eq!(
            run("(buffer-words \"no-such-buffer\")", &env, &ctx),
            LispExp::nil()
        );
    }

    // -----------------------------------------------------------------------
    // The shipped sources
    // -----------------------------------------------------------------------

    #[test]
    fn symbols_are_completed_from_what_the_interpreter_can_resolve() {
        let (ctx, env) = with_sources();
        run("(defun zzz-a-real-function () nil)", &env, &ctx);
        typed(&ctx, "zzz-a-real-f");

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "zzz-a-real-function");
    }

    #[test]
    fn a_name_that_does_not_exist_is_not_offered() {
        let (ctx, env) = with_sources();
        typed(&ctx, "zzz-no-such-n");
        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "zzz-no-such-n");
        assert!(echo(&ctx).contains("No completion"), "got {:?}", echo(&ctx));
    }

    #[test]
    fn a_macro_and_a_variable_are_offered_with_what_they_are() {
        let (ctx, env) = with_sources();
        run(
            "(setq *seen* nil)\
             (defun present (items choose) (setq *seen* items))\
             (setq *completion-read-function* 'present)\
             (setq zzz-shared-name 1)\
             (defun zzz-shared-name-fn () nil)",
            &env,
            &ctx,
        );
        typed(&ctx, "zzz-shared-name");
        run("(completion-at-point)", &env, &ctx);
        let kinds = strings(&run("(mapcar 'cdr *seen*)", &env, &ctx));
        assert!(kinds.contains(&"variable".to_string()), "got {kinds:?}");
        assert!(kinds.contains(&"function".to_string()), "got {kinds:?}");
    }

    #[test]
    fn the_word_being_typed_is_not_offered_back() {
        // It is in the buffer, so `capf-buffer-words` reads it -- and offering
        // it makes it the common prefix, so the key press appears to do
        // nothing at all.
        let (ctx, env) = with_sources();
        typed(&ctx, "alphabet alphabetic alpha");
        run(
            "(set-completion-functions nil '(capf-buffer-words))",
            &env,
            &ctx,
        );

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(
            contents(&ctx),
            "alphabet alphabetic alphabet",
            "with `alpha' offered back, the candidates' common prefix would be \
             `alpha' and this would fill in what was already there"
        );
    }

    #[test]
    fn words_come_from_other_buffers_too() {
        let (ctx, env) = with_sources();
        run("(buffer-create \"other\")", &env, &ctx);
        run(
            "(with-current-buffer \"other\" (lambda () (insert \"zzzelsewhere\")))",
            &env,
            &ctx,
        );
        typed(&ctx, "zzzelse");
        run(
            "(set-completion-functions nil '(capf-buffer-words))",
            &env,
            &ctx,
        );

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "zzzelsewhere");
    }

    #[test]
    fn a_declared_keyword_is_completed() {
        let (ctx, env) = with_sources();
        run(
            "(make-mode 'toy) (put 'toy 'keywords '(\"zzzbegin\"))",
            &env,
            &ctx,
        );
        ctx.with_buffer_mut("*scratch*", |b| b.current_mode = "toy".into());
        typed(&ctx, "zzzbeg");
        run(
            "(set-completion-functions nil '(capf-mode-keywords))",
            &env,
            &ctx,
        );

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "zzzbegin");
    }

    #[test]
    fn a_buffer_name_is_completed() {
        let (ctx, env) = with_sources();
        run("(buffer-create \"zzz-a-buffer\")", &env, &ctx);
        typed(&ctx, "zzz-a-b");
        run(
            "(set-completion-functions nil '(capf-buffer-names))",
            &env,
            &ctx,
        );

        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "zzz-a-buffer");
    }

    #[test]
    fn a_path_is_completed_and_plain_text_is_left_to_the_others() {
        let (ctx, env) = with_sources();
        run(
            "(set-completion-functions nil '(capf-file-name))",
            &env,
            &ctx,
        );

        typed(&ctx, "notapath");
        assert_eq!(run("(completion-at-point)", &env, &ctx), LispExp::nil());
        assert!(
            echo(&ctx).contains("No completion source"),
            "text with no separator in it is not a path: {:?}",
            echo(&ctx)
        );
    }

    #[test]
    fn regexp_opt_escapes_what_the_engine_would_otherwise_read() {
        // This language's names are made of metacharacters. Unquoted, the
        // pattern for `1+' matches `111', and the one for `*items*' does not
        // compile -- and a rule that fails to compile is logged and dropped,
        // so the mode silently colours nothing.
        let (ctx, env) = plain();
        let pattern = match run("(regexp-opt '(\"1+\" \"*items*\"))", &env, &ctx) {
            LispExp::String(s) => (*s).clone(),
            other => panic!("expected a string, got {other:?}"),
        };
        let compiled = regex::Regex::new(&pattern).expect("the pattern must compile");
        assert!(compiled.is_match("1+"));
        assert!(compiled.is_match("*items*"));
        assert!(!compiled.is_match("111"));
    }

    #[test]
    fn a_regexp_for_no_words_matches_nothing_rather_than_everything() {
        // An alternation of nothing matches the empty string at every
        // position, which as a syntax rule faces the whole buffer.
        let (ctx, env) = plain();
        let pattern = match run("(regexp-opt nil)", &env, &ctx) {
            LispExp::String(s) => (*s).clone(),
            other => panic!("expected a string, got {other:?}"),
        };
        let compiled = regex::Regex::new(&pattern).expect("the pattern must compile");
        assert!(!compiled.is_match(""));
        assert!(!compiled.is_match("anything at all"));
    }

    #[test]
    fn rust_mode_colours_and_completes_from_one_list() {
        // The feature working end to end in something that ships: the words
        // `capf-mode-keywords' offers are the same words the grammar colours,
        // because there is only one list.
        let (ctx, env) = with_sources();
        eval_str(include_str!("../../lisp/rust-mode.lisp"), &env, &ctx).expect("loading rust-mode");
        let declared = strings(&run("(get 'rust-mode 'keywords)", &env, &ctx));
        assert!(declared.contains(&"unsafe".to_string()), "got {declared:?}");
        assert!(declared.contains(&"usize".to_string()), "got {declared:?}");

        ctx.with_buffer_mut("*scratch*", |b| b.current_mode = "rust-mode".into());
        typed(&ctx, "unsaf");
        run(
            "(set-completion-functions nil '(capf-mode-keywords))",
            &env,
            &ctx,
        );
        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx), "unsafe");
    }

    #[test]
    fn a_words_regexp_does_not_take_a_capture_group_with_it() {
        // A rule wrapping this still wants group 1 for its own -- that is how
        // a grammar colours a name and not the delimiter it had to match.
        let (ctx, env) = plain();
        let pattern = match run(
            "(concat \"x(\" (regexp-opt '(\"a\" \"b\")) \")y\")",
            &env,
            &ctx,
        ) {
            LispExp::String(s) => (*s).clone(),
            other => panic!("expected a string, got {other:?}"),
        };
        let compiled = regex::Regex::new(&pattern).expect("the pattern must compile");
        let found = compiled.captures("xay").expect("it should match");
        assert_eq!(
            found.len(),
            2,
            "group 0 and the caller's group 1, and no more"
        );
        assert_eq!(found.get(1).map(|m| m.as_str()), Some("a"));
    }

    #[test]
    fn a_substring_may_be_asked_for_in_either_order() {
        // A caller holding a region has it as two positions, not as a sorted
        // pair, and should not have to sort it before asking.
        let (ctx, env) = plain();
        typed(&ctx, "alphabet");
        assert_eq!(
            run("(buffer-substring 5 1)", &env, &ctx),
            run("(buffer-substring 1 5)", &env, &ctx)
        );
        assert_eq!(
            run("(buffer-substring 1 5)", &env, &ctx),
            LispExp::string("lpha".into())
        );
    }

    #[test]
    fn a_name_bound_in_two_scopes_is_offered_once() {
        // `defun` binds in whatever environment it runs in, so a definition
        // inside a `let' shadows the outer one rather than replacing it and
        // both bindings exist at once. Only one of them would resolve.
        let (ctx, env) = plain();
        run("(defun zzz-twice () 1)", &env, &ctx);
        let names = strings(&run(
            "(let ((ignored 1)) (defun zzz-twice () 2) (all-functions))",
            &env,
            &ctx,
        ));
        assert_eq!(names.iter().filter(|name| *name == "zzz-twice").count(), 1);
    }

    #[test]
    fn the_default_sources_are_ordered_most_specific_first() {
        let (ctx, env) = with_sources();
        assert_eq!(
            strings(&run("(completion-functions)", &env, &ctx)),
            vec![
                "capf-file-name",
                "capf-mode-keywords",
                "capf-symbols",
                "capf-buffer-names",
                "capf-buffer-words",
            ],
            "the source that answers to anything must come last"
        );
    }
}
