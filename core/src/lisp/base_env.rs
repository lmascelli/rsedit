use crate::lisp::lisp::SharedAtom;
use crate::lisp::{Env, EvalError, LispExp, eval};
use std::sync::{Arc, RwLock};

fn primitive_funcall<T>(args: &[LispExp<T>], ctx: &mut T) -> Result<LispExp<T>, EvalError>
where
    T: Clone + PartialEq + std::fmt::Debug + Send + Sync + 'static,
{
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
            eval(&lambda.body, call_frame, ctx)
        }
        LispExp::Primitive(func) => func(func_args, ctx),
        _ => Err(EvalError::UncorrectFunctionDefinition),
    }
}

fn primitive_atom<T>(args: &[LispExp<T>], _ctx: &mut T) -> Result<LispExp<T>, EvalError>
where
    T: Clone + PartialEq + std::fmt::Debug + Send + Sync + 'static,
{
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

fn primitive_deref<T>(args: &[LispExp<T>], _ctx: &mut T) -> Result<LispExp<T>, EvalError>
where
    T: Clone + PartialEq + std::fmt::Debug + Send + Sync + 'static,
{
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

fn primitive_reset<T>(args: &[LispExp<T>], _ctx: &mut T) -> Result<LispExp<T>, EvalError>
where
    T: Clone + PartialEq + std::fmt::Debug + Send + Sync + 'static,
{
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

pub fn setup_base_env<T>(env: std::sync::Arc<Env<T>>)
where
    T: Clone + std::fmt::Debug + PartialEq + Send + Sync + 'static,
{
    env.set_function("funcall".into(), LispExp::Primitive(primitive_funcall));
    env.set_function("atom".into(), LispExp::Primitive(primitive_atom));
    env.set_function("deref".into(), LispExp::Primitive(primitive_deref));
    env.set_function("reset".into(), LispExp::Primitive(primitive_reset));
}
