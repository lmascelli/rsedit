//! `catch`/`throw`, `signal` and `condition-case`: the two non-local exits.
//!
//! They deliberately behave differently. A throw is a *control transfer* and
//! belongs to the `catch` naming its tag -- it passes straight through a
//! `condition-case`. An error is a *failure* and belongs to the nearest
//! matching `condition-case` -- it passes straight through a `catch`.
#[cfg(test)]
mod tests {
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval, setup_base_env};
    use std::sync::Arc;

    fn eval_str(source: &str) -> Result<LispExp<()>, EvalError<()>> {
        let env = Env::new_root();
        setup_base_env(env.clone());
        let wrapped = format!("(progn {source})");
        let mut parser = Parser::new(&wrapped);
        let ast = parser.next().expect("failed to parse test script");
        eval(&ast, env, &())
    }

    fn eval_ok(source: &str) -> LispExp<()> {
        eval_str(source).unwrap_or_else(|e| panic!("eval of `{source}` failed: {e:?}"))
    }

    // ------------------------------ catch / throw -------------------------

    #[test]
    fn a_catch_without_a_throw_returns_its_last_body_form() {
        assert_eq!(eval_ok("(catch 'tag 1 2 3)"), LispExp::number(3.0));
        assert_eq!(eval_ok("(catch 'tag)"), LispExp::nil());
    }

    #[test]
    fn a_throw_unwinds_to_the_matching_catch_and_supplies_its_value() {
        assert_eq!(
            eval_ok("(catch 'done (throw 'done 42) 99)"),
            LispExp::number(42.0)
        );
    }

    #[test]
    fn a_throw_unwinds_out_of_arbitrarily_deep_calls() {
        assert_eq!(
            eval_ok(
                "(defun deep (n) (if (= n 0) (throw 'found 7) (deep (- n 1))))
                 (catch 'found (deep 50))"
            ),
            LispExp::number(7.0)
        );
    }

    #[test]
    fn a_throw_escapes_a_loop_mid_iteration() {
        assert_eq!(
            eval_ok("(catch 'done (dolist (x '(1 2 3 4)) (if (= x 3) (throw 'done x))))"),
            LispExp::number(3.0)
        );
    }

    #[test]
    fn an_inner_catch_with_another_tag_lets_the_throw_pass_through() {
        assert_eq!(
            eval_ok("(catch 'outer (catch 'inner (throw 'outer 5)) 99)"),
            LispExp::number(5.0)
        );
    }

    #[test]
    fn the_innermost_matching_catch_wins() {
        assert_eq!(
            eval_ok("(catch 'tag (catch 'tag (throw 'tag 1) 2) 3)"),
            LispExp::number(3.0)
        );
    }

    #[test]
    fn a_throw_carries_any_value_not_just_a_number() {
        assert_eq!(
            eval_ok("(catch 'tag (throw 'tag '(a b c)))"),
            LispExp::proper_list(vec![
                LispExp::symbol("a".into()),
                LispExp::symbol("b".into()),
                LispExp::symbol("c".into()),
            ])
        );
    }

    #[test]
    fn a_throw_with_no_catch_at_all_escapes_carrying_its_payload() {
        // The payload travels inside the error rather than beside it, so an
        // uncaught throw can still be inspected in full by the host.
        let err = eval_str("(throw 'nowhere 1)").expect_err("should not be caught");
        match err {
            EvalError::Throw { tag, value } => {
                assert_eq!(tag, LispExp::symbol("nowhere".into()));
                assert_eq!(value, LispExp::number(1.0));
            }
            other => panic!("expected a Throw carrying its payload, got {other:?}"),
        }
    }

    #[test]
    fn throw_is_a_function_so_it_works_through_funcall() {
        assert_eq!(
            eval_ok("(catch 'tag (funcall 'throw 'tag 8))"),
            LispExp::number(8.0)
        );
    }

    // ----------------------------- condition-case -------------------------

    #[test]
    fn condition_case_returns_the_body_value_when_nothing_fails() {
        assert_eq!(
            eval_ok("(condition-case nil (+ 1 2) (error 'handled))"),
            LispExp::number(3.0)
        );
    }

    #[test]
    fn the_error_handler_runs_and_supplies_the_value() {
        assert_eq!(
            eval_ok("(condition-case nil (undefined-fn) (error 'handled))"),
            LispExp::symbol("handled".into())
        );
    }

    #[test]
    fn a_specific_condition_matches_only_its_own_error() {
        assert_eq!(
            eval_ok("(condition-case nil (undefined-fn) (undefined-function 'right))"),
            LispExp::symbol("right".into())
        );
        assert!(matches!(
            eval_str("(condition-case nil (undefined-fn) (out-of-fuel 'wrong))")
                .expect_err("the handler should not have matched"),
            EvalError::UndefinedFunction(_)
        ));
    }

    #[test]
    fn a_handler_condition_may_be_a_list_and_the_first_match_wins() {
        assert_eq!(
            eval_ok(
                "(condition-case nil (undefined-fn)
                   ((out-of-fuel undefined-function) 'matched))"
            ),
            LispExp::symbol("matched".into())
        );
        assert_eq!(
            eval_ok(
                "(condition-case nil (undefined-fn)
                   (unbound-variable 'first)
                   (undefined-function 'second))"
            ),
            LispExp::symbol("second".into())
        );
    }

    // ------------- the point of the generic error type --------------------

    #[test]
    fn a_handler_receives_the_offending_value_itself_not_a_rendering_of_it() {
        // `(car 5)` fails with wrong-type-argument. The handler gets the 5 as
        // a number it can compute with -- before the error type was generic
        // this was a `format!("{:?}")` string and the value was lost.
        assert_eq!(
            eval_ok("(condition-case e (car 5) (wrong-type-argument (nth 1 (cdr e))))"),
            LispExp::number(5.0)
        );
        assert_eq!(
            eval_ok("(condition-case e (car 5) (wrong-type-argument (+ 1 (nth 1 (cdr e)))))"),
            LispExp::number(6.0)
        );
    }

    #[test]
    fn the_bound_variable_is_the_condition_symbol_consed_onto_its_data() {
        assert_eq!(
            eval_ok("(condition-case e (undefined-fn) (error (car e)))"),
            LispExp::symbol("undefined-function".into())
        );
        assert_eq!(
            eval_ok("(condition-case e (undefined-fn) (error (car (cdr e))))"),
            LispExp::symbol("undefined-fn".into())
        );
    }

    #[test]
    fn lisp_can_raise_and_catch_its_own_conditions() {
        assert_eq!(
            eval_ok("(condition-case e (signal 'my-error '(1 2)) (my-error (cdr e)))"),
            LispExp::proper_list(vec![LispExp::number(1.0), LispExp::number(2.0)])
        );
        // `error` still covers a user-defined condition.
        assert_eq!(
            eval_ok("(condition-case nil (signal 'my-error nil) (error 'caught))"),
            LispExp::symbol("caught".into())
        );
        // A handler for a different condition does not.
        assert!(matches!(
            eval_str("(condition-case nil (signal 'my-error nil) (other-error 'wrong))")
                .expect_err("the handler should not have matched"),
            EvalError::Signal { .. }
        ));
    }

    // ------------------------- where the two interact ---------------------

    #[test]
    fn a_condition_case_does_not_intercept_a_throw() {
        assert_eq!(
            eval_ok("(catch 'tag (condition-case nil (throw 'tag 11) (error 'wrong)))"),
            LispExp::number(11.0)
        );
    }

    #[test]
    fn a_catch_does_not_intercept_an_error() {
        assert!(matches!(
            eval_str("(catch 'tag (undefined-fn))").expect_err("the catch must not swallow this"),
            EvalError::UndefinedFunction(_)
        ));
    }

    #[test]
    fn unwind_protect_cleanups_run_on_the_way_out_of_a_throw() {
        assert_eq!(
            eval_ok(
                "(setq trace nil)
                 (catch 'tag (unwind-protect (throw 'tag 'thrown) (setq trace 'cleaned)))
                 trace"
            ),
            LispExp::symbol("cleaned".into())
        );
    }

    #[test]
    fn unwind_protect_cleanups_run_on_the_way_out_of_an_error() {
        assert_eq!(
            eval_ok(
                "(setq trace nil)
                 (condition-case nil
                     (unwind-protect (undefined-fn) (setq trace 'cleaned))
                   (error nil))
                 trace"
            ),
            LispExp::symbol("cleaned".into())
        );
    }
}
