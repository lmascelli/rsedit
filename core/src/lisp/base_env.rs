use crate::lisp::lisp::SharedAtom;
use crate::lisp::{Env, EvalError, LispContext, LispExp, eval};
use std::sync::{Arc, RwLock};

macro_rules! nil {
    () => {
        LispExp::list(vec![])
    };
}

// -------------------------------- HELPER FUNCTIONS ---------------------------
fn expect_number<T: LispContext>(exp: &LispExp<T>) -> Result<f64, EvalError> {
    if let LispExp::Number(n) = exp {
        Ok(*n)
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: format!("{:?}", exp),
        })
    }
}

fn expect_list<T: LispContext>(exp: &LispExp<T>) -> Result<Vec<LispExp<T>>, EvalError> {
    match exp {
        LispExp::List(l) => Ok((**l).clone()),
        other if other.is_nil() => Ok(vec![]),
        other => Err(EvalError::WrongArgumentType {
            expected: "List".into(),
            got: format!("{:?}", other),
        }),
    }
}

fn expect_string<T: LispContext>(exp: &LispExp<T>) -> Result<String, EvalError> {
    if let LispExp::String(s) = exp {
        Ok((**s).clone())
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: format!("{:?}", exp),
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

fn lisp_display<T: LispContext>(exp: &LispExp<T>) -> String {
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
/// by `funcall`, `apply`, `mapcar` and `mapc`.
fn call_callable<T: LispContext>(
    func: &LispExp<T>,
    call_args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    match func {
        LispExp::Lambda(lambda) => {
            if lambda.params.len() != call_args.len() {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: lambda.params.len(),
                    got: call_args.len(),
                });
            }
            let call_frame = Env::new_child(&lambda.env);
            for (i, param_name) in lambda.params.iter().enumerate() {
                call_frame.set_variable(param_name.clone(), call_args[i].clone());
            }
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
        LispExp::Primitive { pointer: f, doc: _} => f(call_args, env, ctx),
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

fn find_assoc<T: LispContext>(args: &[LispExp<T>]) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let list = expect_list(&args[1])?;
    for entry in list {
        let key = match &entry {
            LispExp::List(pair) => pair.first().cloned(),
            LispExp::DottedList(pair, _) => pair.first().cloned(),
            _ => None,
        };
        if key == Some(args[0].clone()) {
            return Ok(entry);
        }
    }
    Ok(LispExp::nil())
}

fn find_member<T: LispContext>(args: &[LispExp<T>]) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let list = expect_list(&args[1])?;
    if let Some(pos) = list.iter().position(|e| *e == args[0]) {
        Ok(LispExp::list(list[pos..].to_vec()))
    } else {
        Ok(LispExp::nil())
    }
}

fn compare_chain<T: LispContext>(
    args: &[LispExp<T>],
    op: fn(f64, f64) -> bool,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    }
    let mut numbers = Vec::with_capacity(args.len());
    for arg in args {
        numbers.push(expect_number(arg)?);
    }
    Ok(LispExp::boolean(numbers.windows(2).all(|w| op(w[0], w[1]))))
}

// ------------------------------- Functions -----------------------------------

fn primitive_funcall<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    }

    // Because funcall is a normal primitive, its arguments are already evaluated.
    // args[0] is the Lambda object itself, args[1..] are the arguments passed to it.
    let func_obj = &args[0];
    let func_args = &args[1..];

    call_callable(func_obj, func_args, env, ctx)
}

fn primitive_function_doc<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        let err = Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
        ctx.log_diagnostic(&format!("{err:?}"));
        err
    } else {
        if let LispExp::Symbol(func_name) = &args[0] {
            if let Some(func) = env.get_function(&func_name) {
                match func {
                    LispExp::Lambda(lambda) => {
                        let doc = if let Some(doc) = &lambda.doc {
                            doc.to_string()
                        } else {
                            "Undocumented function".into()
                        };
                        Ok(LispExp::string(doc))
                    }
                    LispExp::Primitive{pointer: _, doc: doc} => Ok(LispExp::string(
                        (*doc).clone(),
                    )),
                    _ => unreachable!(),
                }
            } else {
                ctx.log_diagnostic(&format!(
                    "{} function is not present in the environment",
                    func_name.as_str()
                ));
                Ok(nil!())
            }
        } else {
            let err = Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: format!("{:?}", args[0]),
            });
            ctx.log_diagnostic(&format!("{err:?}"));
            err
        }
    }
}

// ----------------------------------- Lists -----------------------------------

fn primitive_add_to_list<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() < 2 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        })
    } else {
        if let LispExp::Symbol(list_name) = &args[0] {
            if let Some(LispExp::List(list)) = env.get_variable(&list_name) {
                let mut new_list = (*list).clone();
                for i in 1..args.len() {
                    if !list.contains(&args[i]) {
                        new_list.insert(0, args[i].clone());
                    }
                }
                env.set_variable(list_name.to_string(), LispExp::list(new_list));
                Ok(args[0].clone())
            } else {
                Err(EvalError::RuntimeMessage(format!(
                    "[ERROR] add-to-list {} is not an existing list",
                    list_name
                )))
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: format!("{:?}", &args[0]),
            })
        }
    }
}

fn primitive_append_to_list<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() < 2 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        })
    } else {
        if let LispExp::Symbol(list_name) = &args[0] {
            if let Some(LispExp::List(list)) = env.get_variable(&list_name) {
                let mut new_list = (*list).clone();
                for i in 1..args.len() {
                    if !list.contains(&args[i]) {
                        new_list.push(args[i].clone());
                    }
                }
                env.set_variable(list_name.to_string(), LispExp::list(new_list));
                Ok(args[0].clone())
            } else {
                Err(EvalError::RuntimeMessage(format!(
                    "[ERROR] append-to-list {} is not an existing list",
                    list_name
                )))
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: format!("{:?}", &args[0]),
            })
        }
    }
}

fn primitive_remove_from_list<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() < 2 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        })
    } else {
        if let LispExp::Symbol(list_name) = &args[0] {
            if let Some(LispExp::List(list)) = env.get_variable(&list_name) {
                let mut new_list = vec![];
                for el in list.iter() {
                    let mut el_found = false;
                    for arg in &args[1..] {
                        if el == arg {
                            el_found = true;
                            break;
                        }
                    }
                    if !el_found {
                        new_list.push(el.clone());
                    }
                }
                env.set_variable(list_name.to_string(), LispExp::list(new_list));
                Ok(args[0].clone())
            } else {
                Err(EvalError::RuntimeMessage(format!(
                    "[ERROR] add-to-list {} is not an existing list",
                    list_name
                )))
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: format!("{:?}", &args[0]),
            })
        }
    }
}

fn primitive_car<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    match &args[0] {
        LispExp::List(l) => Ok(l.first().cloned().unwrap_or_else(LispExp::nil)),
        LispExp::DottedList(l, _) => Ok(l.first().cloned().unwrap_or_else(LispExp::nil)),
        other if other.is_nil() => Ok(LispExp::nil()),
        other => Err(EvalError::WrongArgumentType {
            expected: "List".into(),
            got: format!("{:?}", other),
        }),
    }
}

fn primitive_cdr<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    match &args[0] {
        LispExp::List(l) => {
            if l.is_empty() {
                Ok(LispExp::nil())
            } else {
                Ok(LispExp::list(l[1..].to_vec()))
            }
        }
        LispExp::DottedList(l, tail) => {
            if l.len() <= 1 {
                Ok((**tail).clone())
            } else {
                Ok(LispExp::dotted_list(l[1..].to_vec(), (**tail).clone()))
            }
        }
        other if other.is_nil() => Ok(LispExp::nil()),
        other => Err(EvalError::WrongArgumentType {
            expected: "List".into(),
            got: format!("{:?}", other),
        }),
    }
}

fn primitive_cons<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let head = args[0].clone();
    match &args[1] {
        LispExp::List(l) => {
            let mut new_list = Vec::with_capacity(l.len() + 1);
            new_list.push(head);
            new_list.extend(l.iter().cloned());
            Ok(LispExp::list(new_list))
        }
        LispExp::DottedList(l, tail) => {
            let mut new_list = Vec::with_capacity(l.len() + 1);
            new_list.push(head);
            new_list.extend(l.iter().cloned());
            Ok(LispExp::dotted_list(new_list, (**tail).clone()))
        }
        other if other.is_nil() => Ok(LispExp::list(vec![head])),
        other => Ok(LispExp::dotted_list(vec![head], other.clone())),
    }
}

fn primitive_list<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    Ok(LispExp::list(args.to_vec()))
}

fn primitive_nth<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let n = expect_number(&args[0])?;
    let list = expect_list(&args[1])?;
    if n < 0.0 {
        return Ok(LispExp::nil());
    }
    Ok(list.get(n as usize).cloned().unwrap_or_else(LispExp::nil))
}

fn primitive_nthcdr<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let n = expect_number(&args[0])?.max(0.0) as usize;
    let list = expect_list(&args[1])?;
    if n >= list.len() {
        Ok(LispExp::nil())
    } else {
        Ok(LispExp::list(list[n..].to_vec()))
    }
}

fn primitive_length<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let len = match &args[0] {
        LispExp::List(l) => l.len(),
        LispExp::Vector(v) => v.len(),
        LispExp::String(s) => s.chars().count(),
        other if other.is_nil() => 0,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Sequence".into(),
                got: format!("{:?}", other),
            });
        }
    };
    Ok(LispExp::number(len as f64))
}

fn primitive_append<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Ok(LispExp::nil());
    }
    let mut result = Vec::new();
    for arg in &args[0..args.len() - 1] {
        result.extend(expect_list(arg)?);
    }
    match &args[args.len() - 1] {
        LispExp::List(l) => {
            result.extend(l.iter().cloned());
            Ok(LispExp::list(result))
        }
        other if other.is_nil() => Ok(LispExp::list(result)),
        other => Ok(LispExp::dotted_list(result, other.clone())),
    }
}

fn primitive_reverse<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    match &args[0] {
        LispExp::List(l) => {
            let mut v = (**l).clone();
            v.reverse();
            Ok(LispExp::list(v))
        }
        LispExp::Vector(v) => {
            let mut v = (**v).clone();
            v.reverse();
            Ok(LispExp::vec(v))
        }
        LispExp::String(s) => Ok(LispExp::string(s.chars().rev().collect())),
        other if other.is_nil() => Ok(LispExp::nil()),
        other => Err(EvalError::WrongArgumentType {
            expected: "Sequence".into(),
            got: format!("{:?}", other),
        }),
    }
}

fn primitive_member<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    find_member(args)
}

fn primitive_memq<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    find_member(args)
}

fn primitive_assoc<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    find_assoc(args)
}

fn primitive_assq<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    find_assoc(args)
}

fn primitive_elt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let n = expect_number(&args[1])?;
    if n < 0.0 {
        return Ok(LispExp::nil());
    }
    let n = n as usize;
    match &args[0] {
        LispExp::List(l) => Ok(l.get(n).cloned().unwrap_or_else(LispExp::nil)),
        LispExp::Vector(v) => Ok(v.get(n).cloned().unwrap_or_else(LispExp::nil)),
        LispExp::String(s) => Ok(s
            .chars()
            .nth(n)
            .map(|c| LispExp::string(c.to_string()))
            .unwrap_or_else(LispExp::nil)),
        other if other.is_nil() => Ok(LispExp::nil()),
        other => Err(EvalError::WrongArgumentType {
            expected: "Sequence".into(),
            got: format!("{:?}", other),
        }),
    }
}

fn primitive_mapcar<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let list = expect_list(&args[1])?;
    let mut result = Vec::with_capacity(list.len());
    for item in list {
        result.push(call_callable(&args[0], &[item], env.clone(), ctx)?);
    }
    Ok(LispExp::list(result))
}

fn primitive_mapc<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let list = expect_list(&args[1])?;
    for item in &list {
        call_callable(&args[0], std::slice::from_ref(item), env.clone(), ctx)?;
    }
    Ok(args[1].clone())
}

fn primitive_apply<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    }
    let func = &args[0];
    if args.len() == 1 {
        return call_callable(func, &[], env, ctx);
    }
    let mut call_args = args[1..args.len() - 1].to_vec();
    call_args.extend(expect_list(&args[args.len() - 1])?);
    call_callable(func, &call_args, env, ctx)
}

fn primitive_identity<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(args[0].clone())
}

// ----------------------------- Predicates ------------------------------------

fn primitive_equal_impl<T: LispContext>(args: &[LispExp<T>]) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let is_eq = match (&args[0], &args[1]) {
        // Immediate values: compared by value, mirroring how real Elisp's
        // interned symbols and fixnums behave under `eq`.
        (LispExp::Number(a), LispExp::Number(b)) => a == b,
        (LispExp::Symbol(a), LispExp::Symbol(b)) => a == b,
        (a, b) if a.is_nil() && b.is_nil() => true,
        // Compound values: only `eq` if they're literally the same
        // allocation, not just structurally identical.
        (LispExp::List(a), LispExp::List(b)) => Arc::ptr_eq(a, b),
        (LispExp::DottedList(a, ta), LispExp::DottedList(b, tb)) => {
            Arc::ptr_eq(a, b) && Arc::ptr_eq(ta, tb)
        }
        (LispExp::Vector(a), LispExp::Vector(b)) => Arc::ptr_eq(a, b),
        (LispExp::Map(a), LispExp::Map(b)) => Arc::ptr_eq(a, b),
        (LispExp::String(a), LispExp::String(b)) => Arc::ptr_eq(a, b),
        _ => false,
    };
    Ok(LispExp::boolean(is_eq))
}

fn primitive_eq<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    primitive_equal_impl(args)
}

fn primitive_eql<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    primitive_equal_impl(args)
}

fn primitive_equal<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    primitive_equal_impl(args)
}

fn primitive_null<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(args[0].is_nil()))
}

fn primitive_not<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    // `not` and `null` are the same operation in Elisp.
    primitive_null(args, env, ctx)
}

fn primitive_consp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let is_cons = match &args[0] {
        LispExp::List(l) => !l.is_empty(),
        LispExp::DottedList(_, _) => true,
        _ => false,
    };
    Ok(LispExp::boolean(is_cons))
}

fn primitive_listp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let is_list = args[0].is_nil() || matches!(&args[0], LispExp::List(_) | LispExp::DottedList(_, _));
    Ok(LispExp::boolean(is_list))
}

fn primitive_stringp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(matches!(&args[0], LispExp::String(_))))
}

fn primitive_numberp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(matches!(&args[0], LispExp::Number(_))))
}

fn primitive_symbolp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(matches!(&args[0], LispExp::Symbol(_))))
}

fn primitive_functionp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let is_function = match &args[0] {
        LispExp::Lambda(_) | LispExp::Primitive{ .. } => true,
        LispExp::Symbol(name) => env.get_function(name).is_some(),
        _ => false,

    };
    Ok(LispExp::boolean(is_function))
}

fn primitive_vectorp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(matches!(&args[0], LispExp::Vector(_))))
}

fn primitive_zerop<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(expect_number(&args[0])? == 0.0))
}

fn primitive_atom_predicate<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let is_cons = matches!(&args[0], LispExp::List(l) if !l.is_empty())
        || matches!(&args[0], LispExp::DottedList(_, _));
    Ok(LispExp::boolean(!is_cons))
}

// --------------------------------- Math --------------------------------------

fn primitive_sum<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    let mut sum = 0.0;
    for arg in args {
        if let LispExp::Number(number) = arg {
            sum += number;
        } else {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: format!("{:?}", arg),
            });
        }
    }
    Ok(LispExp::number(sum))
}

fn primitive_subtraction<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
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
            for args in &args[1..] {
                result -= expect_number(arg)?;
            }
            Ok(LispExp::number(result))
        }
    }
}

fn primitive_mul<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    let mut product = 1.0;
    for arg in args {
        product *= expect_number(arg)?;
    }
    Ok(LispExp::number(product))
}

fn primitive_div<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    }
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
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
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

fn primitive_1plus<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::number(expect_number(&args[0])? + 1.0))
}

fn primitive_1minus<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::number(expect_number(&args[0])? - 1.0))
}

// ------------------------------- Comparisons ---------------------------------

fn primitive_compare<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    }
    let mut numbers = Vec::with_capacity(args.len());
    for arg in args {
        if let LispExp::Number(n) = arg {
            numbers.push(*n);
        } else {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: format!("{:?}", arg),
            });
        }
    }
    Ok(LispExp::boolean(numbers.windows(2).all(|w| w[0] == w[1])))
}

fn primitive_lt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    compare_chain(args, |a, b| a < b)
}

fn primitive_gt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    compare_chain(args, |a, b| a > b)
}

fn primitive_le<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    compare_chain(args, |a, b| a <= b)
}

fn primitive_ge<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    compare_chain(args, |a, b| a >= b)
}

fn primitive_max<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    }
    let mut m = expect_number(&args[0])?;
    for arg in &args[1..] {
        m = m.max(expect_number(arg)?);
    }
    Ok(LispExp::number(m))
}

fn primitive_min<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    }
    let mut m = expect_number(&args[0])?;
    for arg in &args[1..] {
        m = m.min(expect_number(arg)?);
    }
    Ok(LispExp::number(m))
}

fn primitive_abs<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::number(expect_number(&args[0])?.abs()))
}

// ----------------------------------- Strings ---------------------------------

fn primitive_concat<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    let mut result = String::new();
    for arg in args {
        result.push_str(&expect_string(arg)?);
    }
    Ok(LispExp::string(result))
}

fn primitive_string_eq<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(
        expect_string(&args[0])? == expect_string(&args[1])?,
    ))
}

fn primitive_string_lt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(
        expect_string(&args[0])? < expect_string(&args[1])?,
    ))
}

fn primitive_substring<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() || args.len() > 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let s = expect_string(&args[0])?;
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;
    let normalize = |i: i64| -> i64 { if i < 0 { (len + i).max(0) } else { i.min(len) } };

    let start = if args.len() >= 2 {
        normalize(expect_number(&args[1])? as i64)
    } else {
        0
    };
    let end = if args.len() == 3 {
        normalize(expect_number(&args[2])? as i64)
    } else {
        len
    };
    if start > end {
        return Err(EvalError::RuntimeMessage(
            "Args out of range for substring".into(),
        ));
    }
    Ok(LispExp::string(
        chars[start as usize..end as usize].iter().collect(),
    ))
}

fn primitive_upcase<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::string(expect_string(&args[0])?.to_uppercase()))
}

fn primitive_downcase<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::string(expect_string(&args[0])?.to_lowercase()))
}

fn primitive_number_to_string<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::string(format_number(expect_number(&args[0])?)))
}

fn primitive_string_to_number<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::number(
        expect_string(&args[0])?.trim().parse().unwrap_or(0.0),
    ))
}

fn primitive_symbol_name<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    if let LispExp::Symbol(s) = &args[0] {
        Ok(LispExp::string(s.to_string()))
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "Symbol".into(),
            got: format!("{:?}", args[0]),
        })
    }
}

fn primitive_intern<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::symbol(expect_string(&args[0])?))
}

fn primitive_split_string<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let s = expect_string(&args[0])?;
    let parts: Vec<LispExp<T>> = if args.len() == 2 {
        let sep = expect_string(&args[1])?;
        if sep.is_empty() {
            s.chars().map(|c| LispExp::string(c.to_string())).collect()
        } else {
            s.split(sep.as_str())
                .map(|p| LispExp::string(p.to_string()))
                .collect()
        }
    } else {
        s.split_whitespace()
            .map(|p| LispExp::string(p.to_string()))
            .collect()
    };
    Ok(LispExp::list(parts))
}

fn primitive_format<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    }
    let fmt = expect_string(&args[0])?;
    let mut result = String::new();
    let mut arg_idx = 1;
    let mut chars = fmt.chars();

    let next_arg = |idx: &mut usize| -> Result<&LispExp<T>, EvalError> {
        let val = args.get(*idx).ok_or(EvalError::WrongNumberOfArguments {
            expected: *idx + 1,
            got: args.len(),
        })?;
        *idx += 1;
        Ok(val)
    };

    while let Some(c) = chars.next() {
        if c != '%' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => result.push('%'),
            Some('s') | Some('S') => {
                result.push_str(&lisp_display(next_arg(&mut arg_idx)?));
            }
            Some('d') => {
                result.push_str(&format!(
                    "{}",
                    expect_number(next_arg(&mut arg_idx)?)? as i64
                ));
            }
            Some('f') => {
                result.push_str(&format!("{}", expect_number(next_arg(&mut arg_idx)?)?));
            }
            Some(other) => {
                result.push('%');
                result.push(other);
            }
            None => result.push('%'),
        }
    }
    Ok(LispExp::string(result))
}

// -------------------------------- MULTI-THREADING ----------------------------

fn primitive_make_atom<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        })
    } else {
        Ok(LispExp::Atom(SharedAtom(Arc::new(RwLock::new(
            args[0].clone(),
        )))))
    }
}

fn primitive_deref<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        })
    } else {
        if let LispExp::Atom(atom_lock) = &args[0] {
            let guard = atom_lock
                .0
                .read()
                .map_err(|_| EvalError::UncorrectFunctionDefinition)?;
            Ok(guard.clone())
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Atom".into(),
                got: format!("{:?}", args[0]),
            })
        }
    }
}

fn primitive_reset<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() < 2 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        })
    } else {
        if let LispExp::Atom(atom_lock) = &args[0] {
            let new_val = &args[1];
            let mut guard = atom_lock
                .0
                .write()
                .map_err(|_| EvalError::UncorrectFunctionDefinition)?;
            *guard = new_val.clone();
            Ok(new_val.clone())
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Atom".into(),
                got: format!("{:?}", args[0]),
            })
        }
    }
}

// -------------------------------- CONCURRENCY --------------------------------

fn primitive_resume<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.is_empty() {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    } else {
        if let LispExp::Fiber(shared_fiber) = &args[0] {
            let mut fiber = shared_fiber
                .0
                .write()
                .map_err(|_| EvalError::UncorrectFunctionDefinition)?;

            if fiber.is_done {
                return Ok(LispExp::symbol("nil".into()));
            }

            if fiber.body.is_empty() {
                fiber.is_done = true;
                return Ok(LispExp::symbol("nil".into()));
            }

            let next_exp = fiber.body.remove(0);

            if fiber.body.is_empty() {
                fiber.is_done = true;
            }
            eval(&next_exp, fiber.env.clone(), ctx)
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Fiber".into(),
                got: format!("{:?}", args[0]),
            })
        }
    }
}

// -------------------------------- CONSTRUCTOR --------------------------------
pub fn setup_base_env<T: LispContext>(env: std::sync::Arc<Env<T>>) {
    // ------------------------------- Functions  ------------------------------
    // Functions
    env.set_function(
        "funcall".into(),
        LispExp::primitive(
            primitive_funcall,
            Some(
                "(funcall FUNCTION &rest ARGS): Call FUNCTION with ARGS. \
                 Unlike `apply`/`mapcar`, FUNCTION must already be a callable \
                 value (a lambda or a primitive) -- a symbol naming a function \
                 is NOT resolved and calling one signals an error.\n\n\
                 Example:\n\
                 (funcall (lambda (x y) (+ x y)) 2 3) => 5\n\
                 (funcall 'car '(1 2))                => error, 'car is a \
                 symbol, not a callable value; use (funcall #'car '(1 2)) or \
                 `apply` instead."
                    .into(),
            ),
        ),
    );
    env.set_function(
        "function-doc".into(),
        LispExp::primitive(
            primitive_function_doc,
            Some(
                "(function-doc SYMBOL): Return the documentation string of the \
                 function bound to SYMBOL, or \"Undocumented function\" if it \
                 has none. Returns nil and logs a diagnostic if SYMBOL names no \
                 function. Not a standard Elisp primitive -- the closest real \
                 Elisp equivalent is `documentation`.\n\n\
                 Example:\n\
                 (function-doc 'car) => \"(car LIST): Return the first \
                 element of LIST, or nil if LIST is nil.\"\n\
                 (function-doc 'no-such-fn) => nil"
                    .into(),
            ),
        ),
    );
    // Multithreading
    env.set_function("make-atom".into(), LispExp::primitive(primitive_make_atom, None));
    env.set_function(
        "deref".into(),
        LispExp::primitive(
            primitive_deref,
            Some(
                "(deref ATOM): Return the current value stored in ATOM.\n\n\
                 Example:\n\
                 (setq counter (atom 0))\n\
                 (deref counter) => 0"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "reset".into(),
        LispExp::primitive(
            primitive_reset,
            Some(
                "(reset ATOM NEWVAL): Set ATOM's stored value to NEWVAL and \
                 return NEWVAL.\n\n\
                 Example:\n\
                 (setq counter (atom 0))\n\
                 (reset counter 42) => 42\n\
                 (deref counter)    => 42"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "resume".into(),
        LispExp::primitive(
            primitive_resume,
            Some(
                "(resume FIBER): Run the next suspended expression of FIBER \
                 and return its value. Returns nil once FIBER has no \
                 expressions left to run.\n\n\
                 Example:\n\
                 (setq f (make-fiber '((log \"a\") (log \"b\"))))\n\
                 (resume f) ; runs (log \"a\")\n\
                 (resume f) ; runs (log \"b\")\n\
                 (resume f) => nil ; fiber is done"
                    .into(),
            ),
        ),
    );

    // Base math
    // `+`, `-` and `mod`/`%` are deliberately left undocumented: `(+)` should
    // return 0 but currently errors, `(- 5)` should negate to -5 but returns
    // 5 unchanged, and `mod` doesn't follow Elisp's "result takes the sign of
    // the divisor" rule for a negative divisor. Documenting them now would
    // just describe the bugs as if they were the intended behavior.
    env.set_function("+".into(), LispExp::primitive(primitive_sum, None));
    env.set_function("-".into(), LispExp::primitive(primitive_subtraction, None));
    env.set_function(
        "=".into(),
        LispExp::primitive(
            primitive_compare,
            Some(
                "(= NUMBER &rest NUMBERS): Return t if all arguments are \
                 numerically equal, nil otherwise. Requires at least one \
                 argument.\n\n\
                 Example:\n\
                 (= 1 1 1) => t\n\
                 (= 1 1 2) => nil\n\
                 (= 3)     => t"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "*".into(),
        LispExp::primitive(
            primitive_mul,
            Some(
                "(* &rest NUMBERS): Return the product of NUMBERS. With no \
                 arguments, returns 1.\n\n\
                 Example:\n\
                 (* 2 3 4) => 24\n\
                 (*)       => 1"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "/".into(),
        LispExp::primitive(
            primitive_div,
            Some(
                "(/ NUMBER &rest DIVISORS): Divide NUMBER by each of DIVISORS \
                 in turn. With a single argument, returns its reciprocal. \
                 Signals a runtime error on division by zero.\n\n\
                 Example:\n\
                 (/ 20 2 5) => 2\n\
                 (/ 4)      => 0.25\n\
                 (/ 1 0)    => error, division by zero"
                    .into(),
            ),
        ),
    );
    env.set_function("mod".into(), LispExp::primitive(primitive_mod, None));
    env.set_function("%".into(), LispExp::primitive(primitive_mod, None));
    env.set_function(
        "1+".into(),
        LispExp::primitive(
            primitive_1plus,
            Some(
                "(1+ NUMBER): Return NUMBER plus one.\n\n\
                 Example:\n\
                 (1+ 4) => 5"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "1-".into(),
        LispExp::primitive(
            primitive_1minus,
            Some(
                "(1- NUMBER): Return NUMBER minus one.\n\n\
                 Example:\n\
                 (1- 4) => 3"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "<".into(),
        LispExp::primitive(
            primitive_lt,
            Some(
                "(< NUMBER &rest NUMBERS): Return t if the arguments are in \
                 strictly increasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (< 1 2 3) => t\n\
                 (< 1 3 2) => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        ">".into(),
        LispExp::primitive(
            primitive_gt,
            Some(
                "(> NUMBER &rest NUMBERS): Return t if the arguments are in \
                 strictly decreasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (> 3 2 1) => t\n\
                 (> 3 1 2) => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "<=".into(),
        LispExp::primitive(
            primitive_le,
            Some(
                "(<= NUMBER &rest NUMBERS): Return t if the arguments are in \
                 non-decreasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (<= 1 1 2) => t\n\
                 (<= 2 1)   => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        ">=".into(),
        LispExp::primitive(
            primitive_ge,
            Some(
                "(>= NUMBER &rest NUMBERS): Return t if the arguments are in \
                 non-increasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (>= 2 1 1) => t\n\
                 (>= 1 2)   => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "max".into(),
        LispExp::primitive(
            primitive_max,
            Some(
                "(max NUMBER &rest NUMBERS): Return the largest of the \
                 arguments.\n\n\
                 Example:\n\
                 (max 1 5 3) => 5"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "min".into(),
        LispExp::primitive(
            primitive_min,
            Some(
                "(min NUMBER &rest NUMBERS): Return the smallest of the \
                 arguments.\n\n\
                 Example:\n\
                 (min 1 5 3) => 1"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "abs".into(),
        LispExp::primitive(
            primitive_abs,
            Some(
                "(abs NUMBER): Return the absolute value of NUMBER.\n\n\
                 Example:\n\
                 (abs -5) => 5\n\
                 (abs 5)  => 5"
                    .into(),
            ),
        ),
    );

    // List manipulation
    env.set_function(
        "add-to-list".into(),
        LispExp::primitive(
            primitive_add_to_list,
            Some(
                "(add-to-list LIST-VAR &rest ELEMENTS): Prepend each of \
                 ELEMENTS not already `member` of the list bound to LIST-VAR, \
                 and rebind LIST-VAR to the result. Returns LIST-VAR. Unlike \
                 real Elisp's `add-to-list`, this accepts several ELEMENTS at \
                 once and has no APPEND or COMPARE-FN argument. LIST-VAR must \
                 already be bound to a list.\n\n\
                 Example:\n\
                 (defvar my-list '(2 3))\n\
                 (add-to-list 'my-list 1) => my-list\n\
                 my-list                  => (1 2 3)\n\
                 (add-to-list 'my-list 1) ; 1 already present, unchanged\n\
                 my-list                  => (1 2 3)"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "append-to-list".into(),
        LispExp::primitive(
            primitive_append_to_list,
            Some(
                "(append-to-list LIST-VAR &rest ELEMENTS): Like `add-to-list`, \
                 but appends each of ELEMENTS not already present to the end \
                 of the list bound to LIST-VAR instead of the front. Not a \
                 standard Elisp primitive.\n\n\
                 Example:\n\
                 (defvar my-list '(1 2))\n\
                 (append-to-list 'my-list 3) => my-list\n\
                 my-list                     => (1 2 3)"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "remove-from-list".into(),
        LispExp::primitive(
            primitive_remove_from_list,
            Some(
                "(remove-from-list LIST-VAR &rest ELEMENTS): Rebind LIST-VAR \
                 to a copy of its list with every element `equal` to one of \
                 ELEMENTS removed. Returns LIST-VAR. Not a standard Elisp \
                 primitive.\n\n\
                 Example:\n\
                 (defvar my-list '(1 2 3 4))\n\
                 (remove-from-list 'my-list 2 4) => my-list\n\
                 my-list                         => (1 3)"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "car".into(),
        LispExp::primitive(
            primitive_car,
            Some(
                "(car LIST): Return the first element of LIST, or nil if LIST \
                 is nil.\n\n\
                 Example:\n\
                 (car '(1 2 3)) => 1\n\
                 (car nil)      => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "cdr".into(),
        LispExp::primitive(
            primitive_cdr,
            Some(
                "(cdr LIST): Return LIST with its first element removed, or \
                 nil if LIST is nil or has one element. On a dotted pair, \
                 returns the tail.\n\n\
                 Example:\n\
                 (cdr '(1 2 3))   => (2 3)\n\
                 (cdr '(1))       => nil\n\
                 (cdr (cons 1 2)) => 2"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "cons".into(),
        LispExp::primitive(
            primitive_cons,
            Some(
                "(cons CAR CDR): Construct a new cons cell with CAR as its \
                 first element and CDR as its rest. If CDR is a list, the \
                 result is a proper list; if CDR is anything else (other than \
                 nil), the result is a dotted pair.\n\n\
                 Example:\n\
                 (cons 1 '(2 3)) => (1 2 3)\n\
                 (cons 1 2)      => (1 . 2)"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "list".into(),
        LispExp::primitive(
            primitive_list,
            Some(
                "(list &rest ARGS): Return a newly built list containing \
                 ARGS.\n\n\
                 Example:\n\
                 (list 1 2 3) => (1 2 3)\n\
                 (list)       => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "nth".into(),
        LispExp::primitive(
            primitive_nth,
            Some(
                "(nth N LIST): Return the Nth element of LIST (zero-indexed), \
                 or nil if N is negative or past the end of LIST.\n\n\
                 Example:\n\
                 (nth 0 '(a b c)) => a\n\
                 (nth 2 '(a b c)) => c\n\
                 (nth 5 '(a b c)) => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "nthcdr".into(),
        LispExp::primitive(
            primitive_nthcdr,
            Some(
                "(nthcdr N LIST): Return LIST with its first N elements \
                 removed. A negative N is treated as 0. Returns nil once N is \
                 at or past the end of LIST.\n\n\
                 Example:\n\
                 (nthcdr 2 '(1 2 3 4)) => (3 4)\n\
                 (nthcdr 99 '(1 2 3))  => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "length".into(),
        LispExp::primitive(
            primitive_length,
            Some(
                "(length SEQUENCE): Return the number of elements in \
                 SEQUENCE, which may be a list, vector, string, or nil (0).\n\n\
                 Example:\n\
                 (length '(1 2 3)) => 3\n\
                 (length \"abc\")    => 3\n\
                 (length nil)      => 0"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "append".into(),
        LispExp::primitive(
            primitive_append,
            Some(
                "(append &rest SEQUENCES): Concatenate all the given \
                 SEQUENCES into a list. If the final SEQUENCE is not a proper \
                 list, the result is a dotted list ending in that value. With \
                 no arguments, returns nil.\n\n\
                 Example:\n\
                 (append '(1 2) '(3 4)) => (1 2 3 4)\n\
                 (append '(1 2) 3)      => (1 2 . 3)\n\
                 (append)               => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "reverse".into(),
        LispExp::primitive(
            primitive_reverse,
            Some(
                "(reverse SEQUENCE): Return a new sequence with the elements \
                 of SEQUENCE (a list, vector, or string) in reverse order.\n\n\
                 Example:\n\
                 (reverse '(1 2 3)) => (3 2 1)\n\
                 (reverse \"abc\")    => \"cba\""
                    .into(),
            ),
        ),
    );
    env.set_function(
        "member".into(),
        LispExp::primitive(
            primitive_member,
            Some(
                "(member ELEMENT LIST): Return the first sublist of LIST whose \
                 car is `equal` to ELEMENT, or nil if not found.\n\n\
                 Example:\n\
                 (member 2 '(1 2 3)) => (2 3)\n\
                 (member 9 '(1 2 3)) => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "memq".into(),
        LispExp::primitive(
            primitive_memq,
            Some(
                "(memq ELEMENT LIST): Return the first sublist of LIST whose \
                 car matches ELEMENT, or nil if not found. Since this \
                 implementation's `eq` is structural rather than \
                 identity-based, `memq` currently behaves the same as \
                 `member`.\n\n\
                 Example:\n\
                 (memq 'b '(a b c)) => (b c)"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "assoc".into(),
        LispExp::primitive(
            primitive_assoc,
            Some(
                "(assoc KEY ALIST): Return the first element of ALIST (an \
                 association list of cons cells or lists) whose car is `equal` \
                 to KEY, or nil if not found.\n\n\
                 Example:\n\
                 (assoc 'b '((a . 1) (b . 2))) => (b . 2)\n\
                 (assoc 'z '((a . 1) (b . 2))) => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "assq".into(),
        LispExp::primitive(
            primitive_assq,
            Some(
                "(assq KEY ALIST): Like `assoc`, but intended to compare KEY \
                 with `eq`. As with `memq`, this currently behaves the same as \
                 `assoc` since `eq` is structural here.\n\n\
                 Example:\n\
                 (assq 'b '((a . 1) (b . 2))) => (b . 2)"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "elt".into(),
        LispExp::primitive(
            primitive_elt,
            Some(
                "(elt SEQUENCE N): Return the Nth element of SEQUENCE (a list, \
                 vector, or string), or nil if N is negative or past the end \
                 of SEQUENCE.\n\n\
                 Example:\n\
                 (elt '(a b c) 1) => b\n\
                 (elt \"abc\" 1)    => \"b\""
                    .into(),
            ),
        ),
    );
    env.set_function(
        "mapcar".into(),
        LispExp::primitive(
            primitive_mapcar,
            Some(
                "(mapcar FUNCTION LIST): Apply FUNCTION to each element of \
                 LIST in turn and return a list of the results. FUNCTION may \
                 be a lambda, a primitive, or a symbol naming a function.\n\n\
                 Example:\n\
                 (mapcar '1+ '(1 2 3))              => (2 3 4)\n\
                 (mapcar (lambda (x) (* x x)) '(1 2 3)) => (1 4 9)"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "mapc".into(),
        LispExp::primitive(
            primitive_mapc,
            Some(
                "(mapc FUNCTION LIST): Apply FUNCTION to each element of LIST \
                 for its side effects and return LIST unchanged.\n\n\
                 Example:\n\
                 (mapc (lambda (x) (log (number-to-string x))) '(1 2 3))\n\
                 ; logs \"1\", \"2\", \"3\" and returns (1 2 3)"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "apply".into(),
        LispExp::primitive(
            primitive_apply,
            Some(
                "(apply FUNCTION &rest ARGS LIST): Call FUNCTION with ARGS \
                 followed by the elements of the final LIST argument, all \
                 spliced together. FUNCTION may be a lambda, a primitive, or a \
                 symbol naming a function.\n\n\
                 Example:\n\
                 (apply '+ '(1 2 3))       => 6\n\
                 (apply '+ 1 2 '(3 4))     => 10"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "identity".into(),
        LispExp::primitive(
            primitive_identity,
            Some(
                "(identity ARG): Return ARG unchanged.\n\n\
                 Example:\n\
                 (identity 42) => 42"
                    .into(),
            ),
        ),
    );

    // ------------------------------- Predicates ------------------------------
    // `eq`/`eql` are left undocumented: both are currently implemented as
    // structural (deep) equality, the same as `equal`, rather than the
    // identity/type-aware comparisons real Elisp specifies. `functionp` is
    // also left undocumented: real Elisp resolves a symbol argument through
    // `fboundp` (so `(functionp 'car)` is t), but this implementation only
    // recognizes already-callable values, so it always says nil for a quoted
    // symbol.
    env.set_function("eq".into(), LispExp::primitive(primitive_eq, None));
    env.set_function("eql".into(), LispExp::primitive(primitive_eql, None));
    env.set_function(
        "equal".into(),
        LispExp::primitive(
            primitive_equal,
            Some(
                "(equal A B): Return t if A and B have the same structure and \
                 contents (deep, structural comparison), nil otherwise.\n\n\
                 Example:\n\
                 (equal '(1 2) '(1 2)) => t\n\
                 (equal \"ab\" \"ab\")    => t\n\
                 (equal '(1 2) '(1 3)) => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "null".into(),
        LispExp::primitive(
            primitive_null,
            Some(
                "(null OBJECT): Return t if OBJECT is nil, nil otherwise.\n\n\
                 Example:\n\
                 (null nil)   => t\n\
                 (null '(1))  => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "not".into(),
        LispExp::primitive(
            primitive_not,
            Some(
                "(not OBJECT): Return t if OBJECT is nil, nil otherwise. \
                 Identical to `null`.\n\n\
                 Example:\n\
                 (not nil) => t\n\
                 (not t)   => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "consp".into(),
        LispExp::primitive(
            primitive_consp,
            Some(
                "(consp OBJECT): Return t if OBJECT is a cons cell -- a \
                 non-empty list or a dotted pair -- nil otherwise.\n\n\
                 Example:\n\
                 (consp '(1 2)) => t\n\
                 (consp nil)    => nil\n\
                 (consp 5)      => nil"
                    .into(),
            ),
        ),
    );
    env.set_function("listp".into(), LispExp::primitive(primitive_listp, None));
    env.set_function(
        "stringp".into(),
        LispExp::primitive(
            primitive_stringp,
            Some(
                "(stringp OBJECT): Return t if OBJECT is a string, nil \
                 otherwise.\n\n\
                 Example:\n\
                 (stringp \"hi\") => t\n\
                 (stringp 5)    => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "numberp".into(),
        LispExp::primitive(
            primitive_numberp,
            Some(
                "(numberp OBJECT): Return t if OBJECT is a number, nil \
                 otherwise.\n\n\
                 Example:\n\
                 (numberp 5)     => t\n\
                 (numberp \"5\")   => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "symbolp".into(),
        LispExp::primitive(
            primitive_symbolp,
            Some(
                "(symbolp OBJECT): Return t if OBJECT is a symbol, nil \
                 otherwise.\n\n\
                 Example:\n\
                 (symbolp 'foo) => t\n\
                 (symbolp \"foo\") => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "functionp".into(),
        LispExp::primitive(primitive_functionp, None),
    );
    env.set_function(
        "vectorp".into(),
        LispExp::primitive(
            primitive_vectorp,
            Some(
                "(vectorp OBJECT): Return t if OBJECT is a vector, nil \
                 otherwise.\n\n\
                 Example:\n\
                 (vectorp [1 2 3]) => t\n\
                 (vectorp '(1 2))  => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "zerop".into(),
        LispExp::primitive(
            primitive_zerop,
            Some(
                "(zerop NUMBER): Return t if NUMBER is zero, nil otherwise.\n\n\
                 Example:\n\
                 (zerop 0) => t\n\
                 (zerop 1) => nil"
                    .into(),
            ),
        ),
    );

    env.set_function(
    "atom".into(),
    LispExp::primitive(primitive_atom_predicate, None),
    );

    // ------------------------------- Strings ------------------------------
    env.set_function(
        "concat".into(),
        LispExp::primitive(
            primitive_concat,
            Some(
                "(concat &rest STRINGS): Concatenate STRINGS into a single \
                 string.\n\n\
                 Example:\n\
                 (concat \"foo\" \"-\" \"bar\") => \"foo-bar\""
                    .into(),
            ),
        ),
    );
    env.set_function(
        "string=".into(),
        LispExp::primitive(
            primitive_string_eq,
            Some(
                "(string= STRING1 STRING2): Return t if STRING1 and STRING2 \
                 have the same contents, nil otherwise.\n\n\
                 Example:\n\
                 (string= \"foo\" \"foo\") => t\n\
                 (string= \"foo\" \"bar\") => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "string<".into(),
        LispExp::primitive(
            primitive_string_lt,
            Some(
                "(string< STRING1 STRING2): Return t if STRING1 sorts before \
                 STRING2 lexicographically, nil otherwise.\n\n\
                 Example:\n\
                 (string< \"abc\" \"abd\") => t\n\
                 (string< \"abd\" \"abc\") => nil"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "substring".into(),
        LispExp::primitive(
            primitive_substring,
            Some(
                "(substring STRING &optional START END): Return the substring \
                 of STRING from START (inclusive, default 0) to END \
                 (exclusive, default the length of STRING). Negative indices \
                 count from the end of STRING. Signals a runtime error if \
                 START is after END.\n\n\
                 Example:\n\
                 (substring \"hello\" 1 3)  => \"el\"\n\
                 (substring \"hello\" -3)   => \"llo\"\n\
                 (substring \"hello\" -3 -1) => \"ll\""
                    .into(),
            ),
        ),
    );
    env.set_function(
        "upcase".into(),
        LispExp::primitive(
            primitive_upcase,
            Some(
                "(upcase STRING): Return a copy of STRING with all letters \
                 uppercased.\n\n\
                 Example:\n\
                 (upcase \"hello\") => \"HELLO\""
                    .into(),
            ),
        ),
    );
    env.set_function(
        "downcase".into(),
        LispExp::primitive(
            primitive_downcase,
            Some(
                "(downcase STRING): Return a copy of STRING with all letters \
                 lowercased.\n\n\
                 Example:\n\
                 (downcase \"HELLO\") => \"hello\""
                    .into(),
            ),
        ),
    );
    env.set_function(
        "format".into(),
        LispExp::primitive(
            primitive_format,
            Some(
                "(format STRING &rest OBJECTS): Format OBJECTS according to \
                 the directives in STRING and return the result. Supports \
                 %s/%S (display), %d (integer), %f (float), and %% (literal \
                 percent). Signals an error if there are fewer OBJECTS than \
                 directives require.\n\n\
                 Example:\n\
                 (format \"%s is %d\" \"age\" 30) => \"age is 30\"\n\
                 (format \"100%%\")               => \"100%\""
                    .into(),
            ),
        ),
    );
    env.set_function(
        "number-to-string".into(),
        LispExp::primitive(
            primitive_number_to_string,
            Some(
                "(number-to-string NUMBER): Return the decimal string \
                 representation of NUMBER, omitting the decimal point for \
                 integral values.\n\n\
                 Example:\n\
                 (number-to-string 5)   => \"5\"\n\
                 (number-to-string 5.5) => \"5.5\""
                    .into(),
            ),
        ),
    );
    env.set_function(
        "string-to-number".into(),
        LispExp::primitive(
            primitive_string_to_number,
            Some(
                "(string-to-number STRING): Parse STRING as a number and \
                 return it, ignoring leading/trailing whitespace. Returns 0 if \
                 STRING cannot be parsed.\n\n\
                 Example:\n\
                 (string-to-number \"42\")     => 42\n\
                 (string-to-number \"nope\")   => 0"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "symbol-name".into(),
        LispExp::primitive(
            primitive_symbol_name,
            Some(
                "(symbol-name SYMBOL): Return the name of SYMBOL as a \
                 string.\n\n\
                 Example:\n\
                 (symbol-name 'foo) => \"foo\""
                    .into(),
            ),
        ),
    );
    env.set_function(
        "intern".into(),
        LispExp::primitive(
            primitive_intern,
            Some(
                "(intern STRING): Return the symbol named STRING.\n\n\
                 Example:\n\
                 (intern \"foo\") => foo"
                    .into(),
            ),
        ),
    );
    env.set_function(
        "split-string".into(),
        LispExp::primitive(
            primitive_split_string,
            Some(
                "(split-string STRING &optional SEPARATORS): Split STRING into \
                 a list of substrings. With no SEPARATORS, splits on \
                 whitespace and discards empty pieces. With an empty \
                 SEPARATORS string, splits into individual characters. \
                 Otherwise splits on literal occurrences of SEPARATORS.\n\n\
                 Example:\n\
                 (split-string \"  a  b c \") => (\"a\" \"b\" \"c\")\n\
                 (split-string \"a,b,c\" \",\") => (\"a\" \"b\" \"c\")\n\
                 (split-string \"ab\" \"\")     => (\"a\" \"b\")"
                    .into(),
            ),
        ),
    );
}
