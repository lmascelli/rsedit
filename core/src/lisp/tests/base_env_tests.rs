//! Edge-case tests for the *real* standard library in `base_env.rs`.
//!
//! `lisp_core_compliance_tests.rs` deliberately avoids `setup_base_env` so
//! it stays a pure check of `lisp.rs` itself; that means none of the ~60
//! primitives registered by `setup_base_env` (list ops, arithmetic,
//! strings, predicates, ...) had any dedicated tests anywhere. This file
//! fills that gap, focused on boundary conditions and the handful of
//! primitives (`eq`/`eql`, `listp`, `functionp`, `atom`/`make-atom`, `+`,
//! `-`, `mod`) that were fixed to match real Emacs Lisp semantics.
#[cfg(test)]
mod tests {
    use crate::lisp::{Env, EvalError, LispContext, LispExp, Parser, eval, setup_base_env};
    use std::sync::Arc;

    fn env_with_primitives() -> Arc<Env<()>> {
        let env = Env::new_root();
        setup_base_env(env.clone());
        env
    }

    fn eval_str(source: &str, env: Arc<Env<()>>) -> Result<LispExp<()>, EvalError> {
        let wrapped = format!("(progn {})", source);
        let mut parser = Parser::new(&wrapped);
        let ast = parser.next().expect("failed to parse test script");
        eval(&ast, env, &())
    }

    fn eval_ok(source: &str) -> LispExp<()> {
        eval_str(source, env_with_primitives()).unwrap_or_else(|e| {
            panic!("eval of `{source}` failed: {e:?}");
        })
    }

    fn eval_err(source: &str) -> EvalError {
        eval_str(source, env_with_primitives()).expect_err(&format!("expected `{source}` to fail"))
    }

    // ===========================================================
    // Lists: dotted pairs, append, out-of-range access
    // ===========================================================

    #[test]
    fn car_and_cdr_work_on_a_dotted_pair() {
        assert_eq!(eval_ok("(car (cons 1 2))"), LispExp::number(1.0));
        assert_eq!(eval_ok("(cdr (cons 1 2))"), LispExp::number(2.0));
    }

    #[test]
    fn cons_onto_a_dotted_pair_extends_the_proper_list_prefix() {
        // (cons 1 (cons 2 3)) => (1 2 . 3)
        assert_eq!(
            eval_ok("(cons 1 (cons 2 3))"),
            LispExp::dotted_list(
                vec![LispExp::number(1.0), LispExp::number(2.0)],
                LispExp::number(3.0)
            )
        );
    }

    #[test]
    fn append_with_a_non_list_final_argument_produces_a_dotted_list() {
        assert_eq!(
            eval_ok("(append '(1 2) 3)"),
            LispExp::dotted_list(
                vec![LispExp::number(1.0), LispExp::number(2.0)],
                LispExp::number(3.0)
            )
        );
    }

    #[test]
    fn append_with_no_arguments_returns_nil() {
        assert_eq!(eval_ok("(append)"), LispExp::nil());
    }

    #[test]
    fn nth_and_elt_out_of_range_or_negative_return_nil_not_an_error() {
        assert_eq!(eval_ok("(nth 5 '(1 2 3))"), LispExp::nil());
        assert_eq!(eval_ok("(nth -1 '(1 2 3))"), LispExp::nil());
        assert_eq!(eval_ok("(elt \"hi\" 5)"), LispExp::nil());
        assert_eq!(eval_ok("(elt \"hi\" -1)"), LispExp::nil());
    }

    #[test]
    fn nthcdr_at_or_past_the_end_returns_nil() {
        assert_eq!(eval_ok("(nthcdr 3 '(1 2 3))"), LispExp::nil());
        assert_eq!(eval_ok("(nthcdr 99 '(1 2 3))"), LispExp::nil());
    }

    // ===========================================================
    // Arithmetic: division by zero, unary quirks, chaining, mod sign
    // ===========================================================

    #[test]
    fn division_with_a_single_argument_returns_the_reciprocal() {
        assert_eq!(eval_ok("(/ 4)"), LispExp::number(0.25));
    }

    #[test]
    fn division_and_mod_by_zero_signal_a_runtime_error() {
        assert!(matches!(eval_err("(/ 1 0)"), EvalError::RuntimeMessage(_)));
        assert!(matches!(eval_err("(/ 0)"), EvalError::RuntimeMessage(_)));
        assert!(matches!(
            eval_err("(mod 1 0)"),
            EvalError::RuntimeMessage(_)
        ));
    }

    #[test]
    fn sum_with_no_arguments_returns_zero() {
        assert_eq!(eval_ok("(+)"), LispExp::number(0.0));
    }

    #[test]
    fn subtraction_with_a_single_argument_negates_it() {
        assert_eq!(eval_ok("(- 5)"), LispExp::number(-5.0));
        assert_eq!(eval_ok("(- -5)"), LispExp::number(5.0));
    }

    #[test]
    fn mod_result_takes_the_sign_of_the_divisor() {
        assert_eq!(eval_ok("(mod 7 3)"), LispExp::number(1.0));
        assert_eq!(eval_ok("(mod 7 -3)"), LispExp::number(-2.0));
        assert_eq!(eval_ok("(mod -7 3)"), LispExp::number(2.0));
        assert_eq!(eval_ok("(mod -7 -3)"), LispExp::number(-1.0));
    }

    #[test]
    fn chained_relational_comparisons_check_every_adjacent_pair() {
        assert_eq!(eval_ok("(< 1 2 3)"), LispExp::t());
        assert_eq!(eval_ok("(< 1 3 2)"), LispExp::nil());
        assert_eq!(eval_ok("(<= 1 1 2)"), LispExp::t());
    }

    #[test]
    fn numeric_equality_requires_every_argument_to_match() {
        assert_eq!(eval_ok("(= 1 1 1)"), LispExp::t());
        assert_eq!(eval_ok("(= 1 1 2)"), LispExp::nil());
    }

    // ===========================================================
    // Strings: negative substring indices, format directives
    // ===========================================================

    #[test]
    fn substring_negative_indices_count_from_the_end() {
        assert_eq!(
            eval_ok("(substring \"hello\" -3)"),
            LispExp::string("llo".into())
        );
        assert_eq!(
            eval_ok("(substring \"hello\" -3 -1)"),
            LispExp::string("ll".into())
        );
    }

    #[test]
    fn substring_start_after_end_is_a_runtime_error() {
        assert!(matches!(
            eval_err("(substring \"hi\" 1 0)"),
            EvalError::RuntimeMessage(_)
        ));
    }

    #[test]
    fn format_percent_percent_is_a_literal_percent() {
        assert_eq!(
            eval_ok("(format \"100%%\")"),
            LispExp::string("100%".into())
        );
    }

    #[test]
    fn format_running_out_of_arguments_is_an_error() {
        assert!(matches!(
            eval_err("(format \"%s\")"),
            EvalError::WrongNumberOfArguments { .. }
        ));
    }

    #[test]
    fn split_string_defaults_to_whitespace_and_drops_empty_pieces() {
        assert_eq!(
            eval_ok("(split-string \"  a  b c \")"),
            LispExp::list(vec![
                LispExp::string("a".into()),
                LispExp::string("b".into()),
                LispExp::string("c".into()),
            ])
        );
    }

    #[test]
    fn split_string_with_empty_separator_splits_into_characters() {
        assert_eq!(
            eval_ok("(split-string \"ab\" \"\")"),
            LispExp::list(vec![
                LispExp::string("a".into()),
                LispExp::string("b".into())
            ])
        );
    }

    #[test]
    fn number_to_string_omits_the_decimal_point_for_integral_values() {
        assert_eq!(eval_ok("(number-to-string 5)"), LispExp::string("5".into()));
        assert_eq!(
            eval_ok("(number-to-string 5.5)"),
            LispExp::string("5.5".into())
        );
    }

    #[test]
    fn string_to_number_returns_zero_for_unparseable_input() {
        assert_eq!(
            eval_ok("(string-to-number \"not-a-number\")"),
            LispExp::number(0.0)
        );
    }

    // ===========================================================
    // Calling conventions: funcall vs. apply/mapcar symbol resolution
    // ===========================================================

    #[test]
    fn funcall_apply_and_mapcar_all_resolve_a_symbol_argument() {
        // `funcall`, `apply`, and `mapcar` all go through the shared
        // `call_callable` helper, so a quoted symbol naming a function
        // works the same way in all three -- matching real Elisp, where
        // `funcall` accepts a symbol too.
        assert_eq!(eval_ok("(funcall 'car '(1 2))"), LispExp::number(1.0));
        assert_eq!(eval_ok("(apply 'car '((1 2)))"), LispExp::number(1.0));
        assert_eq!(
            eval_ok("(mapcar 'car '((1 2) (3 4)))"),
            LispExp::list(vec![LispExp::number(1.0), LispExp::number(3.0)])
        );
    }

    #[test]
    fn add_to_list_requires_an_existing_list_variable() {
        assert!(matches!(
            eval_err("(add-to-list 'undefined-list 1)"),
            EvalError::RuntimeMessage(_)
        ));
    }

    // ===========================================================
    // Predicates: eq/eql identity vs. equal structural comparison,
    // listp/functionp resolving nil and function-bound symbols
    // ===========================================================

    #[test]
    fn eq_is_identity_based_while_equal_is_structural() {
        // Two independently-built lists with the same contents are `equal`
        // but not `eq` -- they're different allocations.
        assert_eq!(eval_ok("(eq (list 1 2) (list 1 2))"), LispExp::nil());
        assert_eq!(eval_ok("(equal (list 1 2) (list 1 2))"), LispExp::t());
    }

    #[test]
    fn eq_treats_numbers_and_symbols_as_immediate_values() {
        // Numbers and symbols have no separate "identity" in this VM, so
        // `eq` compares them the same way `eql`/`equal` would.
        assert_eq!(eval_ok("(eq 1 1)"), LispExp::t());
        assert_eq!(eval_ok("(eq 'foo 'foo)"), LispExp::t());
        assert_eq!(eval_ok("(eq 1 2)"), LispExp::nil());
    }

    #[test]
    fn eq_on_a_list_bound_to_two_variables_is_the_same_allocation() {
        // `x` and `y` both point at the one list `setq` built, so this is
        // the case real Elisp's `eq` is actually meant to catch.
        let script = "(setq x (list 1 2)) (setq y x) (eq x y)";
        assert_eq!(eval_ok(script), LispExp::t());
    }

    #[test]
    fn listp_recognizes_nil_as_a_list() {
        assert_eq!(eval_ok("(listp nil)"), LispExp::t());
        assert_eq!(eval_ok("(listp '(1))"), LispExp::t());
        assert_eq!(eval_ok("(listp 5)"), LispExp::nil());
    }

    #[test]
    fn functionp_resolves_a_function_bound_symbol() {
        assert_eq!(eval_ok("(functionp 'car)"), LispExp::t());
        assert_eq!(eval_ok("(functionp (lambda () 1))"), LispExp::t());
        assert_eq!(eval_ok("(functionp 'no-such-function)"), LispExp::nil());
        // A variable-only binding doesn't count.
        assert_eq!(
            eval_ok("(setq x 1) (functionp 'x)"),
            LispExp::nil()
        );
    }

    #[test]
    fn predicates_agree_with_the_actual_runtime_type() {
        assert_eq!(eval_ok("(consp '(1))"), LispExp::t());
        assert_eq!(eval_ok("(consp nil)"), LispExp::nil());
        assert_eq!(eval_ok("(stringp \"x\")"), LispExp::t());
        assert_eq!(eval_ok("(stringp 1)"), LispExp::nil());
    }

    // ===========================================================
    // atom / make-atom: the real Elisp predicate vs. the rsedit
    // concurrency ref-cell constructor it used to share a name with
    // ===========================================================

    #[test]
    fn atom_is_the_real_elisp_non_cons_predicate() {
        assert_eq!(eval_ok("(atom 5)"), LispExp::t());
        assert_eq!(eval_ok("(atom nil)"), LispExp::t());
        assert_eq!(eval_ok("(atom '(1 2))"), LispExp::nil());
    }

    #[test]
    fn make_atom_builds_a_mutable_reference_cell() {
        let script = "(setq r (make-atom 1)) (reset r 2) (deref r)";
        assert_eq!(eval_ok(script), LispExp::number(2.0));
    }
}
