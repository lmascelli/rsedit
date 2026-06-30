mod lisp;
use lisp::SharedAtom;
pub use lisp::{Env, EvalError, LispContext, LispExp, Parser, ParserError, eval};
mod base_env;
pub use base_env::setup_base_env;
mod handshake;
pub use handshake::bootstrap_vm;

#[cfg(test)]
use lisp::{Lambda, Token};
mod tests;
