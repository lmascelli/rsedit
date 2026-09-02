mod context;
mod environment;
mod error;
mod eval;
mod fuel;
mod lispexp;
mod parser;
mod types;
mod utils;

pub use context::LispContext;
pub use environment::Env;
pub use error::EvalError;
pub use eval::eval;
pub use fuel::{DEFAULT_FUEL, Exhausted, FuelMeter, FuelScope};
pub use lispexp::LispExp;
pub use parser::{Parser, ParserError};
use types::{
    ConsCell, ConsIter, FiberState, Lambda, LispPrimitive, SharedAtom, SharedFiber,
};
use utils::{
    bind_lambda_args, condition_matches, data_to_form, error_data, error_symbol,
    parse_lambda_params,
};

mod base_env;
mod handshake;
pub use base_env::{call_callable, setup_base_env};
pub use handshake::bootstrap_vm;

#[cfg(test)]
use parser::Token;
mod tests;
