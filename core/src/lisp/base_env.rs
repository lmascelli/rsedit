use crate::lisp::lisp::SharedAtom;
use crate::lisp::{Env, EvalError, LispContext, LispExp, eval};
use std::sync::{Arc, RwLock};

macro_rules! nil {
    () => {
        LispExp::list(vec![])
    };
}

// -------------------------------- CLASSIC LISP -------------------------------

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

    match func_obj {
        LispExp::Lambda(lambda) => {
            if lambda.params.len() != func_args.len() {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: lambda.params.len(),
                    got: func_args.len(),
                });
            }
            let call_frame = Env::new_child(&lambda.env);
            for (i, param_name) in lambda.params.iter().enumerate() {
                call_frame.set_variable(param_name.clone(), func_args[i].clone());
            }

            if lambda.body.is_empty() {
                return Ok(LispExp::symbol("nil".into()));
            } else {
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
        }
        LispExp::Primitive(func) => func(func_args, env, ctx),
        _ => Err(EvalError::UncorrectFunctionDefinition),
    }
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
                    LispExp::Primitive(_) => Ok(LispExp::string(
                        "Primitive function. Doc not provided at the moment".into(),
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

fn primitive_sum<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() < 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        })
    } else {
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
}

fn primitive_subtraction<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() < 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        })
    } else {
        let mut sum;
        if let LispExp::Number(number) = args[0] {
            sum = number;
        } else {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: format!("{:?}", args[0]),
            });
        }
        for arg in &args[1..] {
            if let LispExp::Number(number) = arg {
                sum -= number;
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: format!("{:?}", arg),
                });
            }
        }
        Ok(LispExp::number(sum))
    }
}

fn primitive_compare<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError> {
    if args.len() != 2 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        })
    } else {
        if args[0] == args[1] {
            Ok(LispExp::symbol("t".into()))
        } else {
            Ok(LispExp::symbol("nil".into()))
        }
    }
}

// -------------------------------- MULTI-THREADING ----------------------------

fn primitive_atom<T: LispContext>(
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
    // Functions
    env.set_function("funcall".into(), LispExp::Primitive(primitive_funcall));
    env.set_function(
        "function-doc".into(),
        LispExp::Primitive(primitive_function_doc),
    );
    env.set_function("atom".into(), LispExp::Primitive(primitive_atom));
    env.set_function("deref".into(), LispExp::Primitive(primitive_deref));
    env.set_function("reset".into(), LispExp::Primitive(primitive_reset));
    env.set_function("resume".into(), LispExp::Primitive(primitive_resume));
    env.set_function("+".into(), LispExp::Primitive(primitive_sum));
    env.set_function("-".into(), LispExp::Primitive(primitive_subtraction));
    env.set_function("=".into(), LispExp::Primitive(primitive_compare));

    // Symbols
    env.set_variable("nil".into(), LispExp::list(vec![]));
    env.set_variable("t".into(), LispExp::number(1.0));
}
