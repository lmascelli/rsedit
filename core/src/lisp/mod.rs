//!
//!  Add a general description of the interpreter here
//!

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
pub use environment::{Env, VARIABLE_DOCUMENTATION};
pub use error::EvalError;
pub use eval::{eval, resume_frames};
#[cfg(test)]
pub use fuel::measure;
pub use fuel::{DEFAULT_FUEL, FuelMeter, FuelScope, set_remaining};
pub use lispexp::LispExp;
pub use parser::{Parser, form_to_data};
use types::{ConsCell, ConsIter, FiberState, SharedAtom, SharedFiber};
pub use types::{Frame, Lambda, LispPrimitive};
use utils::{
    bind_lambda_args, condition_matches, data_to_form, error_data, error_symbol,
    parse_lambda_params,
};
pub use utils::{exact_arity, some_arguments};

mod base;
mod handshake;
pub use base::{call_callable, lisp_display, setup_base_env};
pub use handshake::bootstrap_vm;

#[cfg(test)]
use parser::Token;
mod tests;
