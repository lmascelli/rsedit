//! Arithmetic.
//!
//! Everything here is `f64`, which is the one decision this Lisp makes about
//! numbers. Integer division is spelled by `floor`, and `%` and `mod` are the
//! same primitive under two names.
use super::*;

const SUM_DOC: &str = "(+ &rest NUMBERS): Return the sum of NUMBERS. With no arguments, \
                 returns 0.\n\n\
                 Example:\n\
                 (+ 1 2 3) => 6\n\
                 (+)       => 0";

const SUBTRACTION_DOC: &str = "(- NUMBER &rest SUBTRAHENDS): Subtract each of \
                 SUBTRAHENDS from NUMBER. With a single argument, negates it.\n\n\
                 Example:\n\
                 (- 10 3 2) => 5\n\
                 (- 5)      => -5";

const MOD_DOC: &str = "(mod NUMBER DIVISOR): Return the remainder of dividing NUMBER by \
                 DIVISOR. Signals on a DIVISOR of zero. `%' is the same function under its other \
                 name.\n\n\
                 Example:\n\
                 (mod 7 3) => 1\n\
                 (% 7 3)   => 1";

const PERCENT_DOC: &str = "(% NUMBER DIVISOR): Return the remainder of dividing NUMBER by \
                 DIVISOR. The same function as `mod', under the name C and its descendants use.\n\n\
                 Example:\n\
                 (% 7 3) => 1";

fn primitive_sum<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    let mut sum = 0.0;
    for arg in args {
        if let LispExp::Number(number) = arg {
            sum += number;
        } else {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: arg.clone(),
            });
        }
    }
    Ok(LispExp::number(sum))
}

fn primitive_subtraction<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.is_empty() {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        })
    } else {
        let first = expect_number(&args[0])?;
        if args.len() == 1 {
            Ok(LispExp::number(-first))
        } else {
            let mut result = first;
            for arg in &args[1..] {
                result -= expect_number(arg)?;
            }
            Ok(LispExp::number(result))
        }
    }
}

const MUL_DOC: &str = "(* &rest NUMBERS): Return the product of NUMBERS. With no \
                 arguments, returns 1.\n\n\
                 Example:\n\
                 (* 2 3 4) => 24\n\
                 (*)       => 1";

fn primitive_mul<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    let mut product = 1.0;
    for arg in args {
        product *= expect_number(arg)?;
    }
    Ok(LispExp::number(product))
}

const DIV_DOC: &str = "(/ NUMBER &rest DIVISORS): Divide NUMBER by each of DIVISORS \
                 in turn. With a single argument, returns its reciprocal. \
                 Signals a runtime error on division by zero.\n\n\
                 Example:\n\
                 (/ 20 2 5) => 2\n\
                 (/ 4)      => 0.25\n\
                 (/ 1 0)    => error, division by zero";

fn primitive_div<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    some_arguments(args)?;
    let first = expect_number(&args[0])?;
    if args.len() == 1 {
        if first == 0.0 {
            return Err(EvalError::RuntimeMessage(
                "Arithmetic error: division by zero".into(),
            ));
        }
        return Ok(LispExp::number(1.0 / first));
    }
    let mut result = first;
    for arg in &args[1..] {
        let divisor = expect_number(arg)?;
        if divisor == 0.0 {
            return Err(EvalError::RuntimeMessage(
                "Arithmetic error: division by zero".into(),
            ));
        }
        result /= divisor;
    }
    Ok(LispExp::number(result))
}

fn primitive_mod<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 2)?;
    let a = expect_number(&args[0])?;
    let b = expect_number(&args[1])?;
    if b == 0.0 {
        return Err(EvalError::RuntimeMessage(
            "Arithmetic error: division by zero".into(),
        ));
    }
    let r = a % b;
    let result = if r != 0.0 && (r < 0.0) != (b < 0.0) {
        r + b
    } else {
        r
    };
    Ok(LispExp::number(result))
}

const N_1PLUS_DOC: &str = "(1+ NUMBER): Return NUMBER plus one.\n\n\
                 Example:\n\
                 (1+ 4) => 5";

fn primitive_1plus<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 1)?;
    Ok(LispExp::number(expect_number(&args[0])? + 1.0))
}

const N_1MINUS_DOC: &str = "(1- NUMBER): Return NUMBER minus one.\n\n\
                 Example:\n\
                 (1- 4) => 3";

fn primitive_1minus<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 1)?;
    Ok(LispExp::number(expect_number(&args[0])? - 1.0))
}

// `abs' was filed under `Comparisons' and `floor' under `Strings' in the file
// this came from -- both of them one line long, both of them arithmetic, and
// neither of them findable by looking where they belong. A directory makes that
// kind of drift visible, which is most of the point of being one.
const ABS_DOC: &str = "(abs NUMBER): Return the absolute value of NUMBER.\n\n\
                 Example:\n\
                 (abs -5) => 5\n\
                 (abs 5)  => 5";

number_op!(primitive_abs, abs);

const FLOOR_DOC: &str = "(floor NUMBER): Return the largest integer not greater than NUMBER. \
                 Negative numbers round away from zero, so (floor -1.5) is -2.\n\n\
                 Needed because `/' here is division, not integer division: (/ 1 3) is \
                 0.3333333, and anything counting rows and columns wants the 0.\n\n\
                 Example:\n\
                 (floor (/ 7 3)) => 2";

number_op!(primitive_floor, floor);

/// Register this module's primitives: arithmetic.
///
/// Called by [`super::setup_base_env`]. Here rather than there because a
/// primitive's name, its implementation and its docstring are one fact in three
/// pieces, and they were a thousand lines apart.
pub(super) fn install<T: LispContext>(into: &Registry<T>) {
    into.function("abs", primitive_abs, ABS_DOC);
    into.function("floor", primitive_floor, FLOOR_DOC);
    // Base math
    // `+`, `-` and `mod`/`%` are deliberately left undocumented: `(+)` should
    // return 0 but currently errors, `(- 5)` should negate to -5 but returns
    // 5 unchanged, and `mod` doesn't follow Elisp's "result takes the sign of
    // the divisor" rule for a negative divisor. Documenting them now would
    // just describe the bugs as if they were the intended behavior.
    into.function("+", primitive_sum, SUM_DOC);
    into.function("-", primitive_subtraction, SUBTRACTION_DOC);
    into.function("*", primitive_mul, MUL_DOC);
    into.function("/", primitive_div, DIV_DOC);
    into.function("mod", primitive_mod, MOD_DOC);
    // Its own docstring, naming `%': a reader asking about `%' is shown
    // the call they typed rather than the other spelling of it.
    into.function("%", primitive_mod, PERCENT_DOC);
    into.function("1+", primitive_1plus, N_1PLUS_DOC);
    into.function("1-", primitive_1minus, N_1MINUS_DOC);
}
