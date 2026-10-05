//! Ordering numbers.
//!
//! Variadic, like Lisp's: `(< 1 2 3)` asks whether the whole chain ascends,
//! not whether the first two do. One shape, written once and used by all of
//! them.
use super::*;

const COMPARE_DOC: &str = "(= NUMBER &rest NUMBERS): Return t if all arguments are \
                 numerically equal, nil otherwise. Requires at least one \
                 argument.\n\n\
                 Example:\n\
                 (= 1 1 1) => t\n\
                 (= 1 1 2) => nil\n\
                 (= 3)     => t";

fn primitive_compare<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    some_arguments(args)?;
    let mut numbers = Vec::with_capacity(args.len());
    for arg in args {
        if let LispExp::Number(n) = arg {
            numbers.push(*n);
        } else {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: arg.clone(),
            });
        }
    }
    Ok(LispExp::boolean(numbers.windows(2).all(|w| w[0] == w[1])))
}

const LT_DOC: &str = "(< NUMBER &rest NUMBERS): Return t if the arguments are in \
                 strictly increasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (< 1 2 3) => t\n\
                 (< 1 3 2) => nil";

fn primitive_lt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    compare_chain(args, |a, b| a < b)
}

const GT_DOC: &str = "(> NUMBER &rest NUMBERS): Return t if the arguments are in \
                 strictly decreasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (> 3 2 1) => t\n\
                 (> 3 1 2) => nil";

fn primitive_gt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    compare_chain(args, |a, b| a > b)
}

const LE_DOC: &str = "(<= NUMBER &rest NUMBERS): Return t if the arguments are in \
                 non-decreasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (<= 1 1 2) => t\n\
                 (<= 2 1)   => nil";

fn primitive_le<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    compare_chain(args, |a, b| a <= b)
}

const GE_DOC: &str = "(>= NUMBER &rest NUMBERS): Return t if the arguments are in \
                 non-increasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (>= 2 1 1) => t\n\
                 (>= 1 2)   => nil";

fn primitive_ge<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    compare_chain(args, |a, b| a >= b)
}

const MAX_DOC: &str = "(max NUMBER &rest NUMBERS): Return the largest of the \
                 arguments.\n\n\
                 Example:\n\
                 (max 1 5 3) => 5";

fold_numbers!(primitive_max, max);

const MIN_DOC: &str = "(min NUMBER &rest NUMBERS): Return the smallest of the \
                 arguments.\n\n\
                 Example:\n\
                 (min 1 5 3) => 1";

fold_numbers!(primitive_min, min);

/// Register this module's primitives: ordering numbers.
///
/// Called by [`super::setup_base_env`]. Here rather than there because a
/// primitive's name, its implementation and its docstring are one fact in three
/// pieces, and they were a thousand lines apart.
pub(super) fn install<T: LispContext>(into: &Registry<T>) {
    into.function("=", primitive_compare, COMPARE_DOC);
    into.function("<", primitive_lt, LT_DOC);
    into.function(">", primitive_gt, GT_DOC);
    into.function("<=", primitive_le, LE_DOC);
    into.function(">=", primitive_ge, GE_DOC);
    into.function("max", primitive_max, MAX_DOC);
    into.function("min", primitive_min, MIN_DOC);
}
