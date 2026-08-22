//! Emacs-Lisp-compliance tests for `lisp.rs` in isolation.
//!
//! Everything exercised here comes from the parser/evaluator core
//! (`lisp.rs`) alone: self-evaluating literals, truthiness, and every
//! special form (`if`, `while`, `setq`, `defun`, `lambda`, `progn`, `let`,
//! `let*`, `cond`, `and`, `or`, `when`, `unless`, `dolist`, `dotimes`,
//! `defvar`, `defconst`, `prog1`, `prog2`, `unwind-protect`, `defmacro`,
//! backquote/unquote/unquote-splicing). None of these tests call
//! `setup_base_env` from `base_env.rs`, and no environment used below is
//! seeded with anything beyond three tiny, locally-defined primitives
//! (`+`, `cons`, `eq`) needed to give a few forms (`dolist`, `dotimes`,
//! `defmacro`) something to compute with. This keeps the suite a pure
//! check of `lisp.rs`'s own Elisp-compliance, independent of whatever the
//! "real" standard library in `base_env.rs` does or how it evolves.
#[cfg(test)]
mod tests {
    use crate::lisp::{Env, EvalError, LispContext, LispExp, Parser, eval};
    use std::sync::Arc;

    #[derive(Clone, Debug, PartialEq)]
    struct DummyCtx;

    impl LispContext for DummyCtx {
        fn consume_fuel(&self, _amount: u32) -> Result<(), EvalError> {
            Ok(())
        }
        fn log_diagnostic(&self, _msg: &str) {}
    }

    // ===========================================================
    // Minimal, self-contained primitives -- NOT from base_env.rs.
    // ===========================================================

    fn native_add(
        args: &[LispExp<DummyCtx>],
        _env: Arc<Env<DummyCtx>>,
        _ctx: &DummyCtx,
    ) -> Result<LispExp<DummyCtx>, EvalError> {
        let mut sum = 0.0;
        for arg in args {
            if let LispExp::Number(n) = arg {
                sum += n;
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: format!("{:?}", arg),
                });
            }
        }
        Ok(LispExp::number(sum))
    }

    fn native_cons(
        args: &[LispExp<DummyCtx>],
        _env: Arc<Env<DummyCtx>>,
        _ctx: &DummyCtx,
    ) -> Result<LispExp<DummyCtx>, EvalError> {
        if args.len() != 2 {
            return Err(EvalError::WrongNumberOfArguments {
                expected: 2,
                got: args.len(),
            });
        }
        match &args[1] {
            LispExp::List(list) => {
                let mut items = vec![args[0].clone()];
                items.extend(list.iter().cloned());
                Ok(LispExp::list(items))
            }
            other if other.is_nil() => Ok(LispExp::list(vec![args[0].clone()])),
            other => Err(EvalError::WrongArgumentType {
                expected: "List".into(),
                got: format!("{:?}", other),
            }),
        }
    }

    fn native_eq(
        args: &[LispExp<DummyCtx>],
        _env: Arc<Env<DummyCtx>>,
        _ctx: &DummyCtx,
    ) -> Result<LispExp<DummyCtx>, EvalError> {
        if args.len() != 2 {
            return Err(EvalError::WrongNumberOfArguments {
                expected: 2,
                got: args.len(),
            });
        }
        Ok(LispExp::boolean(args[0] == args[1]))
    }

    /// A root environment with *no* `setup_base_env` call: just the raw
    /// `lisp.rs` special forms plus the three primitives above.
    fn bare_env() -> Arc<Env<DummyCtx>> {
        let env = Env::new_root();
        env.set_function("+".into(), LispExp::primitive(native_add, None));
        env.set_function("cons".into(), LispExp::primitive(native_cons, None));
        env.set_function("eq".into(), LispExp::primitive(native_eq, None));
        env
    }

    fn eval_str(source: &str, env: Arc<Env<DummyCtx>>) -> Result<LispExp<DummyCtx>, EvalError> {
        let mut ctx = DummyCtx;
        // Wrapping in `progn` lets a test script contain several top-level
        // forms while still handing `eval` a single AST node.
        let wrapped = format!("(progn {})", source);
        let mut parser = Parser::new(&wrapped);
        let ast = parser.next().expect("failed to parse test script");
        eval(&ast, env, &mut ctx)
    }

    fn eval_ok(source: &str) -> LispExp<DummyCtx> {
        eval_str(source, bare_env()).unwrap_or_else(|e| {
            panic!("eval of `{source}` failed: {e:?}");
        })
    }

    fn eval_err(source: &str) -> EvalError {
        eval_str(source, bare_env()).expect_err(&format!("expected `{source}` to fail"))
    }

    // ===========================================================
    // Self-evaluating literals & truthiness
    // ===========================================================

    #[test]
    fn nil_and_t_self_evaluate_with_no_environment_at_all() {
        // Not even `bare_env()` -- a totally empty root environment.
        let env = Env::<DummyCtx>::new_root();
        assert_eq!(eval_str("nil", env.clone()).unwrap(), LispExp::nil());
        assert_eq!(eval_str("t", env).unwrap(), LispExp::t());
    }

    #[test]
    fn keyword_symbols_self_evaluate() {
        let env = Env::<DummyCtx>::new_root();
        assert_eq!(
            eval_str(":foo", env.clone()).unwrap(),
            LispExp::symbol(":foo".into())
        );
        assert_eq!(
            eval_str(":a-longer-keyword", env).unwrap(),
            LispExp::symbol(":a-longer-keyword".into())
        );
    }

    #[test]
    fn unbound_ordinary_symbol_still_errors() {
        let env = Env::<DummyCtx>::new_root();
        assert_eq!(
            eval_str("unbound-thing", env).unwrap_err(),
            EvalError::UnboundVariable("unbound-thing".into())
        );
    }

    #[test]
    fn truthiness_only_nil_and_empty_list_are_false() {
        // Elisp truthiness: everything except `nil`/`()` is true, notably
        // including 0, "", and empty vectors/maps -- unlike many Lisps.
        assert_eq!(eval_ok("(if 0 'yes 'no)"), LispExp::symbol("yes".into()));
        assert_eq!(eval_ok("(if \"\" 'yes 'no)"), LispExp::symbol("yes".into()));
        assert_eq!(eval_ok("(if [] 'yes 'no)"), LispExp::symbol("yes".into()));
        assert_eq!(eval_ok("(if {} 'yes 'no)"), LispExp::symbol("yes".into()));
        assert_eq!(eval_ok("(if t 'yes 'no)"), LispExp::symbol("yes".into()));
        assert_eq!(eval_ok("(if nil 'yes 'no)"), LispExp::symbol("no".into()));
        assert_eq!(eval_ok("(if () 'yes 'no)"), LispExp::symbol("no".into()));
    }

    // ===========================================================
    // quote
    // ===========================================================

    #[test]
    fn quote_prevents_evaluation() {
        assert_eq!(
            eval_ok("'unbound-var"),
            LispExp::symbol("unbound-var".into())
        );
        assert_eq!(
            eval_ok("'(1 unbound-var)"),
            LispExp::list(vec![
                LispExp::number(1.0),
                LispExp::symbol("unbound-var".into())
            ])
        );
    }

    #[test]
    fn quote_requires_exactly_one_argument() {
        assert_eq!(eval_err("(quote)"), EvalError::QuoteNotOneArgument);
        assert_eq!(eval_err("(quote a b)"), EvalError::QuoteNotOneArgument);
    }

    // ===========================================================
    // if / while
    // ===========================================================

    #[test]
    fn if_selects_the_right_branch() {
        assert_eq!(eval_ok("(if t 1 2)"), LispExp::number(1.0));
        assert_eq!(eval_ok("(if nil 1 2)"), LispExp::number(2.0));
        assert_eq!(eval_ok("(if nil 1)"), LispExp::nil());
    }

    #[test]
    fn if_requires_condition_and_true_branch() {
        assert_eq!(eval_err("(if)"), EvalError::IfNoConditionProvided);
        assert_eq!(eval_err("(if t)"), EvalError::IfNoTrueBrach);
    }

    #[test]
    fn while_loops_until_condition_is_nil_and_returns_last_body_value() {
        let script = "
            (setq i 0)
            (setq acc 0)
            (while (if (eq i 5) nil t)
                (setq acc (+ acc i))
                (setq i (+ i 1)))
            acc
        ";
        // 0 + 1 + 2 + 3 + 4 = 10
        assert_eq!(eval_ok(script), LispExp::number(10.0));
    }

    #[test]
    fn while_with_never_true_condition_returns_nil() {
        assert_eq!(eval_ok("(while nil 1)"), LispExp::nil());
    }

    // ===========================================================
    // setq
    // ===========================================================

    #[test]
    fn setq_creates_and_updates_variables_and_returns_last_value() {
        let env = bare_env();
        assert_eq!(
            eval_str("(setq a 1 b 2)", env.clone()).unwrap(),
            LispExp::number(2.0)
        );
        assert_eq!(env.get_variable("a"), Some(LispExp::number(1.0)));
        assert_eq!(env.get_variable("b"), Some(LispExp::number(2.0)));

        // Re-setting an existing variable updates it in place rather than
        // shadowing.
        eval_str("(setq a 99)", env.clone()).unwrap();
        assert_eq!(env.get_variable("a"), Some(LispExp::number(99.0)));
    }

    #[test]
    fn setq_requires_a_symbol_target() {
        assert_eq!(eval_err("(setq 1 2)"), EvalError::SetqSymbolRequired);
    }

    // ===========================================================
    // defun / lambda / progn
    // ===========================================================

    #[test]
    fn defun_and_lambda_share_call_semantics() {
        assert_eq!(
            eval_ok("(defun sq (x) (+ x x)) (sq 21)"),
            LispExp::number(42.0)
        );
        // Immediately-invoked anonymous lambda -- no `defun`/`funcall`
        // needed, the evaluator dispatches on the evaluated head directly.
        assert_eq!(eval_ok("((lambda (x) (+ x 1)) 41)"), LispExp::number(42.0));
    }

    #[test]
    fn defun_supports_a_docstring_before_the_body() {
        assert_eq!(
            eval_ok("(defun greet () \"returns a greeting\" 'hello) (greet)"),
            LispExp::symbol("hello".into())
        );
    }

    #[test]
    fn functions_and_variables_are_looked_up_in_separate_namespaces() {
        // Classic Lisp-2 behaviour: binding `x` as a variable does not
        // make `(x)` callable, and vice versa.
        let env = bare_env();
        eval_str("(setq x 10)", env.clone()).unwrap();
        assert_eq!(
            eval_str("(x)", env).unwrap_err(),
            EvalError::UndefinedFunction("x".into())
        );
    }

    #[test]
    fn progn_evaluates_in_order_and_returns_the_last_value() {
        assert_eq!(
            eval_ok("(progn (setq a 1) (setq a 2) (setq a 3) a)"),
            LispExp::number(3.0)
        );
        assert_eq!(eval_ok("(progn)"), LispExp::nil());
    }

    // ===========================================================
    // let / let*
    // ===========================================================

    #[test]
    fn let_bindings_are_evaluated_in_parallel_against_the_outer_scope() {
        // `y` must NOT see the shadowed `x` -- both binding forms are
        // evaluated against the environment `let` was called in.
        let script = "(setq x 1) (let ((x 2) (y x)) y)";
        assert_eq!(eval_ok(script), LispExp::number(1.0));
    }

    #[test]
    fn let_star_bindings_see_earlier_bindings() {
        let script = "(let* ((x 2) (y (+ x 1))) y)";
        assert_eq!(eval_ok(script), LispExp::number(3.0));
    }

    #[test]
    fn let_leaves_the_outer_scope_untouched() {
        let env = bare_env();
        eval_str("(setq x 1)", env.clone()).unwrap();
        eval_str("(let ((x 2)) x)", env.clone()).unwrap();
        assert_eq!(env.get_variable("x"), Some(LispExp::number(1.0)));
    }

    #[test]
    fn let_and_let_star_support_bare_symbol_bindings_defaulting_to_nil() {
        assert_eq!(eval_ok("(let (x) x)"), LispExp::nil());
        assert_eq!(eval_ok("(let* (x (y 1)) (cons x (cons y nil)))"), {
            LispExp::list(vec![LispExp::nil(), LispExp::number(1.0)])
        });
    }

    #[test]
    fn let_reports_malformed_bindings() {
        assert_eq!(eval_err("(let)"), EvalError::LetNoBindingsProvided);
        assert_eq!(eval_err("(let x x)"), EvalError::LetUnvalidBindingList);
        assert_eq!(
            eval_err("(let ((x 1 2)) x)"),
            EvalError::LetUnvalidBindingAt(0)
        );
    }

    #[test]
    fn nested_functions_are_lexically_scoped_not_dynamically() {
        // This VM is deliberately lexically scoped (closer to modern
        // Elisp with `lexical-binding: t`): a function sees the scope it
        // was *defined* in, not the caller's scope.
        let script = "
            (setq x 10)
            (defun get-x () x)
            (let ((x 99)) (get-x))
        ";
        assert_eq!(eval_ok(script), LispExp::number(10.0));
    }

    // ===========================================================
    // cond / and / or / when / unless
    // ===========================================================

    #[test]
    fn cond_evaluates_clauses_in_order_and_stops_at_the_first_truthy_test() {
        let script = "
            (cond
                (nil 'first)
                ((eq 1 2) 'second)
                (t 'third)
                (t 'unreached))
        ";
        assert_eq!(eval_ok(script), LispExp::symbol("third".into()));
    }

    #[test]
    fn cond_clause_without_a_body_returns_the_test_value_itself() {
        assert_eq!(eval_ok("(cond (42))"), LispExp::number(42.0));
    }

    #[test]
    fn cond_falls_through_to_nil_when_nothing_matches() {
        assert_eq!(eval_ok("(cond (nil 1) (nil 2))"), LispExp::nil());
    }

    #[test]
    fn cond_rejects_malformed_clauses() {
        assert_eq!(eval_err("(cond 42)"), EvalError::CondInvalidClause);
        assert_eq!(eval_err("(cond ())"), EvalError::CondInvalidClause);
    }

    #[test]
    fn and_short_circuits_on_the_first_nil_and_returns_the_last_value() {
        assert_eq!(eval_ok("(and 1 2 3)"), LispExp::number(3.0));
        assert_eq!(eval_ok("(and 1 nil 3)"), LispExp::nil());
        assert_eq!(eval_ok("(and)"), LispExp::t());
    }

    #[test]
    fn or_short_circuits_on_the_first_truthy_value() {
        assert_eq!(eval_ok("(or nil nil 3)"), LispExp::number(3.0));
        assert_eq!(eval_ok("(or 1 2)"), LispExp::number(1.0));
        assert_eq!(eval_ok("(or nil nil)"), LispExp::nil());
        assert_eq!(eval_ok("(or)"), LispExp::nil());
    }

    #[test]
    fn and_or_do_not_evaluate_forms_past_the_short_circuit_point() {
        // If short-circuiting were broken, this would throw
        // `UnboundVariable` for `boom` instead of returning early.
        assert_eq!(eval_ok("(and nil boom)"), LispExp::nil());
        assert_eq!(eval_ok("(or t boom)"), LispExp::t());
    }

    #[test]
    fn when_runs_the_body_only_if_the_condition_is_truthy() {
        assert_eq!(eval_ok("(when t 1 2 3)"), LispExp::number(3.0));
        assert_eq!(eval_ok("(when nil 1 2 3)"), LispExp::nil());
    }

    #[test]
    fn unless_runs_the_body_only_if_the_condition_is_nil() {
        assert_eq!(eval_ok("(unless nil 1 2 3)"), LispExp::number(3.0));
        assert_eq!(eval_ok("(unless t 1 2 3)"), LispExp::nil());
    }

    // ===========================================================
    // dolist / dotimes
    // ===========================================================

    #[test]
    fn dolist_iterates_the_list_and_evaluates_the_optional_result_form() {
        let script = "
            (setq acc 0)
            (dolist (x '(1 2 3 4) acc)
                (setq acc (+ acc x)))
        ";
        assert_eq!(eval_ok(script), LispExp::number(10.0));
    }

    #[test]
    fn dolist_without_a_result_form_returns_nil() {
        assert_eq!(eval_ok("(dolist (x '(1 2 3)) x)"), LispExp::nil());
    }

    #[test]
    fn dolist_over_nil_runs_zero_times() {
        let script = "(setq ran nil) (dolist (x nil) (setq ran t)) ran";
        assert_eq!(eval_ok(script), LispExp::nil());
    }

    #[test]
    fn dotimes_counts_from_zero_up_to_but_excluding_the_bound() {
        let script = "
            (setq acc nil)
            (dotimes (i 4)
                (setq acc (cons i acc)))
            acc
        ";
        assert_eq!(
            eval_ok(script),
            LispExp::list(vec![
                LispExp::number(3.0),
                LispExp::number(2.0),
                LispExp::number(1.0),
                LispExp::number(0.0),
            ])
        );
    }

    #[test]
    fn dotimes_evaluates_the_optional_result_form_after_the_loop() {
        let script = "
            (setq acc 0)
            (dotimes (i 5 acc)
                (setq acc (+ acc i)))
        ";
        // 0+1+2+3+4 = 10
        assert_eq!(eval_ok(script), LispExp::number(10.0));
    }

    // ===========================================================
    // defvar / defconst
    // ===========================================================

    #[test]
    fn defvar_only_initializes_the_first_time() {
        let env = bare_env();
        eval_str("(defvar x 1)", env.clone()).unwrap();
        eval_str("(defvar x 2)", env.clone()).unwrap();
        assert_eq!(env.get_variable("x"), Some(LispExp::number(1.0)));
    }

    #[test]
    fn defconst_always_reinitializes() {
        let env = bare_env();
        eval_str("(defconst x 1)", env.clone()).unwrap();
        eval_str("(defconst x 2)", env.clone()).unwrap();
        assert_eq!(env.get_variable("x"), Some(LispExp::number(2.0)));
    }

    #[test]
    fn defvar_without_a_value_form_does_not_bind_the_variable() {
        let env = bare_env();
        eval_str("(defvar y)", env.clone()).unwrap();
        assert_eq!(env.get_variable("y"), None);
    }

    // ===========================================================
    // prog1 / prog2 / unwind-protect
    // ===========================================================

    #[test]
    fn prog1_returns_the_first_forms_value() {
        assert_eq!(eval_ok("(prog1 1 2 3)"), LispExp::number(1.0));
    }

    #[test]
    fn prog2_returns_the_second_forms_value() {
        assert_eq!(eval_ok("(prog2 1 2 3)"), LispExp::number(2.0));
    }

    #[test]
    fn prog1_still_evaluates_every_later_form_for_side_effects() {
        let script =
            "(setq log nil) (prog1 'first (setq log (cons 1 log)) (setq log (cons 2 log))) log";
        assert_eq!(
            eval_ok(script),
            LispExp::list(vec![LispExp::number(2.0), LispExp::number(1.0)])
        );
    }

    #[test]
    fn unwind_protect_returns_the_body_value_when_nothing_fails() {
        assert_eq!(eval_ok("(unwind-protect 42 99)"), LispExp::number(42.0));
    }

    #[test]
    fn unwind_protect_runs_cleanup_forms_even_when_the_body_errors() {
        let env = bare_env();
        let result = eval_str(
            "(unwind-protect (this-function-does-not-exist) (setq cleanup-ran t))",
            env.clone(),
        );
        assert!(result.is_err());
        assert_eq!(env.get_variable("cleanup-ran"), Some(LispExp::t()));
    }

    #[test]
    fn unwind_protect_propagates_the_bodys_error_after_cleanup() {
        assert_eq!(
            eval_err("(unwind-protect (this-function-does-not-exist) 1)"),
            EvalError::UndefinedFunction("this-function-does-not-exist".into())
        );
    }

    // ===========================================================
    // defmacro + backquote/unquote/unquote-splicing
    // ===========================================================

    #[test]
    fn defmacro_arguments_are_not_evaluated_before_expansion() {
        // `boom` is unbound. If macro arguments were evaluated eagerly
        // like a function's, `(capture boom)` would raise
        // `UnboundVariable` before the macro body ever ran. Because
        // they're passed through as raw, unevaluated AST and only
        // wrapped in `quote` by the macro body, the call succeeds and
        // simply returns the symbol `boom` itself.
        let env = bare_env();
        eval_str("(defmacro capture (x) `(quote ,x))", env.clone()).unwrap();
        assert_eq!(
            eval_str("(capture boom)", env).unwrap(),
            LispExp::symbol("boom".into())
        );
    }

    #[test]
    fn defmacro_expansion_runs_in_the_callers_environment() {
        let env = bare_env();
        eval_str("(defmacro get-caller-x () 'x)", env.clone()).unwrap();
        assert_eq!(
            eval_str("(let ((x 7)) (get-caller-x))", env).unwrap(),
            LispExp::number(7.0)
        );
    }

    #[test]
    fn defmacro_can_build_control_flow() {
        let env = bare_env();
        eval_str(
            "(defmacro my-if (test then else) `(if ,test ,then ,else))",
            env.clone(),
        )
        .unwrap();
        assert_eq!(
            eval_str("(my-if t 'yes 'no)", env.clone()).unwrap(),
            LispExp::symbol("yes".into())
        );
        assert_eq!(
            eval_str("(my-if nil 'yes 'no)", env).unwrap(),
            LispExp::symbol("no".into())
        );
    }

    #[test]
    fn defmacro_checks_arity() {
        let env = bare_env();
        eval_str("(defmacro one-arg (x) x)", env.clone()).unwrap();
        assert_eq!(
            eval_str("(one-arg 1 2)", env).unwrap_err(),
            EvalError::WrongNumberOfArguments {
                expected: 1,
                got: 2
            }
        );
    }

    #[test]
    fn backquote_with_no_unquotes_behaves_like_quote() {
        assert_eq!(
            eval_ok("`(1 2 3)"),
            LispExp::list(vec![
                LispExp::number(1.0),
                LispExp::number(2.0),
                LispExp::number(3.0),
            ])
        );
    }

    #[test]
    fn unquote_splices_a_single_evaluated_value() {
        assert_eq!(
            eval_ok("`(1 ,(+ 1 1) 3)"),
            LispExp::list(vec![
                LispExp::number(1.0),
                LispExp::number(2.0),
                LispExp::number(3.0),
            ])
        );
    }

    #[test]
    fn unquote_splicing_splices_every_element_of_an_evaluated_list() {
        assert_eq!(
            eval_ok("`(1 ,@(cons 2 (cons 3 nil)) 4)"),
            LispExp::list(vec![
                LispExp::number(1.0),
                LispExp::number(2.0),
                LispExp::number(3.0),
                LispExp::number(4.0),
            ])
        );
    }

    #[test]
    fn backquote_recurses_into_nested_lists_and_vectors() {
        assert_eq!(
            eval_ok("`(a (b ,(+ 1 2)) [c ,(+ 3 4)])"),
            LispExp::list(vec![
                LispExp::symbol("a".into()),
                LispExp::list(vec![LispExp::symbol("b".into()), LispExp::number(3.0)]),
                LispExp::vec(vec![LispExp::symbol("c".into()), LispExp::number(7.0)]),
            ])
        );
    }

    #[test]
    fn backquote_requires_exactly_one_argument() {
        assert_eq!(eval_err("(backquote)"), EvalError::BackquoteNotOneArgument);
    }

    // ===========================================================
    // Closures / funarg (lexical scoping sanity check with zero
    // primitives beyond the local `+`)
    // ===========================================================

    #[test]
    fn lambdas_close_over_their_defining_environment() {
        // `(make-adder 5)` is itself a compound head, so the evaluator
        // evaluates it down to a `Lambda` first and then invokes that
        // directly -- exercising closures without needing `funcall`
        // (that primitive lives in `base_env.rs`, not `lisp.rs`).
        let script = "
            (defun make-adder (n) (lambda (x) (+ x n)))
            ((make-adder 5) 37)
        ";
        assert_eq!(eval_ok(script), LispExp::number(42.0));
    }

    #[test]
    fn each_closure_call_gets_a_fresh_frame() {
        // Calling `make-adder` twice must not let the two closures'
        // captured `n` bleed into each other.
        let script = "
            (defun make-adder (n) (lambda (x) (+ x n)))
            (cons ((make-adder 1) 10) (cons ((make-adder 2) 10) nil))
        ";
        assert_eq!(
            eval_ok(script),
            LispExp::list(vec![LispExp::number(11.0), LispExp::number(12.0)])
        );
    }

    // ===========================================================
    // &optional / &rest parameter lists (defun, lambda, and defmacro all
    // share the same parsing/binding code, so a handful of tests on each
    // form is enough to exercise it once rather than three times).
    // ===========================================================

    #[test]
    fn optional_params_default_to_nil_when_omitted() {
        let script = "(defun greet (name &optional greeting) (cons name (cons greeting nil)))";
        assert_eq!(
            eval_ok(&format!("{script} (greet \"a\")")),
            LispExp::list(vec![LispExp::string("a".into()), LispExp::nil()])
        );
        assert_eq!(
            eval_ok(&format!("{script} (greet \"a\" \"hi\")")),
            LispExp::list(vec![
                LispExp::string("a".into()),
                LispExp::string("hi".into())
            ])
        );
    }

    #[test]
    fn rest_param_collects_every_remaining_argument_into_a_list() {
        assert_eq!(
            eval_ok("(defun args-of (a &rest more) more) (args-of 1 2 3 4)"),
            LispExp::list(vec![
                LispExp::number(2.0),
                LispExp::number(3.0),
                LispExp::number(4.0),
            ])
        );
        // No extra arguments -> the rest param is bound to an empty list,
        // not left unbound. `LispExp::list(vec![])`, not `LispExp::nil()`:
        // both are "falsy", but they're different representations of an
        // empty list under this VM's structural equality.
        assert_eq!(
            eval_ok("(defun args-of (a &rest more) more) (args-of 1)"),
            LispExp::list(vec![])
        );
    }

    #[test]
    fn required_optional_and_rest_combine_in_one_param_list() {
        let script = "
            (defun f (a &optional b &rest c) (cons a (cons b c)))
        ";
        assert_eq!(
            eval_ok(&format!("{script} (f 1)")),
            LispExp::list(vec![LispExp::number(1.0), LispExp::nil()])
        );
        assert_eq!(
            eval_ok(&format!("{script} (f 1 2 3 4)")),
            LispExp::list(vec![
                LispExp::number(1.0),
                LispExp::number(2.0),
                LispExp::number(3.0),
                LispExp::number(4.0),
            ])
        );
    }

    #[test]
    fn too_few_required_arguments_is_still_an_arity_error() {
        assert_eq!(
            eval_err("(defun f (a &optional b) a) (f)"),
            EvalError::WrongNumberOfArguments {
                expected: 1,
                got: 0
            }
        );
    }

    #[test]
    fn too_many_arguments_without_a_rest_param_is_an_arity_error() {
        assert_eq!(
            eval_err("(defun f (a &optional b) a) (f 1 2 3)"),
            EvalError::WrongNumberOfArguments {
                expected: 2,
                got: 3
            }
        );
    }

    #[test]
    fn immediately_invoked_lambdas_respect_optional_and_rest_too() {
        // Exercises the direct-lambda-call path (no `defun`/symbol lookup
        // involved), not just the named-function-call path.
        assert_eq!(eval_ok("((lambda (&optional x) x))"), LispExp::nil());
        assert_eq!(
            eval_ok("((lambda (&rest xs) xs) 1 2)"),
            LispExp::list(vec![LispExp::number(1.0), LispExp::number(2.0)])
        );
    }

    #[test]
    fn defmacro_supports_optional_and_rest_params_too() {
        // Macros go through the same parser/binder as defun/lambda. `list`
        // isn't in this file's deliberately bare environment (see the
        // module doc comment), so build the quoted form with `cons` alone:
        // `items` is already bound to the raw, unevaluated call-site args
        // as a list, so `(cons 'quote (cons items nil))` expands to
        // `(quote (1 2 3))`, which evaluates back to the plain list.
        let script = "
            (defmacro my-list (&rest items) (cons 'quote (cons items nil)))
        ";
        assert_eq!(
            eval_ok(&format!("{script} (my-list 1 2 3)")),
            LispExp::list(vec![
                LispExp::number(1.0),
                LispExp::number(2.0),
                LispExp::number(3.0),
            ])
        );
    }

    #[test]
    fn rest_marker_requires_exactly_one_name_after_it() {
        assert_eq!(
            eval_err("(defun f (&rest) 1)"),
            EvalError::DefunRestMustHaveExactlyOneParam
        );
        assert_eq!(
            eval_err("(defun f (&rest a b) 1)"),
            EvalError::DefunRestMustHaveExactlyOneParam
        );
    }

    #[test]
    fn param_markers_reject_nonsensical_placement() {
        // &optional after &rest.
        assert_eq!(
            eval_err("(defun f (a &rest b &optional c) 1)"),
            EvalError::DefunMisplacedParamMarker
        );
        // &optional appearing twice.
        assert_eq!(
            eval_err("(defun f (a &optional b &optional c) 1)"),
            EvalError::DefunMisplacedParamMarker
        );
    }
}
