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
}
