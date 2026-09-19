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
