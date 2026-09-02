//!
//! Description of the utility functions
//!

use super::{Env, EvalError, Lambda, LispContext, LispExp};
use std::sync::Arc;

/// The inverse: reconstitute a form `eval` can dispatch on from a data
/// list. Only reached when data is evaluated — `(eval (list '+ 1 2))`,
/// and macro expansions.
pub(super) fn data_to_form<T: LispContext>(exp: &LispExp<T>) -> Result<LispExp<T>, EvalError<T>> {
    match exp {
        LispExp::Cons(_) => {
            let (items, tail) = exp.split_list();
            if !tail.is_nil() {
                return Err(EvalError::UnvalidFunctionCall);
            }
            // `(quote X)` is the one form whose argument must stay data.
            // The reader leaves it that way (it runs `form_to_data` over
            // the literal at read time), and a macro that builds a quote
            // form by hand -- `(cons 'quote (cons items nil))` -- has to
            // behave identically. Recursing here would turn X into syntax
            // and `quote` would then hand a syntax node back as a value.
            if items.len() == 2 {
                if matches!(&items[0], LispExp::Symbol(s) if s.as_str() == "quote") {
                    return Ok(LispExp::form(vec![items[0].clone(), items[1].clone()]));
                }
            }
            let mut form = Vec::with_capacity(items.len());
            for item in &items {
                form.push(data_to_form(item)?);
            }
            Ok(LispExp::form(form))
        }
        LispExp::Vector(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                out.push(data_to_form(item)?);
            }
            Ok(LispExp::vec(out))
        }
        other => Ok(other.clone()),
    }
}

/// Binds ARGS into CALL_FRAME according to LAMBDA's parameter list:
/// required params first (one-to-one), then `&optional` params
/// (defaulting to `nil` once ARGS runs out), then -- if LAMBDA has an
/// `&rest` param -- every remaining argument collected into a single
/// list. Shared by every lambda/macro call site so the required/
/// optional/rest binding rules only need to be implemented once.
pub fn bind_lambda_args<T: LispContext>(
    lambda: &Lambda<T>,
    args: &[LispExp<T>],
    call_frame: &Arc<Env<T>>,
) -> Result<(), EvalError<T>> {
    let min = lambda.params.len();
    let max = min + lambda.optionals.len();
    if args.len() < min || (lambda.rest.is_none() && args.len() > max) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: if args.len() < min { min } else { max },
            got: args.len(),
        });
    }

    let mut idx = 0;
    for name in &lambda.params {
        call_frame.set_variable(name.clone(), args[idx].clone());
        idx += 1;
    }
    for name in &lambda.optionals {
        let value = args.get(idx).cloned().unwrap_or_else(LispExp::nil);
        call_frame.set_variable(name.clone(), value);
        idx += 1;
    }
    if let Some(rest_name) = &lambda.rest {
        // `idx` can run past `args.len()` here -- e.g. `params.len() == 1`,
        // `optionals.len() == 1`, but only the one required argument was
        // supplied -- so it must be clamped before slicing.
        let rest_start = idx.min(args.len());
        call_frame.set_variable(
            rest_name.clone(),
            // The `&rest` container is data even for a macro, where the
            // elements it holds are unevaluated syntax -- the body walks it
            // with `car`/`cdr` either way.
            LispExp::proper_list(args[rest_start..].to_vec()),
        );
    }
    Ok(())
}

/// Parses a `defun`/`lambda`/`defmacro` parameter list into its three
/// buckets, mirroring Emacs Lisp's own grammar: `(REQUIRED...  [&optional
/// OPTIONAL...] [&rest REST])`. `&optional` and `&rest` are markers, not
/// bindable names -- they select which bucket the symbols that follow
/// land in, and are consumed rather than returned.
pub(super) fn parse_lambda_params<T: LispContext>(
    params_list: &[LispExp<T>],
) -> Result<(Vec<String>, Vec<String>, Option<String>), EvalError<T>> {
    #[derive(PartialEq)]
    enum Mode {
        Required,
        Optional,
        Rest,
        RestDone,
    }

    let mut required = Vec::new();
    let mut optionals = Vec::new();
    let mut rest = None;
    let mut mode = Mode::Required;

    for param in params_list {
        let LispExp::Symbol(name) = param else {
            return Err(EvalError::DefunParamIsNotASymbol);
        };
        match name.as_str() {
            "&optional" => {
                if mode != Mode::Required {
                    return Err(EvalError::DefunMisplacedParamMarker);
                }
                mode = Mode::Optional;
            }
            "&rest" => {
                if mode == Mode::Rest || mode == Mode::RestDone {
                    return Err(EvalError::DefunMisplacedParamMarker);
                }
                mode = Mode::Rest;
            }
            other => match mode {
                Mode::Required => required.push(other.to_string()),
                Mode::Optional => optionals.push(other.to_string()),
                Mode::Rest => {
                    rest = Some(other.to_string());
                    mode = Mode::RestDone;
                }
                Mode::RestDone => return Err(EvalError::DefunRestMustHaveExactlyOneParam),
            },
        }
    }

    // `&rest` with nothing after it.
    if mode == Mode::Rest {
        return Err(EvalError::DefunRestMustHaveExactlyOneParam);
    }

    Ok((required, optionals, rest))
}

// The condition symbol a `condition-case` handler matches against.
///
/// Every malformed-special-form variant collapses to `error`: none of them is
/// worth naming individually from Lisp, and they all mean the same thing --
/// the form was written wrongly.
pub(super) fn error_symbol<T: LispContext>(err: &EvalError<T>) -> LispExp<T> {
    let name = match err {
        EvalError::Signal { symbol, .. } => return symbol.clone(),
        EvalError::UnboundVariable(_) => "unbound-variable",
        EvalError::UndefinedFunction(_) => "undefined-function",
        EvalError::UnvalidFunctionCall | EvalError::UncorrectFunctionDefinition => {
            "invalid-function"
        }
        EvalError::WrongNumberOfArguments { .. } => "wrong-number-of-arguments",
        EvalError::WrongArgumentType { .. } => "wrong-type-argument",
        EvalError::OutOfFuel => "out-of-fuel",
        EvalError::RuntimeMessage(_) => "runtime-error",
        _ => "error",
    };
    LispExp::symbol(name.into())
}

/// The data a `condition-case` binds alongside the condition symbol.
///
/// Carrying the offending *value* rather than a rendering of it is what the
/// generic error type buys: a handler can look at what actually went wrong.
pub(super) fn error_data<T: LispContext>(err: &EvalError<T>) -> LispExp<T> {
    match err {
        EvalError::Signal { data, .. } => data.clone(),
        EvalError::WrongArgumentType { expected, got } => {
            LispExp::proper_list(vec![LispExp::string(expected.clone()), got.clone()])
        }
        EvalError::UnboundVariable(name) | EvalError::UndefinedFunction(name) => {
            LispExp::proper_list(vec![LispExp::symbol(name.clone())])
        }
        EvalError::WrongNumberOfArguments { expected, got } => LispExp::proper_list(vec![
            LispExp::number(*expected as f64),
            LispExp::number(*got as f64),
        ]),
        EvalError::RuntimeMessage(msg) => LispExp::proper_list(vec![LispExp::string(msg.clone())]),
        other => LispExp::proper_list(vec![LispExp::string(format!("{:?}", other))]),
    }
}

/// Does a handler's condition cover `symbol`? `t` and `error` cover
/// everything; a list covers whatever any of its elements covers.
pub(super) fn condition_matches<T: LispContext>(
    condition: &LispExp<T>,
    symbol: &LispExp<T>,
) -> bool {
    match condition {
        LispExp::Symbol(name) => {
            let name = name.as_str();
            name == "t" || name == "error" || condition == symbol
        }
        // Handler conditions are unevaluated syntax, so a list of them is a
        // `Form`, not a `Cons`.
        LispExp::Form(items) => items.iter().any(|c| condition_matches(c, symbol)),
        _ => false,
    }
}
