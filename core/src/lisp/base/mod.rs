//! The primitives every program in this Lisp has, whatever host it runs in.
//!
//! # What belongs here, and what does not
//!
//! A primitive belongs here when it would mean the same thing in a Lisp with no
//! editor behind it: `car`, `+`, `length`, `funcall`. One that needs a buffer, a
//! window, point or a keymap is an *editor* primitive and belongs in
//! `crate::primitives` -- the test is whether the implementation can be written
//! against `T: LispContext` without knowing what `T` is.
//!
//! The split is enforced, not merely described: `tests::layering_tests` fails if
//! anything under `lisp/` so much as names an editor type.
//!
//! # How to find a primitive
//!
//! By what it is about. One file per kind, and the file's own doc says which
//! kind:
//!
//! | file | what is in it |
//! |------|---------------|
//! | [`functions`] | calling, evaluating, `throw` and `signal` |
//! | [`inspect`] | what is bound, and in which namespace |
//! | [`symbols`] | symbols and their properties |
//! | [`lists`] | cons cells and everything made of them |
//! | [`predicates`] | equality, and asking what something is |
//! | [`math`] | arithmetic |
//! | [`comparisons`] | ordering numbers |
//! | [`strings`] | strings, and converting to and from them |
//! | [`atoms`] | the one mutable thing that crosses a thread |
//! | [`fibers`] | work that can be put down and picked up |
//!
//! This file holds what the ten have in common: the macros that generate the
//! repetitive shapes, the helpers that read an argument, and
//! [`setup_base_env`], which is the list of the ten and nothing else.
//!
//! It was one file of 2,790 lines with those ten as comment banners, and a
//! 389-line registration block at the bottom that named every primitive a
//! second time.

use crate::lisp::{
    Env, EvalError, LispContext, LispExp, Parser, VARIABLE_DOCUMENTATION, bind_lambda_args, eval,
    exact_arity, resume_frames, some_arguments,
};
use std::sync::{Arc, RwLock};

macro_rules! nil {
    () => {
        LispExp::nil()
    };
}
use crate::lisp::{LispPrimitive, SharedAtom};

//
// Four shapes that recur often enough that the three-line signature every
// primitive must carry was most of what the functions below contained. Each
// macro leaves exactly one thing at the call site -- the predicate's pattern,
// the operation's method -- so that what distinguishes two primitives is the
// only thing written about them.
//
// Macros rather than higher-order functions because a primitive is a `fn`
// pointer, not a closure: `LispExp::primitive` takes a bare function, so the
// four arguments and the return type have to be spelled out somewhere, and a
// factory returning `impl Fn` could not be registered.

/// A primitive answering t or nil for one argument's shape.
macro_rules! predicate {
    ($name:ident, $pattern:pat) => {
        fn $name<T: LispContext>(
            args: &[LispExp<T>],
            _env: Arc<Env<T>>,
            _ctx: &T,
        ) -> Result<LispExp<T>, EvalError<T>> {
            exact_arity(args, 1)?;
            Ok(LispExp::boolean(matches!(&args[0], $pattern)))
        }
    };
    // `atom' is `consp' inverted, and saying so is better than a second copy
    // whose only difference is a `!' easily lost while reading.
    ($name:ident, not $pattern:pat) => {
        fn $name<T: LispContext>(
            args: &[LispExp<T>],
            _env: Arc<Env<T>>,
            _ctx: &T,
        ) -> Result<LispExp<T>, EvalError<T>> {
            exact_arity(args, 1)?;
            Ok(LispExp::boolean(!matches!(&args[0], $pattern)))
        }
    };
}

/// A primitive taking one number and giving one back.
macro_rules! number_op {
    ($name:ident, $method:ident) => {
        fn $name<T: LispContext>(
            args: &[LispExp<T>],
            _env: Arc<Env<T>>,
            _ctx: &T,
        ) -> Result<LispExp<T>, EvalError<T>> {
            exact_arity(args, 1)?;
            Ok(LispExp::number(expect_number(&args[0])?.$method()))
        }
    };
}

/// A primitive taking one string and giving one back.
macro_rules! string_op {
    ($name:ident, $method:ident) => {
        fn $name<T: LispContext>(
            args: &[LispExp<T>],
            _env: Arc<Env<T>>,
            _ctx: &T,
        ) -> Result<LispExp<T>, EvalError<T>> {
            exact_arity(args, 1)?;
            Ok(LispExp::string(expect_string(&args[0])?.$method()))
        }
    };
}

/// A primitive folding one or more numbers down to one.
macro_rules! fold_numbers {
    ($name:ident, $method:ident) => {
        fn $name<T: LispContext>(
            args: &[LispExp<T>],
            _env: Arc<Env<T>>,
            _ctx: &T,
        ) -> Result<LispExp<T>, EvalError<T>> {
            some_arguments(args)?;
            let mut folded = expect_number(&args[0])?;
            for arg in &args[1..] {
                folded = folded.$method(expect_number(arg)?);
            }
            Ok(LispExp::number(folded))
        }
    };
}

/// A primitive listing every name in one of the environment's namespaces.
macro_rules! name_list {
    ($name:ident, $names:ident) => {
        fn $name<T: LispContext>(
            _args: &[LispExp<T>],
            env: Arc<Env<T>>,
            _ctx: &T,
        ) -> Result<LispExp<T>, EvalError<T>> {
            Ok(LispExp::proper_list(
                env.$names().into_iter().map(LispExp::string).collect(),
            ))
        }
    };
}

/// A primitive that does nothing but raise its own kind of non-local exit.
macro_rules! raise {
    ($name:ident, $variant:ident { $first:ident, $second:ident }) => {
        fn $name<T: LispContext>(
            args: &[LispExp<T>],
            _env: Arc<Env<T>>,
            _ctx: &T,
        ) -> Result<LispExp<T>, EvalError<T>> {
            exact_arity(args, 2)?;
            Err(EvalError::$variant {
                $first: args[0].clone(),
                $second: args[1].clone(),
            })
        }
    };
}

fn expect_number<T: LispContext>(exp: &LispExp<T>) -> Result<f64, EvalError<T>> {
    if let LispExp::Number(n) = exp {
        Ok(*n)
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: exp.clone(),
        })
    }
}

/// Materialise a data list into a vector.
///
/// Walking a cons chain into a `Vec` is O(n), exactly what the previous
/// `Form(l) => (**l).clone()` cost -- it copied the whole backing vector
/// too. So every caller keeps the complexity it already had, and the ones
/// that want the O(1) chain (`car`, `cdr`, `cons`) simply do not call
/// this. Tightening the remaining callers to walk the chain in place is a
/// separate, independently testable change.
/// Charge the host for `units` of work, saturating rather than wrapping on the
/// (unreachable in practice) list longer than `u32::MAX`.
fn charge<T: LispContext>(ctx: &T, units: usize) -> Result<(), EvalError<T>> {
    ctx.consume_fuel(u32::try_from(units).unwrap_or(u32::MAX))
}

/// Collect a list argument into a vector, charging one unit per element.
///
/// The evaluator charges one unit per *reduction step*, which is the right
/// model for a step-shaped interpreter but silently mis-prices primitives that
/// walk their argument: `(length lst)` is one step whether the list holds three
/// elements or a hundred thousand. That made the budget bound the number of
/// steps rather than the amount of work, so `(while t (length big-list))`
/// burned its whole budget over minutes instead of milliseconds. Charging per
/// element restores the invariant the budget is supposed to provide -- that a
/// runaway command is stopped in bounded *time*.
fn expect_list<T: LispContext>(exp: &LispExp<T>, ctx: &T) -> Result<Vec<LispExp<T>>, EvalError<T>> {
    match exp {
        LispExp::Cons(_) => {
            let items: Vec<LispExp<T>> = exp.iter().collect();
            charge(ctx, items.len())?;
            Ok(items)
        }
        other if other.is_nil() => Ok(vec![]),
        other => Err(EvalError::WrongArgumentType {
            expected: "List".into(),
            got: other.clone(),
        }),
    }
}

/// True for anything Lisp calls a list: a cons chain, or `nil` for the
/// empty one. Used where a primitive rebinds a list-valued variable.
fn is_list_value<T: LispContext>(exp: &LispExp<T>) -> bool {
    matches!(exp, LispExp::Cons(_)) || exp.is_nil()
}

fn expect_string<T: LispContext>(exp: &LispExp<T>) -> Result<String, EvalError<T>> {
    if let LispExp::String(s) = exp {
        Ok((**s).clone())
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: exp.clone(),
        })
    }
}

fn format_number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

/// How a value looks to a person: a string as its own text, a symbol as its
/// name, everything else as the reader would write it.
///
/// Shared so that every route from a value to the user agrees -- `format`'s
/// `%s`, `message`, and `insert`. Two conversions would mean a number reaching
/// the echo area and the same number reaching a buffer could disagree about
/// whether it has a trailing `.0`.
pub fn lisp_display<T: LispContext>(exp: &LispExp<T>) -> String {
    match exp {
        LispExp::String(s) => (**s).clone(),
        LispExp::Symbol(s) => (**s).clone(),
        LispExp::Number(n) => format_number(*n),
        other if other.is_nil() => "nil".into(),
        other => format!("{:?}", other),
    }
}

/// Invokes something callable (a `Lambda`, a `Primitive`, or a symbol naming
/// one in the function namespace) with already-evaluated arguments. Shared
/// by `funcall`, `apply`, `mapcar` and `mapc`, and exported for hosts that
/// need to invoke a Lisp callback from Rust with values they already have
/// in hand (no need to build and `eval` a quoted call AST just to pass
/// already-evaluated arguments back into Lisp).
pub fn call_callable<T: LispContext>(
    func: &LispExp<T>,
    call_args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    match func {
        LispExp::Lambda(lambda) => {
            let call_frame = Env::new_child(&lambda.env);
            bind_lambda_args(lambda, call_args, &call_frame)?;
            if lambda.body.is_empty() {
                return Ok(LispExp::nil());
            }
            for exp in &lambda.body[0..lambda.body.len() - 1] {
                eval(exp, call_frame.clone(), ctx)?;
            }
            eval(
                lambda
                    .body
                    .last()
                    .expect("Failed to get the last expression in the function call"),
                call_frame,
                ctx,
            )
        }
        LispExp::Primitive { pointer: f, doc: _ } => f(call_args, env, ctx),
        LispExp::Symbol(name) => {
            if let Some(resolved) = env.get_function(name) {
                call_callable(&resolved, call_args, env, ctx)
            } else {
                Err(EvalError::UndefinedFunction(name.to_string()))
            }
        }
        _ => Err(EvalError::UncorrectFunctionDefinition),
    }
}

fn find_assoc<T: LispContext>(args: &[LispExp<T>], ctx: &T) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 2)?;
    let list = expect_list(&args[1], ctx)?;
    for entry in list {
        // `(a . 1)` and `(a 1)` are both valid alist entries, and both are
        // now a cons whose car is the key.
        let key = match &entry {
            LispExp::Cons(cell) => Some(cell.car.clone()),
            _ => None,
        };
        if key == Some(args[0].clone()) {
            return Ok(entry);
        }
    }
    Ok(LispExp::nil())
}

fn find_member<T: LispContext>(args: &[LispExp<T>], ctx: &T) -> Result<LispExp<T>, EvalError<T>> {
    exact_arity(args, 2)?;
    let list = expect_list(&args[1], ctx)?;
    if let Some(pos) = list.iter().position(|e| *e == args[0]) {
        Ok(LispExp::proper_list(list[pos..].to_vec()))
    } else {
        Ok(LispExp::nil())
    }
}

fn compare_chain<T: LispContext>(
    args: &[LispExp<T>],
    op: fn(f64, f64) -> bool,
) -> Result<LispExp<T>, EvalError<T>> {
    some_arguments(args)?;
    let mut numbers = Vec::with_capacity(args.len());
    for arg in args {
        numbers.push(expect_number(arg)?);
    }
    Ok(LispExp::boolean(numbers.windows(2).all(|w| op(w[0], w[1]))))
}

// The ten kinds. Declared *below* the macros above and the helpers above that,
// because a `macro_rules!` is in scope for the rest of its own module and for
// the modules declared after it -- and every one of these uses them.
mod atoms;
mod comparisons;
mod fibers;
mod functions;
mod inspect;
mod lists;
mod math;
mod predicates;
mod strings;
mod symbols;

/// What a base-env module registers into.
///
/// The same shape as the editor's [`crate::primitives::Registry`], and
/// deliberately: the two halves of this editor's Lisp surface are installed the
/// same way, so a reader who has seen one has seen both.
///
/// It has no `command` method, and cannot: a command is something a user
/// invokes by name and whose arguments an *editor* collects, and nothing in
/// this directory knows there is an editor.
pub(crate) struct Registry<'a, T: LispContext> {
    env: &'a Arc<Env<T>>,
}

impl<T: LispContext> Registry<'_, T> {
    /// Bind NAME to a primitive.
    ///
    /// DOC must begin with the primitive's own call -- `(name ARGS): what it
    /// does` -- because a primitive has no parameter list anything can read, so
    /// that first line *is* the signature `describe-function` shows.
    pub(crate) fn function(&self, name: &str, pointer: LispPrimitive<T>, doc: &'static str) {
        self.env
            .set_function(name.into(), LispExp::primitive(pointer, Some(doc.into())));
    }
}

/// Install every primitive this Lisp has of its own.
///
/// Each module registers its own, through [`Registry`]; this is the list of
/// modules. Alphabetical, because the order does not matter -- each
/// registration inserts under a name no other uses -- so alphabetical is the
/// order in which a missing module is visible.
pub fn setup_base_env<T: LispContext>(env: Arc<Env<T>>) {
    let into = Registry { env: &env };
    atoms::install(&into);
    comparisons::install(&into);
    fibers::install(&into);
    functions::install(&into);
    inspect::install(&into);
    lists::install(&into);
    math::install(&into);
    predicates::install(&into);
    strings::install(&into);
    symbols::install(&into);
}
