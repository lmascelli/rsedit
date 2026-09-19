//! What things cost, as the fuel budget sees it.
//!
//! The budget is a runaway-loop guard: it exists so a mistyped `(while t ...)`
//! cannot hang the editor, and it is sized so that happens in about a second.
//! That guarantee only holds while the charge for a piece of work is
//! proportional to the work -- which is why `expect_list` charges per element
//! rather than per call.
//!
//! These tests are about the two ways that goes wrong: Lisp that does
//! quadratic work without meaning to, and primitives that do a lot of work for
//! one unit of fuel.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
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

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env")
    }

    /// An editor with the modules loaded, for the ones that are about Lisp
    /// this project ships rather than about the interpreter.
    fn editor_with_modules() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor();
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/completion.lisp"),
            include_str!("../../lisp/manpage.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// What one evaluation costs, exactly.
    fn cost(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> u64 {
        let ast = Parser::new(src).next().expect("source must parse");
        let (outcome, spent) =
            crate::lisp::measure(ctx.fuel_meter(), || eval(&ast, env.clone(), ctx));
        outcome.unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"));
        spent
    }

    /// A list of `n` strings, built the cheap way.
    fn list_of(n: usize) -> String {
        format!(
            "(let ((out nil) (i 0)) (while (< i {n}) (setq out (cons \"name\" out)) \
             (setq i (+ i 1))) out)"
        )
    }

    // ----------------------------------------------------------------
    // The pattern that ran out of fuel
    // ----------------------------------------------------------------

    #[test]
    fn appending_one_element_at_a_time_is_quadratic_and_runs_out() {
        // The shape `manpage-names` and `find-file-recursive--candidates` both
        // had: `append` copies everything gathered so far, so building an
        // n-element list this way costs about n squared. It gives out at
        // roughly 4,500 elements -- which is *under* the 5,000 that
        // `find-file-recursive-limit` advertises, and far under the number of
        // manual pages on a real system.
        let (ctx, env) = editor();
        let outcome = eval_str(
            "(let ((out nil) (i 0)) (while (< i 6000) \
               (setq out (append out (list i))) (setq i (+ i 1))) (length out))",
            &env,
            &ctx,
        );
        assert!(
            matches!(outcome, Err(EvalError::OutOfFuel)),
            "this is the bug, and it should stay demonstrable: {outcome:?}"
        );
    }

    #[test]
    fn consing_and_reversing_the_same_list_is_linear_and_fits() {
        // The fix, at the same size.
        let (ctx, env) = editor();
        let answer = run(
            "(let ((out nil) (i 0)) (while (< i 6000) \
               (setq out (cons i out)) (setq i (+ i 1))) (length (reverse out)))",
            &env,
            &ctx,
        );
        assert_eq!(answer, LispExp::number(6000.0));
    }

    #[test]
    fn mapcar_over_a_long_list_fits() {
        // What `manpage-names` uses now. One pass, one result.
        let (ctx, env) = editor();
        let answer = run(
            &format!(
                "(length (mapcar (lambda (x) x) {}))",
                list_of(20000)
            ),
            &env,
            &ctx,
        );
        assert_eq!(answer, LispExp::number(20000.0));
    }

    // ----------------------------------------------------------------
    // The modules that had it
    // ----------------------------------------------------------------

    #[test]
    fn reading_a_page_name_costs_the_same_whatever_its_length() {
        // `manpage--strip-extensions` used to walk the name a character at a
        // time asking `(< n (length name))` -- and `length` on a string
        // charges per character, so each step cost the length and the whole
        // thing cost its square: 184 units for a 7-character name, 808 for a
        // 20-character one. Called once per page on a system with twenty
        // thousand of them, that was the entire budget spent deciding what
        // the files were called.
        //
        // Asserting that the cost does not grow with the name is the precise
        // claim, and it is an exact integer rather than a timing, so it means
        // the same on every machine.
        let (ctx, env) = editor_with_modules();
        let short = cost(r#"(manpage--strip-extensions "ls.1.gz")"#, &env, &ctx);
        let long = cost(
            r#"(manpage--strip-extensions "systemd-analyze-verify.1.gz")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            short, long,
            "a longer name must not cost more: {short} against {long}"
        );
    }

    #[test]
    fn naming_twenty_thousand_pages_fits_in_the_budget() {
        // The size that failed: a system with a real set of manual pages.
        //
        // Costed rather than run. One name is measured and multiplied, which
        // is exact -- the cost does not depend on the name, which is what the
        // test above establishes -- and does not spend fifteen seconds of a
        // debug build proving arithmetic.
        let (ctx, env) = editor_with_modules();
        let each = cost(r#"(manpage--strip-extensions "git-rebase.1.gz")"#, &env, &ctx);
        let total = each * 20_000;
        assert!(
            total < u64::from(crate::lisp::DEFAULT_FUEL),
            "naming 20,000 pages costs {total} of {} -- it used to be over the whole budget",
            crate::lisp::DEFAULT_FUEL
        );
    }

    #[test]
    fn a_directory_walk_is_charged_for_what_it_found() {
        // It used to hand back twenty thousand paths for three units, which
        // is the same gap `fuzzy-filter` had: a loop of walks that the
        // runaway guard would never stop.
        let (ctx, env) = editor_with_modules();
        let spent = cost(r#"(directory-files-recursive "." 200)"#, &env, &ctx);
        let found = match run(r#"(length (nth 1 (directory-files-recursive "." 200)))"#, &env, &ctx) {
            LispExp::Number(n) => n as u64,
            other => panic!("expected a count, got {other:?}"),
        };
        assert!(
            spent >= found,
            "walking {found} paths should cost at least {found} units, cost {spent}"
        );
    }


    #[test]
    fn a_long_candidate_list_can_be_filtered_without_running_out() {
        // `manpage-candidates` on a machine with a real set of manual pages:
        // twenty thousand names is an ordinary number, not an extreme one.
        let (ctx, env) = editor();
        let answer = run(
            &format!(r#"(length (fuzzy-filter "nm" {}))"#, list_of(20000)),
            &env,
            &ctx,
        );
        assert!(!answer.is_nil());
    }

    #[test]
    fn filtering_is_charged_for_what_it_walks() {
        // A primitive that scores twenty thousand candidates for one unit of
        // fuel would let `(while t (fuzzy-filter ...))` run far past the
        // second the guard is sized for.
        //
        // Measured rather than demonstrated by exhaustion: `measure` counts
        // the units exactly, which is both a stronger claim and a test that
        // finishes -- proving it by running the budget down means doing ten
        // million scorings, which takes seventeen seconds in a debug build.
        let (ctx, env) = editor();
        let names = run(&list_of(5000), &env, &ctx);
        env.set_variable("names".into(), names);

        let ast = Parser::new(r#"(fuzzy-filter "nm" names)"#)
            .next()
            .expect("source must parse");
        let (outcome, spent) = crate::lisp::measure(ctx.fuel_meter(), || {
            eval(&ast, env.clone(), &ctx)
        });
        assert!(outcome.is_ok());
        assert!(
            spent >= 5000,
            "scoring 5000 candidates should cost at least 5000 units, cost {spent}"
        );
    }

    #[test]
    fn one_filtering_of_a_long_list_still_costs_almost_nothing() {
        // The other half: charging honestly must not make ordinary use
        // expensive. Twenty thousand of ten million is a fifth of a percent,
        // so this can be done many times over within one command.
        let (ctx, env) = editor();
        let answer = run(
            &format!(
                r#"(let ((names {}) (i 0)) (while (< i 5) (fuzzy-filter "nm" names) (setq i (+ i 1))) i)"#,
                list_of(5000)
            ),
            &env,
            &ctx,
        );
        assert_eq!(answer, LispExp::number(5.0));
    }
}
