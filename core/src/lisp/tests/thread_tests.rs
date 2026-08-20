#[cfg(test)]
mod tests {
    use crate::lisp::base_env::setup_base_env;
    use crate::lisp::{Env, EvalError, LispContext, LispExp, Parser, eval};
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    // A thread-safe dummy host context
    #[derive(Clone, Debug)]
    struct ThreadCtx {
        pub host_counter: Arc<RwLock<usize>>,
    }

    impl LispContext for ThreadCtx {
        fn consume_fuel(&self, _amount: u32) -> Result<(), EvalError> {
            Ok(())
        }

        fn log_diagnostic(&self, _msg: &str) {}
    }

    // Required for the generic bound T: PartialEq
    impl PartialEq for ThreadCtx {
        fn eq(&self, other: &Self) -> bool {
            Arc::ptr_eq(&self.host_counter, &other.host_counter)
        }
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

    fn setup_thread_env() -> (Arc<Env<ThreadCtx>>, ThreadCtx) {
        let env = Env::new_root();

        // Load the atom, deref, reset, and funcall primitives
        setup_base_env(env.clone());

        // Inject a sleep primitive to safely test async execution delays
        env.set_function(
            "sleep-ms".into(),
            LispExp::primitive(
                |args, _, _ctx| {
                    if let Some(LispExp::Number(ms)) = args.first() {
                        std::thread::sleep(Duration::from_millis(*ms as u64));
                    }
                    Ok(LispExp::list(vec![])) // return nil
                },
                None,
            ),
        );

        // Inject a primitive that mutates the Host Context directly
        env.set_function(
            "bump-host!".into(),
            LispExp::primitive(
                |_args, _, ctx: &ThreadCtx| {
                    let mut lock = ctx.host_counter.write().unwrap();
                    *lock += 1;
                    Ok(LispExp::number(*lock as f64))
                },
                None,
            ),
        );

        let ctx = ThreadCtx {
            host_counter: Arc::new(RwLock::new(0)),
        };

        (env, ctx)
    }

    #[test]
    fn test_baseline_atom_operations() {
        let (env, mut ctx) = setup_thread_env();

        let script = r#"
            (setq my-state (make-atom 10.0))
            (reset my-state 99.0)
            (deref my-state)
        "#;

        let result = eval_script(script, env, &mut ctx).unwrap();
        assert_eq!(result, LispExp::Number(99.0));
    }

    #[test]
    fn test_spawn_async_execution_and_synchronization() {
        let (env, mut ctx) = setup_thread_env();

        // Script: Initialize an atom to 0.
        // Spawn a background thread that sleeps for 50ms, then updates the atom to 42.
        let script = r#"
            (setq shared-val (make-atom 0.0))
            (spawn (lambda ()
                (progn
                    (sleep-ms 50.0)
                    (reset shared-val 42.0))))
        "#;

        eval_script(script, env.clone(), &mut ctx).unwrap();

        // IMMEDIATE ASSERTION:
        // The main thread continues instantly. The atom should STILL be 0.0 here!
        let immediate_val = eval_script("(deref shared-val)", env.clone(), &mut ctx).unwrap();
        assert_eq!(immediate_val, LispExp::Number(0.0));

        // SYNCHRONIZATION:
        // Spinlock/poll until the background thread finishes its sleep and updates the atom.
        let mut timeout = 0;
        loop {
            let current_val = eval_script("(deref shared-val)", env.clone(), &mut ctx).unwrap();
            if current_val == LispExp::Number(42.0) {
                break; // Thread successfully completed!
            }

            std::thread::sleep(Duration::from_millis(10));
            timeout += 10;

            if timeout > 1000 {
                panic!("Background thread failed to execute within 1 second!");
            }
        }
    }

    #[test]
    fn test_multiple_threads_mutating_host_context() {
        let (env, mut ctx) = setup_thread_env();

        // Spawn 10 independent background threads.
        // Each thread sleeps for a short random-ish time, then mutates the Rust Host context.
        let script = r#"
            (defun spawn-worker (delay)
                (spawn (lambda ()
                    (progn
                        (sleep-ms delay)
                        (bump-host!)))))
            
            (spawn-worker 10.0)
            (spawn-worker 20.0)
            (spawn-worker 30.0)
            (spawn-worker 40.0)
            (spawn-worker 50.0)
            (spawn-worker 15.0)
            (spawn-worker 25.0)
            (spawn-worker 35.0)
            (spawn-worker 45.0)
            (spawn-worker 55.0)
        "#;

        eval_script(script, env.clone(), &mut ctx).unwrap();

        // Wait for all 10 threads to finish (max sleep is 55ms, we wait 150ms to be safe)
        std::thread::sleep(Duration::from_millis(150));

        // Read the host counter natively in Rust
        let final_count = *ctx.host_counter.read().unwrap();

        // If data-races were possible, this number would be unpredictable.
        // Thanks to Arc<RwLock<T>> and pure lexical environment cloning, it is strictly 10.
        assert_eq!(final_count, 10);
    }
}
