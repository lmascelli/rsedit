mod lisp;
pub use lisp::{Env, EvalError, LispExp, Parser, ParserError, eval};

#[cfg(test)]
use lisp::{Lambda, Token};
mod tests;
