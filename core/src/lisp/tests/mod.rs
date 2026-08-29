#[cfg(test)]
mod tests {
    impl crate::lisp::LispContext for () {
        fn consume_fuel(&self, _amount: u32) -> Result<(), crate::lisp::EvalError> {
            Ok(())
        }

        fn log_diagnostic(&self, _msg: &str) {}
    }
}

// Parser tests
mod lexer_tests;
mod parser_tests;
// Eval tests
mod backtrace_tests;
mod base_env_tests;
mod eval_tests;
mod lexical_context;
mod lisp_core_compliance_tests;
// Thread, concurrency and fuel
mod fiber_tests;
mod thread_tests;
mod fuel;
// Handshake
mod handshake;
