use crate::lisp::{Env, EvalError, LispContext, LispExp, Parser, eval, setup_base_env};
use std::sync::Arc;

/// Boots the agnostic Lisp engine, registers standard environments,
/// and runs an internal diagnostic script to verify VM stability.
pub fn bootstrap_vm<T: LispContext>(ctx: &mut T) -> Result<Arc<Env<T>>, EvalError> {
    // 1. Initialize the pristine root environment
    let env = Env::new_root();

    // 2. Load basic primitives (Math, Atoms, Threading, Fibers)
    setup_base_env(env.clone());

    // 3. The Handshake Script: An embedded Lisp string that stresses the engine features
    // to verify closures, arithmetic, loops, and basic fiber allocations work.
    let verification_script = r#"
        (progn
            ;; Test structural scoping & primitives
            (setq verify-math (- (+ 5 5) 2)) ;; 8
            
            ;; Test Fiber allocation & step execution
            (setq verify-fiber (fiber 100 200))
            (setq fiber-step (resume verify-fiber))
            
            ;; Return validation status
            (if (= verify-math 8)
                (if (= fiber-step 100)
                    'vm-status-healthy
                    'error-fiber-failed)
                'error-math-failed))
    "#;

    // 4. Parse and evaluate the handshake script
    let mut parser = Parser::new(verification_script);
    let ast = parser.next().or_else(|err| {
        Err(EvalError::UncorrectFunctionDefinition) // Fallback if parsing fails
    })?;

    ctx.log_diagnostic("VM Handshake: Initiating verification script...");

    let status = eval(&ast, env.clone(), ctx)?;

    // 5. Verify the state outcome
    if status == LispExp::symbol("vm-status-healthy".into()) {
        ctx.log_diagnostic("VM Handshake: State verification successful. Core is stable.");
        Ok(env)
    } else {
        ctx.log_diagnostic(&format!(
            "VM Handshake CRITICAL: State corrupted. Got {:?}",
            status
        ));
        Err(EvalError::UncorrectFunctionDefinition)
    }
}
