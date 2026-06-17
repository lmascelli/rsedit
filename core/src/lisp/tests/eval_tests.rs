#[cfg(test)]
mod test {
    use crate::lisp::{Env, EvalError, LispExp, eval};
    use std::collections::HashMap;

    fn setup_env() -> (Env<()>, ()) {
        (Env::new(), ())
    }
    // ==========================================
    // EVAL TESTS
    // ==========================================

    fn get_var<T>(env: &Env<T>, name: &str) -> Option<LispExp<T>>
    where
        T: Clone,
    {
        env.get_var(name)
    }

    fn get_func<T>(env: &Env<T>, name: &str) -> Option<LispExp<T>>
    where
        T: Clone,
    {
        env.get_func(name)
    }

    fn is_nil<T>(exp: &LispExp<T>) -> bool
    where
        T: PartialEq,
    {
        *exp == LispExp::List(vec![]) || *exp == LispExp::Symbol("nil".into())
    }

    #[test]
    fn test_lisp_2_namespaces() {
        let (mut env, mut ctx) = setup_env();

        // Bind the variable 'buffer' to the string "main.txt"
        // (setq buffer "main.txt")
        env.variables
            .insert("buffer".into(), LispExp::String("main.txt".into()));

        // Bind the function 'buffer' to a mock function that returns a number
        let mock_func = LispExp::List(vec![
            LispExp::Symbol("lambda".into()),
            LispExp::List(vec![]),
            LispExp::Number(42.0),
        ]);
        env.functions.insert("buffer".into(), mock_func);

        // Test 1: Evaluating the symbol alone yields the variable slot
        let var_eval = eval(&LispExp::Symbol("buffer".into()), &mut env, &mut ctx).unwrap();
        assert_eq!(var_eval, LispExp::String("main.txt".into()));

        // Test 2: Evaluating the symbol as a function call executes the function slot
        // Execution of (buffer)
        let call_exp = LispExp::List(vec![LispExp::Symbol("buffer".into())]);
        let func_eval = eval(&call_exp, &mut env, &mut ctx).unwrap();
        assert_eq!(func_eval, LispExp::Number(42.0));
    }

    #[test]
    fn test_dynamic_variable_lookup() {
        let (mut global, mut _ctx) = setup_env();
        global.variables.insert("x".into(), LispExp::Number(10.0));
        global.variables.insert("y".into(), LispExp::Number(20.0));

        let mut local = Env::extend(&mut global as _);
        local.variables.insert("x".into(), LispExp::Number(99.0)); // Shadows global x

        // Local 'x' shadows global 'x'
        assert_eq!(get_var(&local, "x"), Some(LispExp::Number(99.0)));
        // Local falls back to global for 'y'
        assert_eq!(get_var(&local, "y"), Some(LispExp::Number(20.0)));
        // Unbound variable
        assert_eq!(get_var(&local, "z"), None);
    }

    #[test]
    fn test_dynamic_function_lookup() {
        let (mut global, mut _ctx) = setup_env();
        global
            .functions
            .insert("add".into(), LispExp::Symbol("built-in-add".into()));

        let local = Env::extend(&mut global as _);

        // Should find 'add' in the parent's function namespace
        assert_eq!(
            get_func(&local, "add"),
            Some(LispExp::Symbol("built-in-add".into()))
        );
    }

    #[test]
    fn test_elisp_nil_truthiness() {
        // In Elisp, the symbol "nil" and the empty list () are false.
        assert!(is_nil(&LispExp::<()>::Symbol("nil".into())));
        assert!(is_nil(&LispExp::<()>::List(vec![])));

        // Everything else is true (not nil)
        assert!(!is_nil(&LispExp::<()>::Symbol("t".into())));
        assert!(!is_nil(&LispExp::<()>::Number(0.0)));
        assert!(!is_nil(&LispExp::<()>::String("".into())));
        assert!(!is_nil(&LispExp::<()>::Vector(vec![]))); // Empty vectors are true in Elisp!
    }

    #[test]
    fn test_eval_setq() {
        let (mut env, mut ctx) = setup_env();
        // (setq a 42.0)
        let setq_exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Symbol("a".into()),
            LispExp::Number(42.0),
        ]);

        let result = eval(&setq_exp, &mut env, &mut ctx).unwrap();

        assert_eq!(result, LispExp::Number(42.0));
        assert_eq!(env.variables.get("a"), Some(&LispExp::Number(42.0)));
    }

    #[test]
    fn test_eval_defun() {
        let (mut env, mut ctx) = setup_env();

        // (defun my-func () "hello")
        let defun_exp = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::Symbol("my-func".into()),
            LispExp::List(vec![]),
            LispExp::String("hello".into()),
        ]);

        eval(&defun_exp, &mut env, &mut ctx).unwrap();

        // In Elisp, defun binds the symbol's function slot to a lambda representation
        let stored_func = env
            .functions
            .get("my-func")
            .expect("Function should be bound");

        if let LispExp::List(lambda_data) = stored_func {
            assert_eq!(lambda_data[0], LispExp::Symbol("lambda".into()));
            assert_eq!(lambda_data[1], LispExp::List(vec![]));
            assert_eq!(lambda_data[2], LispExp::String("hello".into()));
        } else {
            panic!("defun did not store a lambda list");
        }
    }

    #[test]
    fn test_eval_if() {
        let (mut env, mut ctx) = setup_env();

        // (if nil 1.0 2.0) -> should evaluate to 2.0
        let if_false = LispExp::List(vec![
            LispExp::Symbol("if".into()),
            LispExp::Symbol("nil".into()),
            LispExp::Number(1.0),
            LispExp::Number(2.0),
        ]);
        assert_eq!(
            eval(&if_false, &mut env, &mut ctx).unwrap(),
            LispExp::Number(2.0)
        );

        // (if "truthy" 1.0 2.0) -> should evaluate to 1.0
        let if_true = LispExp::List(vec![
            LispExp::Symbol("if".into()),
            LispExp::String("truthy".into()),
            LispExp::Number(1.0),
            LispExp::Number(2.0),
        ]);
        assert_eq!(
            eval(&if_true, &mut env, &mut ctx).unwrap(),
            LispExp::Number(1.0)
        );
    }

    #[test]
    fn test_error_void_variable() {
        let (mut env, mut ctx) = setup_env();

        let exp = LispExp::Symbol("undefined-var".into());
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            EvalError::UnboundVariable("undefined-var".into())
        );
    }

    #[test]
    fn test_error_void_function() {
        let (mut env, mut ctx) = setup_env();

        // (missing-func 1 2)
        let exp = LispExp::List(vec![
            LispExp::Symbol("missing-func".into()),
            LispExp::Number(1.0),
        ]);
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            EvalError::UndefinedFunction("missing-func".into())
        );
    }

    #[test]
    fn test_error_calling_variable_as_function() {
        let (mut env, mut ctx) = setup_env();

        // Bind 'x' in the variable namespace only
        env.variables.insert("x".into(), LispExp::Number(42.0));

        // Try to call it: (x)
        let exp = LispExp::List(vec![LispExp::Symbol("x".into())]);
        let result = eval(&exp, &mut env, &mut ctx);

        // It should fail because 'x' is not in the function namespace!
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            EvalError::UndefinedFunction("x".into())
        );
    }

    #[test]
    fn test_error_invalid_function_call() {
        let (mut env, mut ctx) = setup_env();

        // (42 "hello") -> The first element is not a symbol!
        let exp = LispExp::List(vec![LispExp::Number(42.0), LispExp::String("hello".into())]);
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::UnvalidFunctionCall);
    }

    #[test]
    fn test_error_setq_non_symbol() {
        let (mut env, mut ctx) = setup_env();

        // (setq 42 "value") -> 42 is not a valid variable name
        let exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Number(42.0),
            LispExp::String("value".into()),
        ]);
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::SetqSymbolRequired);
    }

    #[test]
    fn test_error_defun_non_symbol() {
        let (mut env, mut ctx) = setup_env();

        // (defun "my-func" () 1) -> Function name must be a symbol, not a string
        let exp = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::String("my-func".into()),
            LispExp::List(vec![]),
            LispExp::Number(1.0),
        ]);
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::DefunNameMustBeASymbol);
    }

    #[test]
    fn test_error_if_missing_condition() {
        let (mut env, mut ctx) = setup_env();

        // (if) -> Missing condition and branches
        let exp = LispExp::List(vec![LispExp::Symbol("if".into())]);
        let result = eval(&exp, &mut env, &mut ctx);

        // Your evaluator needs bounds checking for `args[0]` to pass this without panicking!
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::IfNoConditionProvided);
    }

    #[test]
    fn test_eval_robustness_if_missing_branches() {
        let (mut env, mut ctx) = setup_env();

        // (if t) -> Has condition, but missing the true_branch.
        // Currently panics at `args[1].clone()`
        let exp = LispExp::List(vec![
            LispExp::Symbol("if".into()),
            LispExp::Symbol("t".into()),
        ]);
        let _ = eval(&exp, &mut env, &mut ctx);
    }

    #[test]
    fn test_eval_robustness_setq_missing_value() {
        let (mut env, mut ctx) = setup_env();

        // (setq a) -> Missing the value to set!
        // Currently panics at `eval(&args[1], env, ctx)?`
        let exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Symbol("a".into()),
        ]);
        let _ = eval(&exp, &mut env, &mut ctx);
    }

    #[test]
    fn test_eval_robustness_defun_missing_args() {
        let (mut env, mut ctx) = setup_env();

        // (defun my-func) -> Missing params list and body!
        // Currently panics at `args[1].clone()`
        let exp = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::Symbol("my-func".into()),
        ]);
        let _ = eval(&exp, &mut env, &mut ctx);
    }

    #[test]
    fn test_eval_robustness_malformed_lambda_execution() {
        let (mut env, mut ctx) = setup_env();

        // Register a completely broken lambda: (lambda (x)) -> Missing the body!
        let bad_lambda = LispExp::List(vec![
            LispExp::Symbol("lambda".into()),
            LispExp::List(vec![LispExp::Symbol("x".into())]),
        ]);
        env.functions.insert("broken-func".into(), bad_lambda);

        // Execute it: (broken-func 10)
        let exp = LispExp::List(vec![
            LispExp::Symbol("broken-func".into()),
            LispExp::Number(10.0),
        ]);

        let result = eval(&exp, &mut env, &mut ctx);

        // Your code safely catches this with `if lambda_ast.len() != 3`!
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::UncorrectFunctionDefinition);
    }

    #[test]
    fn test_eval_robustness_lambda_params_not_a_list() {
        let (mut env, mut ctx) = setup_env();

        // Register a lambda where the parameters are a String instead of a List
        // e.g., (lambda "x" (+ 1 2))
        let bad_lambda = LispExp::List(vec![
            LispExp::Symbol("lambda".into()),
            LispExp::String("x".into()),
            LispExp::Number(3.0),
        ]);
        env.functions.insert("weird-func".into(), bad_lambda);

        // Execute it: (weird-func)
        let exp = LispExp::List(vec![LispExp::Symbol("weird-func".into())]);

        // Since `params` is not a list, your evaluator silently skips binding parameters
        // and just evaluates the body. We assert that it evaluates to 3.0 safely.
        // Note: In strict Elisp, defining a lambda without a param list is an error.
        let result = eval(&exp, &mut env, &mut ctx).unwrap();
        assert_eq!(result, LispExp::Number(3.0));
    }

    #[test]
    fn test_eval_robustness_empty_vector_and_map() {
        let (mut env, mut ctx) = setup_env();

        // Vector and Map should evaluate their internal items.
        // Testing that empty ones safely evaluate to themselves.
        let empty_vec = LispExp::Vector(vec![]);
        assert_eq!(
            eval(&empty_vec, &mut env, &mut ctx).unwrap(),
            LispExp::Vector(vec![])
        );

        // Maps are currently unimplemented in your `eval` match block (`_ => todo!()`).
        // If you run this test, it will hit the `todo!()` panic.
        // let empty_map = LispExp::Map(std::collections::HashMap::new());
        // eval(&empty_map, &mut env, &mut ctx).unwrap();
    }

    // ==========================================
    // WITH GENERIC CTX EVAL TESTS
    // ==========================================

    // 1. Define a dummy Host Context to test the generic bridge
    #[derive(Clone, Debug, PartialEq)]
    struct TestHost {
        pub state_changes: usize,
    }

    // 2. A mock native primitive that mutates the host context
    fn native_increment_state(
        _args: &[LispExp<TestHost>],
        ctx: &mut TestHost,
    ) -> Result<LispExp<TestHost>, EvalError> {
        ctx.state_changes += 1;
        Ok(LispExp::Symbol("nil".into()))
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
                    expected: "Number".into(),
                    got: format!("{:?}", arg),
                });
            }
        }
        Ok(LispExp::Number(sum))
    }

    // Helper to create a fresh environment with primitives loaded
    fn setup_env_test() -> Env<TestHost> {
        let mut env = Env::new();
        env.functions
            .insert("+".into(), LispExp::Primitive(native_add));
        env.functions.insert(
            "inc-state".into(),
            LispExp::Primitive(native_increment_state),
        );
        env
    }

    #[test]
    fn test_elisp_if_truthiness() {
        let mut env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // Helper macro to generate an `if` AST
        let make_if = |cond: LispExp<TestHost>| -> LispExp<TestHost> {
            LispExp::List(vec![
                LispExp::Symbol("if".into()),
                cond,
                LispExp::String("true_branch".into()),
                LispExp::String("false_branch".into()),
            ])
        };

        let true_res = LispExp::String("true_branch".into());
        let false_res = LispExp::String("false_branch".into());

        // 1. `nil` is false
        let if_nil = make_if(LispExp::Symbol("nil".into()));
        assert_eq!(eval(&if_nil, &mut env, &mut ctx).unwrap(), false_res);

        // 2. Empty list `()` is false
        let if_empty = make_if(LispExp::List(vec![]));
        assert_eq!(eval(&if_empty, &mut env, &mut ctx).unwrap(), false_res);

        // 3. Everything else is true (e.g., Number 0.0, empty string, "t")
        assert_eq!(
            eval(&make_if(LispExp::Number(0.0)), &mut env, &mut ctx).unwrap(),
            true_res
        );
        assert_eq!(
            eval(&make_if(LispExp::String("".into())), &mut env, &mut ctx).unwrap(),
            true_res
        );
        assert_eq!(
            eval(&make_if(LispExp::Symbol("t".into())), &mut env, &mut ctx).unwrap(),
            true_res
        );
    }

    #[test]
    fn test_lisp_2_namespace_isolation() {
        let mut env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // AST: (setq log "var-data")
        let setq_exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Symbol("log".into()),
            LispExp::String("var-data".into()),
        ]);
        eval(&setq_exp, &mut env, &mut ctx).unwrap();

        // Bind 'log' in the function namespace to a lambda
        let mock_lambda = LispExp::List(vec![
            LispExp::Symbol("lambda".into()),
            LispExp::List(vec![]),
            LispExp::Number(99.0),
        ]);
        env.functions.insert("log".into(), mock_lambda);

        // 1. Evaluate as variable: log -> "var-data"
        let var_eval = eval(&LispExp::Symbol("log".into()), &mut env, &mut ctx).unwrap();
        assert_eq!(var_eval, LispExp::String("var-data".into()));

        // 2. Evaluate as function: (log) -> 99.0
        let func_eval = eval(
            &LispExp::List(vec![LispExp::Symbol("log".into())]),
            &mut env,
            &mut ctx,
        )
        .unwrap();
        assert_eq!(func_eval, LispExp::Number(99.0));
    }

    #[test]
    fn test_dynamic_scoping_call_stack() {
        let mut global_env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // (defun get-x () x) -> Relies on 'x' being defined dynamically by caller
        let get_x_def = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::Symbol("get-x".into()),
            LispExp::List(vec![]),
            LispExp::Symbol("x".into()),
        ]);
        eval(&get_x_def, &mut global_env, &mut ctx).unwrap();

        // Create a caller stack frame where x = 100
        let mut caller_env = Env::extend(&mut global_env as *mut Env<TestHost>);
        caller_env
            .variables
            .insert("x".into(), LispExp::Number(100.0));

        // Evaluate (get-x) FROM the caller's environment
        let call_exp = LispExp::List(vec![LispExp::Symbol("get-x".into())]);
        let result = eval(&call_exp, &mut caller_env, &mut ctx).unwrap();

        // It must follow the stack to find 'x' in caller_env
        assert_eq!(result, LispExp::Number(100.0));
    }

    #[test]
    fn test_eval_setq_multiple() {
        let mut env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // (setq a 1.0 b (+ 1.0 2.0))
        let setq_exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Symbol("a".into()),
            LispExp::Number(1.0),
            LispExp::Symbol("b".into()),
            LispExp::List(vec![
                LispExp::Symbol("+".into()),
                LispExp::Number(1.0),
                LispExp::Number(2.0),
            ]),
        ]);

        let result = eval(&setq_exp, &mut env, &mut ctx).unwrap();

        // Returns last assigned value
        assert_eq!(result, LispExp::Number(3.0));
        assert_eq!(env.variables.get("a").unwrap(), &LispExp::Number(1.0));
        assert_eq!(env.variables.get("b").unwrap(), &LispExp::Number(3.0));
    }

    #[test]
    fn test_lambda_argument_binding() {
        let mut env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // (defun add-custom (x y) (+ x y))
        let defun_exp = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::Symbol("add-custom".into()),
            LispExp::List(vec![
                LispExp::Symbol("x".into()),
                LispExp::Symbol("y".into()),
            ]),
            LispExp::List(vec![
                LispExp::Symbol("+".into()),
                LispExp::Symbol("x".into()),
                LispExp::Symbol("y".into()),
            ]),
        ]);
        eval(&defun_exp, &mut env, &mut ctx).unwrap();

        // (add-custom 10.0 20.0)
        let call_exp = LispExp::List(vec![
            LispExp::Symbol("add-custom".into()),
            LispExp::Number(10.0),
            LispExp::Number(20.0),
        ]);
        eprintln!("{:?}", call_exp);
        let result = eval(&call_exp, &mut env, &mut ctx).unwrap();

        assert_eq!(result, LispExp::Number(30.0));

        // Ensure parameters didn't leak into global env
        assert!(env.variables.get("x").is_none());
    }

    #[test]
    fn test_generic_host_mutation() {
        let mut env = setup_env_test();

        // Initialize our "Editor State" equivalent
        let mut ctx = TestHost { state_changes: 0 };

        // AST: (inc-state)
        let call_mutation = LispExp::List(vec![LispExp::Symbol("inc-state".into())]);

        // Call it three times
        eval(&call_mutation, &mut env, &mut ctx).unwrap();
        eval(&call_mutation, &mut env, &mut ctx).unwrap();
        eval(&call_mutation, &mut env, &mut ctx).unwrap();

        // The Rust host context should have tracked the changes natively!
        assert_eq!(ctx.state_changes, 3);
    }

    // Mock Host context to assist evaluating side effects
    #[derive(Clone, Debug, PartialEq)]
    struct MockHost {
        pub tracker: f64,
    }

    fn setup_interpreter_env() -> (Env<MockHost>, MockHost) {
        let mut env = Env::new();
        // Bind a global state mutating tool

        env.functions.insert(
            "bump".into(),
            LispExp::Primitive(|_args: &[LispExp<MockHost>], ctx| {
                ctx.tracker += 1.0;
                Ok(LispExp::Number(ctx.tracker))
            }),
        );

        (env, MockHost { tracker: 0.0 })
    }

    #[test]
    fn test_progn_execution_and_side_effects() {
        let (mut env, mut ctx) = setup_interpreter_env();

        // (progn (bump) (bump) 99.0) -> changes state twice, returns last value
        let exp = LispExp::List(vec![
            LispExp::Symbol("progn".into()),
            LispExp::List(vec![LispExp::Symbol("bump".into())]),
            LispExp::List(vec![LispExp::Symbol("bump".into())]),
            LispExp::Number(99.0),
        ]);

        let result = eval(&exp, &mut env, &mut ctx).unwrap();
        assert_eq!(result, LispExp::Number(99.0));
        assert_eq!(ctx.tracker, 2.0);

        // Empty progn should evaluate to nil symbol safely
        let empty_progn = LispExp::List(vec![LispExp::Symbol("progn".into())]);
        assert_eq!(
            eval(&empty_progn, &mut env, &mut ctx).unwrap(),
            LispExp::Symbol("nil".into())
        )
    }

    #[test]
    fn test_let_scoping_and_shadowing() {
        let (mut env, mut ctx) = setup_interpreter_env();
        // Inject global variable x = 10.0
        env.variables.insert("x".into(), LispExp::Number(10.0));

        // AST representation of:
        // (let ((x 20.0) (y 30.0)) (+ x y))
        // Assuming native primitives like '+' are mapped
        env.functions.insert(
            "+".into(),
            LispExp::Primitive(|args, _| {
                if let (LispExp::Number(a), LispExp::Number(b)) = (&args[0], &args[1]) {
                    Ok(LispExp::Number(a + b))
                } else {
                    Err(EvalError::UnvalidFunctionCall)
                }
            }),
        );

        let let_exp = LispExp::List(vec![
            LispExp::Symbol("let".into()),
            LispExp::List(vec![
                LispExp::List(vec![LispExp::Symbol("x".into()), LispExp::Number(20.0)]),
                LispExp::List(vec![LispExp::Symbol("y".into()), LispExp::Number(30.0)]),
            ]),
            LispExp::List(vec![
                LispExp::Symbol("+".into()),
                LispExp::Symbol("x".into()),
                LispExp::Symbol("y".into()),
            ]),
        ]);

        let result = eval(&let_exp, &mut env, &mut ctx).unwrap();
        assert_eq!(result, LispExp::Number(50.0));

        // CRITICAL CHECK: Global variable namespace must be untouched (Lexical isolation)
        assert_eq!(env.variables.get("x"), Some(&LispExp::Number(10.0)));
        assert!(env.variables.get("y").is_none());
    }

    #[test]
    fn test_let_malformed_syntax_errors() {
        let (mut env, mut ctx) = setup_interpreter_env();

        // 1. Missing binding block entirely
        let no_bindings = LispExp::List(vec![LispExp::Symbol("let".into())]);
        assert_eq!(
            eval(&no_bindings, &mut env, &mut ctx).unwrap_err(),
            EvalError::LetNoBindingsProvided
        );

        // 2. Binding block isn't a collection list: (let x body)
        let invalid_list = LispExp::List(vec![
            LispExp::Symbol("let".into()),
            LispExp::Symbol("x".into()),
            LispExp::Number(1.0),
        ]);
        assert_eq!(
            eval(&invalid_list, &mut env, &mut ctx).unwrap_err(),
            EvalError::LetUnvalidBindingList
        );

        // 3. Malformed tuple inside binding array: (let ((x 1 2)) body)
        let broken_tuple = LispExp::List(vec![
            LispExp::Symbol("let".into()),
            LispExp::List(vec![LispExp::List(vec![
                LispExp::Symbol("x".into()),
                LispExp::Number(1.0),
                LispExp::Number(2.0),
            ])]),
            LispExp::Symbol("x".into()),
        ]);
        assert_eq!(
            eval(&broken_tuple, &mut env, &mut ctx).unwrap_err(),
            EvalError::LetUnvalidBindingAt(0)
        );
    }

    #[test]
    fn test_map_evaluation_and_expression_resolution() {
        let (mut env, mut ctx) = setup_interpreter_env();
        env.variables.insert("factor".into(), LispExp::Number(5.0));

        // A map structure: { "computed-val" (progn (bump) factor) }
        let mut input_map = HashMap::new();
        input_map.insert(
            "computed-val".to_string(),
            LispExp::List(vec![
                LispExp::Symbol("progn".into()),
                LispExp::List(vec![LispExp::Symbol("bump".into())]),
                LispExp::Symbol("factor".into()),
            ]),
        );

        let map_exp = LispExp::Map(input_map);
        let evaluated = eval(&map_exp, &mut env, &mut ctx).unwrap();

        if let LispExp::Map(output_map) = evaluated {
            assert_eq!(output_map.get("computed-val"), Some(&LispExp::Number(5.0)));
            assert_eq!(ctx.tracker, 1.0); // Verifies expressions inner-eval inside Maps
        } else {
            panic!("Evaluation should retain structural map wrapper type");
        }
    }
}
