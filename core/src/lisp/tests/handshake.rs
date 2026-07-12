#[cfg(test)]
mod tests {
    use crate::lisp::bootstrap_vm;
    use crate::lisp::{EvalError, LispContext, Parser, eval};
    use std::sync::RwLock;

    // Define a dummy agnostic host context that tracks fuel and logs
    #[derive(Debug)]
    struct MockHostContext {
        pub fuel_remaining: RwLock<u32>,
        pub logs: RwLock<Vec<String>>,
    }

    impl Clone for MockHostContext {
        fn clone(&self) -> Self {
            unreachable!()
        }
    }

    impl PartialEq for MockHostContext {
        fn eq(&self, _other: &Self) -> bool {
            unreachable!()
        }
    }

    impl LispContext for MockHostContext {
        fn consume_fuel(&self, amount: u32) -> Result<(), EvalError> {
            if *self.fuel_remaining.read().unwrap() < amount {
                Err(EvalError::OutOfFuel)
            } else {
                *self.fuel_remaining.write().unwrap() -= amount;
                Ok(())
            }
        }

        fn log_diagnostic(&self, msg: &str) {
            self.logs.write().unwrap().push(msg.to_string());
        }
    }

    #[test]
    fn test_successful_bootstrap_handshake() {
        let mut ctx = MockHostContext {
            fuel_remaining: RwLock::new(5000),
            logs: RwLock::new(vec![]),
        };

        // If the core code functions flawlessly, bootstrap finishes successfully
        let env_res = bootstrap_vm(&mut ctx);
        assert!(env_res.is_ok());
        assert!(
            ctx.logs.read().unwrap().contains(
                &"VM Handshake: State verification successful. Core is stable.".to_string()
            )
        );
    }

    #[test]
    fn test_fuel_system_stops_infinite_loops() {
        let mut ctx = MockHostContext {
            fuel_remaining: 100.into(), // Explicitly tight budget
            logs: vec![].into(),
        };

        let env = bootstrap_vm(&mut ctx).unwrap();

        // A malicious loop script that would normally lock up the thread forever
        let malicious_script = "(while t (setq x 1))";
        let mut parser = Parser::new(malicious_script);
        let ast = parser.next().unwrap();

        // Evaluate the script
        let res = eval(&ast, env, &mut ctx);

        // The stack safely unrolls!
        assert_eq!(res, Err(EvalError::OutOfFuel));
        assert_eq!(*ctx.fuel_remaining.read().unwrap(), 0);
        // Thread is alive, control is handed back to host safely!
    }
}
