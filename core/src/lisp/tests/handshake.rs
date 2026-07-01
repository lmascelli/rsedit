#[cfg(test)]
mod tests {
    use super::*;
    use crate::lisp::bootstrap_vm;
    use crate::lisp::{EvalError, LispContext, Parser, eval};

    // Define a dummy agnostic host context that tracks fuel and logs
    #[derive(Clone, Debug, PartialEq)]
    struct MockHostContext {
        pub fuel_remaining: u32,
        pub logs: Vec<String>,
    }

    impl LispContext for MockHostContext {
        fn consume_fuel(&mut self, amount: u32) -> Result<(), EvalError> {
            if self.fuel_remaining < amount {
                self.fuel_remaining = 0;
                Err(EvalError::OutOfFuel)
            } else {
                self.fuel_remaining -= amount;
                Ok(())
            }
        }

        fn log_diagnostic(&mut self, msg: &str) {
            self.logs.push(msg.to_string());
        }
    }

    #[test]
    fn test_successful_bootstrap_handshake() {
        let mut ctx = MockHostContext {
            fuel_remaining: 5000,
            logs: vec![],
        };

        // If the core code functions flawlessly, bootstrap finishes successfully
        let env_res = bootstrap_vm(&mut ctx);
        assert!(env_res.is_ok());
        assert!(
            ctx.logs.contains(
                &"VM Handshake: State verification successful. Core is stable.".to_string()
            )
        );
    }

    #[test]
    fn test_fuel_system_stops_infinite_loops() {
        let mut ctx = MockHostContext {
            fuel_remaining: 100, // Explicitly tight budget
            logs: vec![],
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
        assert_eq!(ctx.fuel_remaining, 0);
        // Thread is alive, control is handed back to host safely!
    }
}
