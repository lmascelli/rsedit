#[cfg(test)]
mod tests {
    use crate::lisp::{Env, LispExp, bootstrap_vm, setup_base_env};
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

    fn quote(name: &str) -> LispExp<MockHostContext> {
        LispExp::list(vec![
            LispExp::symbol("quote".into()),
            LispExp::symbol(name.into()),
        ])
    }

    #[test]
    fn test_add_to_list_prepends_and_ignores_duplicates() {
        let env = Env::new_root();
        setup_base_env(env.clone());

        let mut ctx = MockHostContext {
            fuel_remaining: RwLock::new(5000),
            logs: RwLock::new(vec![]),
        };

        // 1. Setup: Define a list (2 3)
        let initial_list = LispExp::proper_list(vec![LispExp::number(2.0), LispExp::number(3.0)]);
        env.set_variable("my-list".into(), initial_list);

        // 2. Evaluate: (add-to-list 'my-list 1) -> Should prepend 1
        let exp_add_1 = LispExp::list(vec![
            LispExp::symbol("add-to-list".into()),
            quote("my-list"),
            LispExp::number(1.0),
        ]);
        eval(&exp_add_1, env.clone(), &mut ctx).unwrap();

        // Check if 1 was prepended
        let current_list = env.get_variable("my-list").unwrap();
        assert_eq!(
            current_list,
            LispExp::proper_list(vec![
                LispExp::number(1.0),
                LispExp::number(2.0),
                LispExp::number(3.0)
            ])
        );

        // 3. Evaluate: (add-to-list 'my-list 2) -> Should do nothing (2 exists)
        let exp_add_duplicate = LispExp::list(vec![
            LispExp::symbol("add-to-list".into()),
            quote("my-list"),
            LispExp::number(2.0),
        ]);
        eval(&exp_add_duplicate, env.clone(), &mut ctx).unwrap();

        // Check it remains unmodified
        let unmodified_list = env.get_variable("my-list").unwrap();
        assert_eq!(
            unmodified_list,
            LispExp::proper_list(vec![
                LispExp::number(1.0),
                LispExp::number(2.0),
                LispExp::number(3.0)
            ])
        );
    }

    #[test]
    fn test_append_to_list_appends_and_ignores_duplicates() {
        let env = Env::new_root();
        setup_base_env(env.clone());
        let mut ctx = MockHostContext {
            fuel_remaining: RwLock::new(5000),
            logs: RwLock::new(vec![]),
        };

        // 1. Setup: Define a list (1 2)
        let initial_list = LispExp::proper_list(vec![LispExp::number(1.0), LispExp::number(2.0)]);
        env.set_variable("my-list".into(), initial_list);

        // 2. Evaluate: (append-to-list 'my-list 3) -> Should append 3
        let exp_append_3 = LispExp::list(vec![
            LispExp::symbol("append-to-list".into()),
            quote("my-list"),
            LispExp::number(3.0),
        ]);
        eval(&exp_append_3, env.clone(), &mut ctx).unwrap();

        // Check if 3 was appended at the end
        let current_list = env.get_variable("my-list").unwrap();
        assert_eq!(
            current_list,
            LispExp::proper_list(vec![
                LispExp::number(1.0),
                LispExp::number(2.0),
                LispExp::number(3.0)
            ])
        );

        // 3. Evaluate: (append-to-list 'my-list 2) -> Should do nothing
        let exp_append_duplicate = LispExp::list(vec![
            LispExp::symbol("append-to-list".into()),
            quote("my-list"),
            LispExp::number(2.0),
        ]);
        eval(&exp_append_duplicate, env.clone(), &mut ctx).unwrap();

        let unmodified_list = env.get_variable("my-list").unwrap();
        assert_eq!(
            unmodified_list,
            LispExp::proper_list(vec![
                LispExp::number(1.0),
                LispExp::number(2.0),
                LispExp::number(3.0)
            ])
        );
    }

    #[test]
    fn test_remove_from_list_handles_multiple_removals() {
        let env = Env::new_root();
        setup_base_env(env.clone());
        let mut ctx = MockHostContext {
            fuel_remaining: RwLock::new(5000),
            logs: RwLock::new(vec![]),
        };

        // 1. Setup: Define a list (1 2 3 4 5)
        let initial_list = LispExp::proper_list(vec![
            LispExp::number(1.0),
            LispExp::number(2.0),
            LispExp::number(3.0),
            LispExp::number(4.0),
            LispExp::number(5.0),
        ]);
        env.set_variable("my-list".into(), initial_list);

        // 2. Evaluate: (remove-from-list 'my-list 2 4)
        let exp_remove = LispExp::list(vec![
            LispExp::symbol("remove-from-list".into()),
            quote("my-list"),
            LispExp::number(2.0),
            LispExp::number(4.0),
        ]);
        eval(&exp_remove, env.clone(), &mut ctx).unwrap();

        // Check if 2 and 4 were removed correctly
        let current_list = env.get_variable("my-list").unwrap();
        assert_eq!(
            current_list,
            LispExp::proper_list(vec![
                LispExp::number(1.0),
                LispExp::number(3.0),
                LispExp::number(5.0)
            ])
        );

        // 3. Evaluate: (remove-from-list 'my-list 99) -> Missing item should be ignored
        let exp_remove_missing = LispExp::list(vec![
            LispExp::symbol("remove-from-list".into()),
            quote("my-list"),
            LispExp::number(99.0),
        ]);
        eval(&exp_remove_missing, env.clone(), &mut ctx).unwrap();

        let unmodified_list = env.get_variable("my-list").unwrap();
        assert_eq!(
            unmodified_list,
            LispExp::proper_list(vec![
                LispExp::number(1.0),
                LispExp::number(3.0),
                LispExp::number(5.0)
            ])
        );
    }
}
