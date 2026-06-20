#[cfg(test)]
mod tests {
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    // A dummy context to satisfy the generic T in our evaluator
    #[derive(Clone, PartialEq, Debug)]
    struct DummyCtx;

    // Helper function to parse and evaluate a simple string expression
    fn eval_str(source: &str, env: Arc<Env<DummyCtx>>, ctx: &mut DummyCtx) -> Result<LispExp<DummyCtx>, EvalError> {
        let mut parser = Parser::new(source);
        let exp = parser.next().unwrap();
        eval(&exp, env, ctx)
    }

    #[test]
    fn test_env_hierarchical_resolution() {
        let root_env = Env::<DummyCtx>::new_root();
        root_env.set_variable("global_var".into(), LispExp::number(100.0));

        let child_env = Env::<DummyCtx>::new_child(&root_env);
        child_env.set_variable("local_var".into(), LispExp::number(42.0));

        // Child should see its own variables
        assert_eq!(child_env.get_variable("local_var"), Some(LispExp::number(42.0)));
        
        // Child should securely traverse up the Arc chain to see parent variables
        assert_eq!(child_env.get_variable("global_var"), Some(LispExp::number(100.0)));
        
        // Root should NOT see child variables
        assert_eq!(root_env.get_variable("local_var"), None);
    }

    #[test]
    fn test_env_update_variable_crawling() {
        let root_env = Env::<DummyCtx>::new_root();
        root_env.set_variable("counter".into(), LispExp::number(1.0));

        let child_env = Env::<DummyCtx>::new_child(&root_env);
        
        // Update the variable from the child scope
        let found_and_updated = child_env.update_variable("counter", LispExp::number(2.0));
        
        assert!(found_and_updated, "Should have found 'counter' in the parent chain");
        
        // Verify the child scope DID NOT shadow it locally
        assert!(!child_env.variables.read().unwrap().contains_key("counter"));
        
        // Verify the root scope WAS mutated through the chain
        assert_eq!(root_env.get_variable("counter"), Some(LispExp::number(2.0)));
    }

    #[test]
    fn test_eval_if_special_form_lazy_evaluation() {
        let root_env = Env::<DummyCtx>::new_root();
        let mut ctx = DummyCtx;

        // Setup a variable to prove branches are evaluated properly
        root_env.set_variable("x".into(), LispExp::number(0.0));
        root_env.set_variable("nil".into(), LispExp::list(vec![]));

        // Test True branch (1 is truthy)
        let true_exp = "(if 1 (setq x 10) (setq x 99))";
        eval_str(true_exp, root_env.clone(), &mut ctx).unwrap();
        assert_eq!(root_env.get_variable("x"), Some(LispExp::number(10.0)));

        // Test False branch (nil is falsy)
        let false_exp = "(if nil (setq x 50) (setq x -5))";
        eval_str(false_exp, root_env.clone(), &mut ctx).unwrap();
        assert_eq!(root_env.get_variable("x"), Some(LispExp::number(-5.0)));
    }

    #[test]
    fn test_eval_setq_shadowing_fix() {
        let root_env = Env::<DummyCtx>::new_root();
        let mut ctx = DummyCtx;

        root_env.set_variable("shared_val".into(), LispExp::number(5.0));
        let child_env = Env::<DummyCtx>::new_child(&root_env);

        // Execute setq in the child environment
        let setq_exp = "(setq shared_val 99)";
        eval_str(setq_exp, child_env.clone(), &mut ctx).unwrap();

        // Because we fixed setq to use `update_variable`, it should modify the root,
        // not create a local shadowed variable in the child.
        assert_eq!(root_env.get_variable("shared_val"), Some(LispExp::number(99.0)));
        assert!(!child_env.variables.read().unwrap().contains_key("shared_val"));
    }

    #[test]
    fn test_eval_let_creates_isolated_scope() {
        let root_env = Env::<DummyCtx>::new_root();
        let mut ctx = DummyCtx;
        
        root_env.set_variable("a".into(), LispExp::number(1.0));

        // 'let' should create a temporary scope where 'a' is 10, but leave root 'a' untouched
        let let_exp = "(let ((a 10)) a)";
        let result = eval_str(let_exp, root_env.clone(), &mut ctx).unwrap();
        
        assert_eq!(result, LispExp::number(10.0));
        // Root environment should remain unchanged after let block exits
        assert_eq!(root_env.get_variable("a"), Some(LispExp::number(1.0)));
    }

    #[test]
    fn test_lexical_vs_dynamic_scoping() {
        let root_env = Env::<DummyCtx>::new_root();
        let mut ctx = DummyCtx;

        // 1. Define 'x' in the global scope
        eval_str("(setq x 10)", root_env.clone(), &mut ctx).unwrap();
        
        // 2. Define a function that returns 'x'
        eval_str("(defun get-x () x)", root_env.clone(), &mut ctx).unwrap();

        // 3. Create a local 'let' block that shadows 'x' with 99, 
        // and call 'get-x' from INSIDE that block.
        let result = eval_str("(let ((x 99)) (get-x))", root_env.clone(), &mut ctx).unwrap();

        // If the language was dynamically scoped, it would look at the caller's scope and return 99.
        // Because we successfully implemented lexical scoping, it strictly uses the scope 
        // where it was DEFINED, returning 10.
        assert_eq!(result, LispExp::number(10.0));
    }

    pub fn primitive_funcall<T>(args: &[LispExp<T>], ctx: &mut T) -> Result<LispExp<T>, EvalError>
where
    T: Clone + PartialEq + std::fmt::Debug + Send + Sync + 'static,
{
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments { expected: 1, got: 0 });
    }
    
    // Because funcall is a normal primitive, its arguments are already evaluated.
    // args[0] is the Lambda object itself, args[1..] are the arguments passed to it.
    let func_obj = &args[0];
    let func_args = &args[1..];

    match func_obj {
        LispExp::Lambda(lambda) => {
            if lambda.params.len() != func_args.len() {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: lambda.params.len(),
                    got: func_args.len(),
                });
            }
            let call_frame = Env::new_child(&lambda.env);
            for (i, param_name) in lambda.params.iter().enumerate() {
                call_frame.set_variable(param_name.clone(), func_args[i].clone());
            }
            eval(&lambda.body, call_frame, ctx)
        }
        LispExp::Primitive(func) => func(func_args, ctx),
        _ => Err(EvalError::UncorrectFunctionDefinition),
    }
}
    
    #[test]
    fn test_upward_funarg_closure_capture() {
        let root_env = Env::<DummyCtx>::new_root();
        let mut ctx = DummyCtx;
        root_env.set_function("funcall".into(), LispExp::Primitive(primitive_funcall));

        // 1. Define a factory function that returns an anonymous lambda.
        // The lambda captures the local 'let' variable "secret".
        let factory_code = "
            (defun make-closure () 
                (let ((secret 42)) 
                    (lambda () secret)))
        ";
        eval_str(factory_code, root_env.clone(), &mut ctx).unwrap();

        // 2. Execute the factory and save the resulting closure to a global variable
        eval_str("(setq my-closure (make-closure))", root_env.clone(), &mut ctx).unwrap();

        // 3. Execute the closure. The 'let' block has long finished executing,
        // but the environment must be kept alive by the Arc<Env> inside the Lambda struct!
        let result = eval_str("(funcall my-closure)", root_env.clone(), &mut ctx).unwrap();

        assert_eq!(result, LispExp::number(42.0));
    }
}
