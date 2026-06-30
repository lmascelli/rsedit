#[cfg(test)]
mod test {
    use crate::lisp::{Env, EvalError, LispContext, LispExp, eval};
    use std::{collections::HashMap, sync::Arc};

    fn setup_env() -> (std::sync::Arc<Env<()>>, ()) {
        let env = Env::new_root();
        env.set_variable("nil".into(), LispExp::list(vec![]));
        env.set_variable("t".into(), LispExp::number(1.0));
        (env, ())
    }
    // ==========================================
    // EVAL TESTS
    // ==========================================

    fn get_var<T: LispContext>(env: &Env<T>, name: &str) -> Option<LispExp<T>>
    where
        T: Clone,
    {
        env.get_variable(name)
    }

    fn get_func<T: LispContext>(env: &Env<T>, name: &str) -> Option<LispExp<T>>
    where
        T: Clone,
    {
        env.get_function(name)
    }

    fn is_nil<T: LispContext>(exp: &LispExp<T>) -> bool
    where
        T: PartialEq,
    {
        *exp == LispExp::list(vec![]) || *exp == LispExp::symbol("nil".into())
    }

    #[test]
    fn test_lisp_2_namespaces() {
        let (env, mut ctx) = setup_env();

        // Bind the variable 'buffer' to the string "main.txt"
        // (setq buffer "main.txt")
        env.set_variable("buffer".into(), LispExp::string("main.txt".into()));

        // NEW DAY 2 CHANGE: Mock the function directly using the optimal Lambda struct
        let mock_func = LispExp::Lambda(Arc::new(crate::lisp::Lambda {
            params: vec![], // No parameters
            body: LispExp::number(42.0),
            env: env.clone(), // Capture current environment
        }));
        env.set_function("buffer".into(), mock_func);

        // Test 1: Evaluating the symbol alone yields the variable slot
        let var_eval = eval(&LispExp::symbol("buffer".into()), env.clone(), &mut ctx).unwrap();
        assert_eq!(var_eval, LispExp::string("main.txt".into()));

        // Test 2: Evaluating the symbol as a function call executes the function slot
        // Execution of (buffer)
        let call_exp = LispExp::list(vec![LispExp::symbol("buffer".into())]);
        let func_eval = eval(&call_exp, env.clone(), &mut ctx).unwrap();
        assert_eq!(func_eval, LispExp::number(42.0));
    }

    #[test]
    fn test_dynamic_variable_lookup() {
        let (global, mut _ctx) = setup_env();
        global.set_variable("x".into(), LispExp::number(10.0));
        global.set_variable("y".into(), LispExp::number(20.0));

        let local = Env::new_child(&global);
        local.set_variable("x".into(), LispExp::number(99.0)); // Shadows global x

        // Local 'x' shadows global 'x'
        assert_eq!(get_var(&local, "x"), Some(LispExp::number(99.0)));
        // Local falls back to global for 'y'
        assert_eq!(get_var(&local, "y"), Some(LispExp::number(20.0)));
        // Unbound variable
        assert_eq!(get_var(&local, "z"), None);
    }

    #[test]
    fn test_dynamic_function_lookup() {
        let (mut global, mut _ctx) = setup_env();
        global.set_function("add".into(), LispExp::symbol("built-in-add".into()));

        let local = Env::new_child(&mut global as _);

        // Should find 'add' in the parent's function namespace
        assert_eq!(
            get_func(&local, "add"),
            Some(LispExp::symbol("built-in-add".into()))
        );
    }

    #[test]
    fn test_elisp_nil_truthiness() {
        // In Elisp, the symbol "nil" and the empty list () are false.
        assert!(is_nil(&LispExp::<()>::symbol("nil".into())));
        assert!(is_nil(&LispExp::<()>::list(vec![])));

        // Everything else is true (not nil)
        assert!(!is_nil(&LispExp::<()>::symbol("t".into())));
        assert!(!is_nil(&LispExp::<()>::number(0.0)));
        assert!(!is_nil(&LispExp::<()>::string("".into())));
        assert!(!is_nil(&LispExp::<()>::vec(vec![]))); // Empty vectors are true in Elisp!
    }

    #[test]
    fn test_eval_setq() {
        let (env, mut ctx) = setup_env();
        // (setq a 42.0)
        let setq_exp = LispExp::list(vec![
            LispExp::symbol("setq".into()),
            LispExp::symbol("a".into()),
            LispExp::number(42.0),
        ]);

        let result = eval(&setq_exp, env.clone(), &mut ctx).unwrap();

        assert_eq!(result, LispExp::Number(42.0));
        assert_eq!(env.get_variable("a"), Some(LispExp::Number(42.0)));
    }

    #[test]
    fn test_eval_defun() {
        let (env, mut ctx) = setup_env();

        // (defun my-func () "hello")
        let defun_exp = LispExp::list(vec![
            LispExp::symbol("defun".into()),
            LispExp::symbol("my-func".into()),
            LispExp::list(vec![]),
            LispExp::string("hello".into()),
        ]);

        eval(&defun_exp, env.clone(), &mut ctx).unwrap();

        // In Elisp, defun binds the symbol's function slot to a lambda representation
        let stored_func = env
            .get_function("my-func")
            .expect("Function should be bound");

        if let LispExp::Lambda(_) = stored_func {
        } else {
            panic!("defun did not store a lambda");
        }
    }

    #[test]
    fn test_eval_if() {
        let (env, mut ctx) = setup_env();

        // (if nil 1.0 2.0) -> should evaluate to 2.0
        let if_false = LispExp::list(vec![
            LispExp::symbol("if".into()),
            LispExp::symbol("nil".into()),
            LispExp::number(1.0),
            LispExp::number(2.0),
        ]);
        assert_eq!(
            eval(&if_false, env.clone(), &mut ctx).unwrap(),
            LispExp::number(2.0)
        );

        // (if "truthy" 1.0 2.0) -> should evaluate to 1.0
        let if_true = LispExp::list(vec![
            LispExp::symbol("if".into()),
            LispExp::string("truthy".into()),
            LispExp::number(1.0),
            LispExp::number(2.0),
        ]);
        assert_eq!(
            eval(&if_true, env.clone(), &mut ctx).unwrap(),
            LispExp::number(1.0)
        );
    }

    #[test]
    fn test_error_void_variable() {
        let (env, mut ctx) = setup_env();

        let exp = LispExp::symbol("undefined-var".into());
        let result = eval(&exp, env.clone(), &mut ctx);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            EvalError::UnboundVariable("undefined-var".into())
        );
    }

    #[test]
    fn test_error_void_function() {
        let (env, mut ctx) = setup_env();

        // (missing-func 1 2)
        let exp = LispExp::list(vec![
            LispExp::symbol("missing-func".into()),
            LispExp::number(1.0),
        ]);
        let result = eval(&exp, env.clone(), &mut ctx);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            EvalError::UndefinedFunction("missing-func".into())
        );
    }

    #[test]
    fn test_error_calling_variable_as_function() {
        let (env, mut ctx) = setup_env();

        // Bind 'x' in the variable namespace only
        env.set_variable("x".into(), LispExp::number(42.0));

        // Try to call it: (x)
        let exp = LispExp::list(vec![LispExp::symbol("x".into())]);
        let result = eval(&exp, env.clone(), &mut ctx);

        // It should fail because 'x' is not in the function namespace!
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            EvalError::UndefinedFunction("x".into())
        );
    }

    #[test]
    fn test_error_invalid_function_call() {
        let (env, mut ctx) = setup_env();

        // (42 "hello") -> The first element is not a symbol!
        let exp = LispExp::list(vec![LispExp::number(42.0), LispExp::string("hello".into())]);
        let result = eval(&exp, env.clone(), &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::UnvalidFunctionCall);
    }

    #[test]
    fn test_error_setq_non_symbol() {
        let (env, mut ctx) = setup_env();

        // (setq 42 "value") -> 42 is not a valid variable name
        let exp = LispExp::list(vec![
            LispExp::symbol("setq".into()),
            LispExp::number(42.0),
            LispExp::string("value".into()),
        ]);
        let result = eval(&exp, env.clone(), &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::SetqSymbolRequired);
    }

    #[test]
    fn test_error_defun_non_symbol() {
        let (env, mut ctx) = setup_env();

        // (defun "my-func" () 1) -> Function name must be a symbol, not a string
        let exp = LispExp::list(vec![
            LispExp::symbol("defun".into()),
            LispExp::string("my-func".into()),
            LispExp::list(vec![]),
            LispExp::number(1.0),
        ]);
        let result = eval(&exp, env.clone(), &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::DefunNameMustBeASymbol);
    }

    #[test]
    fn test_error_if_missing_condition() {
        let (env, mut ctx) = setup_env();

        // (if) -> Missing condition and branches
        let exp = LispExp::list(vec![LispExp::symbol("if".into())]);
        let result = eval(&exp, env.clone(), &mut ctx);

        // Your evaluator needs bounds checking for `args[0]` to pass this without panicking!
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::IfNoConditionProvided);
    }

    #[test]
    fn test_eval_robustness_if_missing_branches() {
        let (env, mut ctx) = setup_env();

        // (if t) -> Has condition, but missing the true_branch.
        // Currently panics at `args[1].clone()`
        let exp = LispExp::list(vec![
            LispExp::symbol("if".into()),
            LispExp::symbol("t".into()),
        ]);
        let _ = eval(&exp, env.clone(), &mut ctx);
    }

    #[test]
    fn test_eval_robustness_setq_missing_value() {
        let (env, mut ctx) = setup_env();

        // (setq a) -> Missing the value to set!
        // Currently panics at `eval(&args[1], env, ctx)?`
        let exp = LispExp::list(vec![
            LispExp::symbol("setq".into()),
            LispExp::symbol("a".into()),
        ]);
        let _ = eval(&exp, env.clone(), &mut ctx);
    }

    #[test]
    fn test_eval_robustness_defun_missing_args() {
        let (env, mut ctx) = setup_env();

        // (defun my-func) -> Missing params list and body!
        // Currently panics at `args[1].clone()`
        let exp = LispExp::list(vec![
            LispExp::symbol("defun".into()),
            LispExp::symbol("my-func".into()),
        ]);
        let _ = eval(&exp, env.clone(), &mut ctx);
    }

    #[test]
    fn test_eval_robustness_malformed_lambda_execution() {
        let (env, mut ctx) = setup_env();

        // Register a completely broken lambda: (lambda (x)) -> Missing the body!
        let bad_lambda = LispExp::list(vec![
            LispExp::symbol("lambda".into()),
            LispExp::list(vec![LispExp::symbol("x".into())]),
        ]);
        env.set_function("broken-func".into(), bad_lambda);

        // Execute it: (broken-func 10)
        let exp = LispExp::list(vec![
            LispExp::symbol("broken-func".into()),
            LispExp::number(10.0),
        ]);

        let result = eval(&exp, env.clone(), &mut ctx);

        // Your code safely catches this with `if lambda_ast.len() != 3`!
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::UncorrectFunctionDefinition);
    }

    #[test]
    fn test_eval_robustness_empty_vector_and_map() {
        let (env, mut ctx) = setup_env();

        // vector and Map should evaluate their internal items.
        // Testing that empty ones safely evaluate to themselves.
        let empty_vec = LispExp::vec(vec![]);
        assert_eq!(
            eval(&empty_vec, env.clone(), &mut ctx).unwrap(),
            LispExp::vec(vec![])
        );

        // Maps are currently unimplemented in your `eval` match block (`_ => todo!()`).
        // If you run this test, it will hit the `todo!()` panic.
        // let empty_map = LispExp::Map(std::collections::HashMap::new());
        // eval(&empty_map, env.clone(), &mut ctx).unwrap();
    }

    // ==========================================
    // WITH GENERIC CTX EVAL TESTS
    // ==========================================

    // 1. Define a dummy Host Context to test the generic bridge
    #[derive(Clone, Debug, PartialEq)]
    struct TestHost {
        pub state_changes: usize,
    }

    impl LispContext for TestHost {
        fn consume_fuel(&mut self, amount: u32) -> Result<(), EvalError> {
            Ok(())
        }

        fn log_diagnostic(&mut self, msg: &str) {}
    }

    // 2. A mock native primitive that mutates the host context
    fn native_increment_state(
        _args: &[LispExp<TestHost>],
        ctx: &mut TestHost,
    ) -> Result<LispExp<TestHost>, EvalError> {
        ctx.state_changes += 1;
        Ok(LispExp::symbol("nil".into()))
    }

    // 3. A mock native primitive for addition
    fn native_add(
        args: &[LispExp<TestHost>],
        _ctx: &mut TestHost,
    ) -> Result<LispExp<TestHost>, EvalError> {
        let mut sum = 0.0;
        for arg in args {
            if let LispExp::Number(n) = arg {
                sum += n;
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "number".into(),
                    got: format!("{:?}", arg),
                });
            }
        }
        Ok(LispExp::number(sum))
    }

    // Helper to create a fresh environment with primitives loaded
    fn setup_env_test() -> Arc<Env<TestHost>> {
        let env = Env::new_root();
        env.set_variable("nil".into(), LispExp::list(vec![]));
        env.set_variable("t".into(), LispExp::number(1.0));
        env.set_function("+".into(), LispExp::Primitive(native_add));
        env.set_function(
            "inc-state".into(),
            LispExp::Primitive(native_increment_state),
        );
        env
    }

    #[test]
    fn test_elisp_if_truthiness() {
        let env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // Helper macro to generate an `if` AST
        let make_if = |cond: LispExp<TestHost>| -> LispExp<TestHost> {
            LispExp::list(vec![
                LispExp::symbol("if".into()),
                cond,
                LispExp::string("true_branch".into()),
                LispExp::string("false_branch".into()),
            ])
        };

        let true_res = LispExp::string("true_branch".into());
        let false_res = LispExp::string("false_branch".into());

        // 1. `nil` is false
        let if_nil = make_if(LispExp::symbol("nil".into()));
        assert_eq!(eval(&if_nil, env.clone(), &mut ctx).unwrap(), false_res);

        // 2. Empty list `()` is false
        let if_empty = make_if(LispExp::list(vec![]));
        assert_eq!(eval(&if_empty, env.clone(), &mut ctx).unwrap(), false_res);

        // 3. Everything else is true (e.g., number 0.0, empty string, "t")
        assert_eq!(
            eval(&make_if(LispExp::number(0.0)), env.clone(), &mut ctx).unwrap(),
            true_res
        );
        assert_eq!(
            eval(&make_if(LispExp::string("".into())), env.clone(), &mut ctx).unwrap(),
            true_res
        );
        assert_eq!(
            eval(&make_if(LispExp::symbol("t".into())), env.clone(), &mut ctx).unwrap(),
            true_res
        );
    }

    #[test]
    fn test_lisp_2_namespace_isolation() {
        let env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // AST: (setq log "var-data")
        let setq_exp = LispExp::list(vec![
            LispExp::symbol("setq".into()),
            LispExp::symbol("log".into()),
            LispExp::string("var-data".into()),
        ]);
        eval(&setq_exp, env.clone(), &mut ctx).unwrap();

        // Bind 'log' in the function namespace to a lambda
        let mock_lambda = LispExp::lambda(crate::lisp::Lambda {
            params: vec![],
            body: LispExp::number(99.0),
            env: env.clone(),
        });
        env.set_function("log".into(), mock_lambda);

        // 1. Evaluate as variable: log -> "var-data"
        let var_eval = eval(&LispExp::symbol("log".into()), env.clone(), &mut ctx).unwrap();
        assert_eq!(var_eval, LispExp::string("var-data".into()));

        // 2. Evaluate as function: (log) -> 99.0
        let func_eval = eval(
            &LispExp::list(vec![LispExp::symbol("log".into())]),
            env.clone(),
            &mut ctx,
        )
        .unwrap();
        assert_eq!(func_eval, LispExp::number(99.0));
    }

    #[test]
    fn test_eval_setq_multiple() {
        let env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // (setq a 1.0 b (+ 1.0 2.0))
        let setq_exp = LispExp::list(vec![
            LispExp::symbol("setq".into()),
            LispExp::symbol("a".into()),
            LispExp::number(1.0),
            LispExp::symbol("b".into()),
            LispExp::list(vec![
                LispExp::symbol("+".into()),
                LispExp::number(1.0),
                LispExp::number(2.0),
            ]),
        ]);

        let result = eval(&setq_exp, env.clone(), &mut ctx).unwrap();

        // Returns last assigned value
        assert_eq!(result, LispExp::number(3.0));
        assert_eq!(env.get_variable("a").unwrap(), LispExp::Number(1.0));
        assert_eq!(env.get_variable("b").unwrap(), LispExp::Number(3.0));
    }

    #[test]
    fn test_lambda_argument_binding() {
        let env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // (defun add-custom (x y) (+ x y))
        let defun_exp = LispExp::list(vec![
            LispExp::symbol("defun".into()),
            LispExp::symbol("add-custom".into()),
            LispExp::list(vec![
                LispExp::symbol("x".into()),
                LispExp::symbol("y".into()),
            ]),
            LispExp::list(vec![
                LispExp::symbol("+".into()),
                LispExp::symbol("x".into()),
                LispExp::symbol("y".into()),
            ]),
        ]);
        eval(&defun_exp, env.clone(), &mut ctx).unwrap();

        // (add-custom 10.0 20.0)
        let call_exp = LispExp::list(vec![
            LispExp::symbol("add-custom".into()),
            LispExp::number(10.0),
            LispExp::number(20.0),
        ]);
        eprintln!("{:?}", call_exp);
        let result = eval(&call_exp, env.clone(), &mut ctx).unwrap();

        assert_eq!(result, LispExp::number(30.0));

        // Ensure parameters didn't leak into global env
        assert!(env.get_variable("x").is_none());
    }

    #[test]
    fn test_generic_host_mutation() {
        let env = setup_env_test();

        // Initialize our "Editor State" equivalent
        let mut ctx = TestHost { state_changes: 0 };

        // AST: (inc-state)
        let call_mutation = LispExp::list(vec![LispExp::symbol("inc-state".into())]);

        // Call it three times
        eval(&call_mutation, env.clone(), &mut ctx).unwrap();
        eval(&call_mutation, env.clone(), &mut ctx).unwrap();
        eval(&call_mutation, env.clone(), &mut ctx).unwrap();

        // The Rust host context should have tracked the changes natively!
        assert_eq!(ctx.state_changes, 3);
    }

    // Mock Host context to assist evaluating side effects
    #[derive(Clone, Debug, PartialEq)]
    struct MockHost {
        pub tracker: f64,
    }

    impl LispContext for MockHost {
        fn consume_fuel(&mut self, amount: u32) -> Result<(), EvalError> {
            Ok(())
        }

        fn log_diagnostic(&mut self, msg: &str) {}
    }

    fn setup_interpreter_env() -> (Arc<Env<MockHost>>, MockHost) {
        let env = Env::new_root();
        // Bind a global state mutating tool

        env.set_function(
            "bump".into(),
            LispExp::Primitive(|_args: &[LispExp<MockHost>], ctx| {
                ctx.tracker += 1.0;
                Ok(LispExp::number(ctx.tracker))
            }),
        );

        (env, MockHost { tracker: 0.0 })
    }

    #[test]
    fn test_progn_execution_and_side_effects() {
        let (env, mut ctx) = setup_interpreter_env();

        // (progn (bump) (bump) 99.0) -> changes state twice, returns last value
        let exp = LispExp::list(vec![
            LispExp::symbol("progn".into()),
            LispExp::list(vec![LispExp::symbol("bump".into())]),
            LispExp::list(vec![LispExp::symbol("bump".into())]),
            LispExp::number(99.0),
        ]);

        let result = eval(&exp, env.clone(), &mut ctx).unwrap();
        assert_eq!(result, LispExp::number(99.0));
        assert_eq!(ctx.tracker, 2.0);

        // Empty progn should evaluate to nil symbol safely
        let empty_progn = LispExp::list(vec![LispExp::symbol("progn".into())]);
        assert_eq!(
            eval(&empty_progn, env.clone(), &mut ctx).unwrap(),
            LispExp::symbol("nil".into())
        )
    }

    #[test]
    fn test_let_scoping_and_shadowing() {
        let (env, mut ctx) = setup_interpreter_env();
        // Inject global variable x = 10.0
        env.set_variable("x".into(), LispExp::number(10.0));

        // AST representation of:
        // (let ((x 20.0) (y 30.0)) (+ x y))
        // Assuming native primitives like '+' are mapped
        env.set_function(
            "+".into(),
            LispExp::Primitive(|args, _| {
                if let (LispExp::Number(a), LispExp::Number(b)) = (&args[0], &args[1]) {
                    Ok(LispExp::number(a + b))
                } else {
                    Err(EvalError::UnvalidFunctionCall)
                }
            }),
        );

        let let_exp = LispExp::list(vec![
            LispExp::symbol("let".into()),
            LispExp::list(vec![
                LispExp::list(vec![LispExp::symbol("x".into()), LispExp::number(20.0)]),
                LispExp::list(vec![LispExp::symbol("y".into()), LispExp::number(30.0)]),
            ]),
            LispExp::list(vec![
                LispExp::symbol("+".into()),
                LispExp::symbol("x".into()),
                LispExp::symbol("y".into()),
            ]),
        ]);

        let result = eval(&let_exp, env.clone(), &mut ctx).unwrap();
        assert_eq!(result, LispExp::number(50.0));

        // CRITICAL CHECK: Global variable namespace must be untouched (Lexical isolation)
        assert_eq!(env.get_variable("x"), Some(LispExp::number(10.0)));
        assert!(env.get_variable("y").is_none());
    }

    #[test]
    fn test_let_malformed_syntax_errors() {
        let (env, mut ctx) = setup_interpreter_env();

        // 1. Missing binding block entirely
        let no_bindings = LispExp::list(vec![LispExp::symbol("let".into())]);
        assert_eq!(
            eval(&no_bindings, env.clone(), &mut ctx).unwrap_err(),
            EvalError::LetNoBindingsProvided
        );

        // 2. Binding block isn't a collection list: (let x body)
        let invalid_list = LispExp::list(vec![
            LispExp::symbol("let".into()),
            LispExp::symbol("x".into()),
            LispExp::number(1.0),
        ]);
        assert_eq!(
            eval(&invalid_list, env.clone(), &mut ctx).unwrap_err(),
            EvalError::LetUnvalidBindingList
        );

        // 3. Malformed tuple inside binding array: (let ((x 1 2)) body)
        let broken_tuple = LispExp::list(vec![
            LispExp::symbol("let".into()),
            LispExp::list(vec![LispExp::list(vec![
                LispExp::symbol("x".into()),
                LispExp::number(1.0),
                LispExp::number(2.0),
            ])]),
            LispExp::symbol("x".into()),
        ]);
        assert_eq!(
            eval(&broken_tuple, env.clone(), &mut ctx).unwrap_err(),
            EvalError::LetUnvalidBindingAt(0)
        );
    }

    #[test]
    fn test_map_evaluation_and_expression_resolution() {
        let (env, mut ctx) = setup_interpreter_env();
        env.set_variable("factor".into(), LispExp::number(5.0));

        // A map structure: { "computed-val" (progn (bump) factor) }
        let mut input_map = HashMap::new();
        input_map.insert(
            "computed-val".to_string(),
            LispExp::list(vec![
                LispExp::symbol("progn".into()),
                LispExp::list(vec![LispExp::symbol("bump".into())]),
                LispExp::symbol("factor".into()),
            ]),
        );

        let map_exp = LispExp::map(input_map);
        let evaluated = eval(&map_exp, env.clone(), &mut ctx).unwrap();

        if let LispExp::Map(output_map) = evaluated {
            assert_eq!(output_map.get("computed-val"), Some(&LispExp::number(5.0)));
            assert_eq!(ctx.tracker, 1.0); // Verifies expressions inner-eval inside Maps
        } else {
            panic!("Evaluation should retain structural map wrapper type");
        }
    }
}
