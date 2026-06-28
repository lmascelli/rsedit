mod lisp;
use lisp::SharedAtom;
pub use lisp::{Env, EvalError, LispExp, Parser, ParserError, eval};
mod base_env;
pub use base_env::setup_base_env;

#[cfg(test)]
use lisp::{Lambda, Token};
mod tests;
