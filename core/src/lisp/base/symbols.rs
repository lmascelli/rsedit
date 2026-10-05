//! Symbols, and what they carry besides a value.
//!
//! A symbol is a name, and a name can have properties hung on it that are
//! neither its value nor its function. `put` and `get` are those.
use super::*;

const PUT_DOC: &str = "(put SYMBOL KEY VALUE): Remember VALUE against SYMBOL under KEY, \
     replacing whatever was there. Returns VALUE.\n\n\
     Properties are global and are not scoped: one set inside a `let' is still there outside \
     it, because a property belongs to the symbol and a symbol is not a binding.\n\n\
     Example:\n\
     (put 'rust-mode 'keywords '(\"fn\" \"let\"))";

const GET_DOC: &str = "(get SYMBOL KEY): What was remembered against SYMBOL under KEY, or nil.\n\n\
     Example:\n\
     (get (major-mode) 'keywords)";

/// A symbol's name, accepting a string too: a caller holding a name should not
/// have to intern it first.
fn property_name<T: LispContext>(exp: &LispExp<T>) -> Option<String> {
    match exp {
        LispExp::Symbol(name) | LispExp::String(name) => Some(name.to_string()),
        _ => None,
    }
}

fn primitive_put<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 3)?;
    let (Some(symbol), Some(key)) = (property_name(&args[0]), property_name(&args[1])) else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol".into(),
            got: args[0].clone(),
        });
    };
    env.put_property(&symbol, &key, args[2].clone());
    Ok(args[2].clone())
}

fn primitive_get<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 2)?;
    let (Some(symbol), Some(key)) = (property_name(&args[0]), property_name(&args[1])) else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol".into(),
            got: args[0].clone(),
        });
    };
    Ok(env.get_property(&symbol, &key).unwrap_or_else(LispExp::nil))
}

/// Register this module's primitives: symbols, and what they carry besides a value.
///
/// Called by [`super::setup_base_env`]. Here rather than there because a
/// primitive's name, its implementation and its docstring are one fact in three
/// pieces, and they were a thousand lines apart.
pub(super) fn install<T: LispContext>(into: &Registry<T>) {
    // Symbol plist
    into.function("put", primitive_put, PUT_DOC);
    into.function("get", primitive_get, GET_DOC);
}
