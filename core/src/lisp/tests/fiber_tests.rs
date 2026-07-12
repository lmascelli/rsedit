#[cfg(test)]
mod tests {
    use crate::lisp::base_env::setup_base_env;
    use crate::lisp::{Env, EvalError, LispContext, LispExp, Parser, eval};
    use std::sync::Arc;

    // A simple dummy context
    #[derive(Clone, Debug, PartialEq)]
    struct FiberCtx;

    impl LispContext for FiberCtx {
        fn consume_fuel(&self, _amount: u32) -> Result<(), EvalError> {
            Ok(())
        }

        fn log_diagnostic(&self, _msg: &str) {}
    }

    // Helper to evaluate multiple expressions easily by wrapping them in a progn
    fn eval_script<T: LispContext>(
        script: &str,
        env: Arc<Env<T>>,
        ctx: &mut T,
    ) -> Result<LispExp<T>, EvalError> {
        let wrapped = format!("(progn {})", script);
        let mut parser = Parser::new(&wrapped);
        let exp = parser.next().unwrap();
        eval(&exp, env, ctx)
    }

    fn setup_fiber_env() -> (Arc<Env<FiberCtx>>, FiberCtx) {
        let env = Env::new_root();

        // This loads `resume`, `atom`, `deref`, `reset`, and `funcall`
        setup_base_env(env.clone());

        (env, FiberCtx)
    }

    #[test]
    fn test_fiber_basic_yielding() {
        let (env, mut ctx) = setup_fiber_env();

        // Create a fiber that yields three distinct numbers
        let script = r#"
            (setq my-task (fiber 
                10.0 
                20.0 
                30.0))
        "#;
        eval_script(script, env.clone(), &mut ctx).unwrap();

        // Crank the fiber forward one step at a time
        let step1 = eval_script("(resume my-task)", env.clone(), &mut ctx).unwrap();
        assert_eq!(step1, LispExp::Number(10.0));

        let step2 = eval_script("(resume my-task)", env.clone(), &mut ctx).unwrap();
        assert_eq!(step2, LispExp::Number(20.0));

        let step3 = eval_script("(resume my-task)", env.clone(), &mut ctx).unwrap();
        assert_eq!(step3, LispExp::Number(30.0));

        // Fiber is exhausted, should safely return nil on all future calls
        let exhausted1 = eval_script("(resume my-task)", env.clone(), &mut ctx).unwrap();
        assert_eq!(exhausted1, LispExp::symbol("nil".into()));

        let exhausted2 = eval_script("(resume my-task)", env.clone(), &mut ctx).unwrap();
        assert_eq!(exhausted2, LispExp::symbol("nil".into()));
    }

    #[test]
    fn test_fiber_internal_state_preservation() {
        let (env, mut ctx) = setup_fiber_env();

        // The fiber uses a `let` block, but because `fiber` captures its environment
        // exactly like a lambda, the `counter` variable should persist across yields!
        let script = r#"
            (setq counter-task 
                (let ((counter 0))
                    (fiber 
                        (setq counter (+ counter 1))
                        (setq counter (+ counter 10))
                        (setq counter (+ counter 100)))))
        "#;

        // Note: You need a native `+` function to make this test work if you haven't
        // added one to your base_env. Let's mock it just for this test:
        env.set_function(
            "+".into(),
            LispExp::Primitive(|args, _, _| {
                let mut sum = 0.0;
                for arg in args {
                    if let LispExp::Number(n) = arg {
                        sum += n;
                    }
                }
                Ok(LispExp::Number(sum))
            }),
        );

        eval_script(script, env.clone(), &mut ctx).unwrap();

        let step1 = eval_script("(resume counter-task)", env.clone(), &mut ctx).unwrap();
        assert_eq!(step1, LispExp::Number(1.0));

        let step2 = eval_script("(resume counter-task)", env.clone(), &mut ctx).unwrap();
        assert_eq!(step2, LispExp::Number(11.0));

        let step3 = eval_script("(resume counter-task)", env.clone(), &mut ctx).unwrap();
        assert_eq!(step3, LispExp::Number(111.0));
    }

    #[test]
    fn test_multiple_interleaved_fibers() {
        let (env, mut ctx) = setup_fiber_env();

        // Create two completely independent fibers
        let script = r#"
            (setq task-a (fiber "A1" "A2"))
            (setq task-b (fiber "B1" "B2"))
        "#;
        eval_script(script, env.clone(), &mut ctx).unwrap();

        // We should be able to alternate calling them without crossing the streams
        let a1 = eval_script("(resume task-a)", env.clone(), &mut ctx).unwrap();
        assert_eq!(a1, LispExp::string("A1".into()));

        let b1 = eval_script("(resume task-b)", env.clone(), &mut ctx).unwrap();
        assert_eq!(b1, LispExp::string("B1".into()));

        let a2 = eval_script("(resume task-a)", env.clone(), &mut ctx).unwrap();
        assert_eq!(a2, LispExp::string("A2".into()));

        let b2 = eval_script("(resume task-b)", env.clone(), &mut ctx).unwrap();
        assert_eq!(b2, LispExp::string("B2".into()));
    }

    #[test]
    fn test_fiber_empty_instantiation() {
        let (env, mut ctx) = setup_fiber_env();

        // (fiber) with no arguments should immediately yield nil
        eval_script("(setq empty-task (fiber))", env.clone(), &mut ctx).unwrap();

        let res = eval_script("(resume empty-task)", env.clone(), &mut ctx).unwrap();
        assert_eq!(res, LispExp::symbol("nil".into()));
    }
}
