#[cfg(test)]
mod tests {
    impl crate::lisp::LispContext for () {
        fn consume_fuel(&self, amount: u32) -> Result<(), crate::lisp::EvalError> {
            Ok(())
        }

        fn log_diagnostic(&self, msg: &str) {}
    }
}

// Parser tests
mod lexer_tests;
mod parser_tests;
// Eval tests
mod eval_tests;
mod lexical_context;
// Thread and concurrency
mod fiber_tests;
mod thread_tests;
// Handshake
mod handshake;
