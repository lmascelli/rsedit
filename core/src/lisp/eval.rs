// ========================================================================== //
//                           +-----------------------------+
//                           |  Lisp evaluation functions  |
//                           +-----------------------------+
// ========================================================================== //

use super::{
    Env, EvalError, FiberState, Lambda, LispContext, LispExp, bind_lambda_args, condition_matches,
    data_to_form, error_data, error_symbol, parse_lambda_params,
};
use std::{collections::HashMap, sync::Arc};

enum EvalStep<T: LispContext> {
    Done(LispExp<T>),
    TailCall(LispExp<T>, Arc<Env<T>>),
}

pub fn eval<T: LispContext>(
    exp: &LispExp<T>,
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    let mut current_exp = exp.clone();
    let mut current_env = env;

    loop {
        match eval_step(&current_exp, current_env.clone(), ctx)? {
            EvalStep::Done(result) => return Ok(result),
            EvalStep::TailCall(next_exp, next_env) => {
                current_exp = next_exp;
                current_env = next_env;
            }
        }
    }
}

fn eval_step<T: LispContext>(
    exp: &LispExp<T>,
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<EvalStep<T>, EvalError<T>> {
    ctx.consume_fuel(1)?;
    match exp {
        LispExp::String(_)
        | LispExp::Number(_)
        | LispExp::Atom(_)
        | LispExp::Fiber(_)
        | LispExp::Lambda(_) => Ok(EvalStep::Done(exp.clone())),

        LispExp::Primitive { pointer: _, doc: _ } => Ok(EvalStep::Done(exp.clone())),

        LispExp::Symbol(symbol) => {
            // `nil`, `t` and keyword symbols (`:foo`) are
            // self-evaluating : they always evaluate to themselves
            // regardless of what is (or isn't) bound in the
            // environment. The same is valid for constant symbol
            // (i.e. those starting with :)
            if symbol.as_str() == "nil" || symbol.as_str() == "t" || symbol.starts_with(':') {
                return Ok(EvalStep::Done(exp.clone()));
            }
            if let Some(var) = env.get_variable(symbol) {
                Ok(EvalStep::Done(var))
            } else {
                Err(EvalError::UnboundVariable(symbol.to_string()))
            }
        }

        LispExp::Cons(_) => Ok(EvalStep::TailCall(data_to_form(exp)?, env)),

        LispExp::Form(list) => {
            if list.is_empty() {
                // `()` evaluates to the empty list, which is `nil` -- there
                // is no longer a second representation of it to return.
                Ok(EvalStep::Done(LispExp::nil()))
            } else {
                let head = &list[0];
                match head {
                    LispExp::Symbol(symbol) => {
                        eval_special_form_or_call_step(symbol, &list[1..], env.clone(), ctx)
                    }

                    LispExp::Form(_) => {
                        let mut new_ast = vec![eval(head, env.clone(), ctx)?];
                        for arg in &list[1..] {
                            new_ast.push(arg.clone());
                        }
                        return Ok(EvalStep::TailCall(LispExp::form(new_ast), env.clone()));
                    }

                    LispExp::Lambda(lambda) => {
                        // Directly eval the lambda with the arguments
                        let mut evaled_args = Vec::new();
                        for arg in &list[1..] {
                            evaled_args.push(eval(arg, env.clone(), ctx)?);
                        }

                        let call_frame = Env::new_child(&lambda.env);
                        ctx.push_call_frame("<lambda>");
                        bind_lambda_args(lambda, &evaled_args, &call_frame)?;

                        if lambda.body.is_empty() {
                            ctx.pop_call_frame();
                            return Ok(EvalStep::Done(LispExp::symbol("nil".into())));
                        }

                        for arg in &lambda.body[0..lambda.body.len() - 1] {
                            eval(arg, call_frame.clone(), ctx)?;
                        }

                        // About to tail-call into the last body form: this
                        // frame is done, the trampoline takes over from here.
                        ctx.pop_call_frame();
                        return Ok(EvalStep::TailCall(
                            lambda
                                .body
                                .last()
                                .expect("Failed to get the last expression in the function call")
                                .clone(),
                            call_frame,
                        ));
                    }
                    _ => {
                        return Err(EvalError::UnvalidFunctionCall);
                    }
                }
            }
        }

        LispExp::Vector(vec) => {
            let mut new_vec = Vec::with_capacity(vec.len());
            for v in vec.iter() {
                new_vec.push(eval(v, env.clone(), ctx)?);
            }
            Ok(EvalStep::Done(LispExp::vec(new_vec)))
        }

        LispExp::Map(map) => {
            let mut new_map = HashMap::new();
            for (k, v) in map.iter() {
                new_map.insert(k.clone(), eval(v, env.clone(), ctx)?);
            }
            Ok(EvalStep::Done(LispExp::map(new_map)))
        }
    }
}

fn eval_special_form_or_call_step<T: LispContext>(
    symbol: &str,
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<EvalStep<T>, EvalError<T>> {
    match symbol {
        "quote" => {
            if args.len() != 1 {
                Err(EvalError::QuoteNotOneArgument)
            } else {
                Ok(EvalStep::Done(args[0].clone()))
            }
        }

        "if" => {
            if args.len() < 1 {
                Err(EvalError::IfNoConditionProvided)
            } else if args.len() < 2 {
                Err(EvalError::IfNoTrueBrach)
            } else {
                let condition = eval(&args[0], env.clone(), ctx)?;
                if condition.is_truthy() {
                    Ok(EvalStep::TailCall(args[1].clone(), env.clone()))
                } else {
                    if args.len() > 2 {
                        Ok(EvalStep::TailCall(args[2].clone(), env.clone()))
                    } else {
                        Ok(EvalStep::Done(LispExp::symbol("nil".into())))
                    }
                }
            }
        }

        "while" => {
            if args.is_empty() {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: 1,
                    got: 0,
                });
            }

            let condition = &args[0];
            let body = &args[1..];

            let mut last_result = LispExp::symbol("nil".into());

            loop {
                let cond_val = eval(condition, env.clone(), ctx)?;
                if cond_val.is_nil() {
                    break;
                }
                for exp in body {
                    last_result = eval(exp, env.clone(), ctx)?;
                }
            }

            Ok(EvalStep::Done(last_result))
        }

        "spawn" => {
            if args.is_empty() {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: 1,
                    got: 0,
                });
            }

            let target_closure = eval(&args[0], env.clone(), ctx)?;

            if let LispExp::Lambda(lambda_data) = target_closure {
                let lambda_clone = lambda_data.clone();
                let mut thread_ctx = ctx.clone();

                std::thread::spawn(move || {
                    // Metering state is thread-local, so this thread starts on
                    // the compile-time default rather than the host's configured
                    // budget until it is told otherwise.
                    thread_ctx.begin_thread_evaluation();
                    let thread_frame = Env::new_child(&lambda_clone.env);
                    for exp in &lambda_clone.body {
                        if let Err(err) = eval(exp, thread_frame.clone(), &mut thread_ctx) {
                            thread_ctx.log_diagnostic(&format!("[LISP thread] {err:?}"));
                            break;
                        }
                    }
                });

                Ok(EvalStep::Done(LispExp::form(vec![])))
            } else {
                Err(EvalError::WrongArgumentType {
                    expected: "Lambda".into(),
                    got: target_closure.clone(),
                })
            }
        }

        "fiber" => {
            if args.is_empty() {
                return Ok(EvalStep::Done(LispExp::fiber(FiberState {
                    body: vec![],
                    env: env.clone(),
                    is_done: true,
                })));
            } else {
                Ok(EvalStep::Done(LispExp::fiber(FiberState {
                    body: args.to_vec(),
                    env: Env::new_child(&env),
                    is_done: false,
                })))
            }
        }

        "setq" => {
            if args.len() < 2 || args.len() % 2 != 0 {
                return Err(EvalError::SetqWrongNumberOfArgs(args.len()));
            }
            let mut is_symbol = true;
            let mut list_var_name = String::from("unreachable");
            let mut value = LispExp::symbol("nil".into());
            for arg in args {
                if is_symbol {
                    if let LispExp::Symbol(var_name) = arg {
                        list_var_name = var_name.to_string();
                        is_symbol = false;
                    } else {
                        return Err(EvalError::SetqSymbolRequired);
                    }
                } else {
                    value = eval(arg, env.clone(), ctx)?;
                    if !env.update_variable(&list_var_name, value.clone()) {
                        env.set_variable(list_var_name.clone(), value.clone());
                    }
                    is_symbol = true;
                }
            }
            Ok(EvalStep::Done(value))
        }

        // (defun NAME (REQUIRED... [&optional OPTIONAL...] [&rest REST])
        //   [DOCSTRING] BODY...)
        "defun" => {
            if args.len() < 3 {
                return Err(EvalError::DefunNotCorrectExpression);
            }
            if let LispExp::Symbol(func_name) = &args[0] {
                let mut body_index = 2;
                let mut doc = None;
                let (params, optionals, rest) = if let LispExp::Form(params_list) = &args[1] {
                    parse_lambda_params(params_list)?
                } else {
                    return Err(EvalError::DefunParamsAreNotAList);
                };

                if let LispExp::String(doc_string) = &args[2]
                    && args.len() > 3
                {
                    doc = Some(Arc::new(doc_string.to_string()));
                    body_index = 3;
                }

                let lambda = Lambda {
                    params,
                    optionals,
                    rest,
                    body: args[body_index..].to_vec(),
                    env: env.clone(),
                    doc,
                };
                env.set_function(func_name.to_string(), LispExp::lambda(lambda));

                Ok(EvalStep::Done(LispExp::symbol(func_name.to_string())))
            } else {
                Err(EvalError::DefunNameMustBeASymbol)
            }
        }

        // (lambda (REQUIRED... [&optional OPTIONAL...] [&rest REST]) BODY...)
        "lambda" => {
            if args.is_empty() {
                return Err(EvalError::DefunNotCorrectExpression);
            }

            let (params, optionals, rest) = if let LispExp::Form(params_list) = &args[0] {
                parse_lambda_params(params_list)?
            } else {
                return Err(EvalError::DefunParamsAreNotAList);
            };

            let body = args[1..].to_vec();

            Ok(EvalStep::Done(LispExp::lambda(Lambda {
                params,
                optionals,
                rest,
                body,
                env: env.clone(),
                doc: None,
            })))
        }

        "prog1" => {
            if args.is_empty() {
                Err(EvalError::WrongNumberOfArguments {
                    expected: 1,
                    got: 0,
                })
            } else {
                let first = eval(&args[0], env.clone(), ctx)?;
                for e in &args[1..] {
                    eval(e, env.clone(), ctx)?;
                }
                Ok(EvalStep::Done(first))
            }
        }

        "prog2" => {
            if args.len() < 2 {
                Err(EvalError::WrongNumberOfArguments {
                    expected: 2,
                    got: args.len(),
                })
            } else {
                eval(&args[0], env.clone(), ctx)?;
                let second = eval(&args[1], env.clone(), ctx)?;
                for e in &args[2..] {
                    eval(e, env.clone(), ctx)?;
                }
                Ok(EvalStep::Done(second))
            }
        }

        "progn" => {
            if args.is_empty() {
                return Ok(EvalStep::Done(LispExp::symbol("nil".into())));
            }
            for arg in &args[0..args.len() - 1] {
                eval(arg, env.clone(), ctx)?;
            }
            Ok(EvalStep::TailCall(
                args.last()
                    .expect("Failed to get the last progn expression")
                    .clone(),
                env.clone(),
            ))
        }

        "let" => {
            if args.is_empty() {
                return Err(EvalError::LetNoBindingsProvided);
            }

            let let_env = Env::new_child(&env);
            if let LispExp::Form(bindings) = &args[0] {
                for (i, binding) in bindings.iter().enumerate() {
                    let (name, value_form) = parse_let_binding(binding, i)?;
                    let val = match value_form {
                        Some(value_form) => eval(&value_form, env.clone(), ctx)?,
                        None => LispExp::nil(),
                    };
                    let_env.set_variable(name, val);
                }
            } else if !args[0].is_nil() {
                return Err(EvalError::LetUnvalidBindingList);
            }

            let body = &args[1..];
            if body.is_empty() {
                return Ok(EvalStep::Done(LispExp::nil()));
            }

            for arg in &body[0..body.len() - 1] {
                eval(arg, let_env.clone(), ctx)?;
            }

            Ok(EvalStep::TailCall(
                body.last()
                    .expect("Failed to get the last let expression")
                    .clone(),
                let_env,
            ))
        }

        // Like `let`, but each binding is evaluated (and immediately visible
        // to subsequent bindings) in sequence rather than in parallel.
        "let*" => {
            if args.is_empty() {
                Err(EvalError::LetNoBindingsProvided)
            } else {
                let let_env = Env::new_child(&env);
                if let LispExp::Form(bindings) = &args[0] {
                    for (i, binding) in bindings.iter().enumerate() {
                        let (name, value_form) = parse_let_binding(binding, i)?;
                        let val = match value_form {
                            Some(value_form) => eval(&value_form, let_env.clone(), ctx)?,
                            None => LispExp::nil(),
                        };
                        let_env.set_variable(name, val);
                    }
                } else if !args[0].is_nil() {
                    return Err(EvalError::LetUnvalidBindingList);
                }

                let body = &args[1..];
                if body.is_empty() {
                    return Ok(EvalStep::Done(LispExp::nil()));
                }

                for arg in &body[0..body.len() - 1] {
                    eval(arg, let_env.clone(), ctx)?;
                }

                Ok(EvalStep::TailCall(
                    body.last()
                        .expect("Failed to get the last let* expression")
                        .clone(),
                    let_env,
                ))
            }
        }

        "cond" => {
            for clause in args {
                let clause_list = if let LispExp::Form(clause_list) = clause {
                    clause_list
                } else {
                    return Err(EvalError::CondInvalidClause);
                };
                if clause_list.is_empty() {
                    return Err(EvalError::CondInvalidClause);
                }

                let test_val = eval(&clause_list[0], env.clone(), ctx)?;
                if test_val.is_truthy() {
                    let body = &clause_list[1..];
                    if body.is_empty() {
                        return Ok(EvalStep::Done(test_val));
                    }

                    for e in &body[0..body.len() - 1] {
                        eval(e, env.clone(), ctx)?;
                    }

                    return Ok(EvalStep::TailCall(
                        body.last()
                            .expect("Failed to get the last cond clause expression")
                            .clone(),
                        env.clone(),
                    ));
                }
            }
            Ok(EvalStep::Done(LispExp::nil()))
        }

        "and" => {
            if args.is_empty() {
                return Ok(EvalStep::Done(LispExp::t()));
            }
            for arg in &args[0..args.len() - 1] {
                if eval(arg, env.clone(), ctx)?.is_nil() {
                    return Ok(EvalStep::Done(LispExp::nil()));
                }
            }
            Ok(EvalStep::TailCall(
                args.last()
                    .expect("Failed to get the last and expression")
                    .clone(),
                env.clone(),
            ))
        }

        "or" => {
            if args.is_empty() {
                return Ok(EvalStep::Done(LispExp::nil()));
            }
            for arg in &args[0..args.len() - 1] {
                let val = eval(arg, env.clone(), ctx)?;
                if val.is_truthy() {
                    return Ok(EvalStep::Done(val));
                }
            }
            Ok(EvalStep::TailCall(
                args.last()
                    .expect("Failed to get the last or expression")
                    .clone(),
                env.clone(),
            ))
        }

        "when" => {
            if args.is_empty() {
                Err(EvalError::WrongNumberOfArguments {
                    expected: 1,
                    got: 0,
                })
            } else {
                if eval(&args[0], env.clone(), ctx)?.is_nil() {
                    Ok(EvalStep::Done(LispExp::nil()))
                } else {
                    let body = &args[1..];
                    if body.is_empty() {
                        Ok(EvalStep::Done(LispExp::nil()))
                    } else {
                        for e in &body[0..body.len() - 1] {
                            eval(e, env.clone(), ctx)?;
                        }
                        Ok(EvalStep::TailCall(
                            body.last()
                                .expect("Failed to get the last when expression")
                                .clone(),
                            env.clone(),
                        ))
                    }
                }
            }
        }

        "unless" => {
            if args.is_empty() {
                Err(EvalError::WrongNumberOfArguments {
                    expected: 1,
                    got: 0,
                })
            } else {
                if eval(&args[0], env.clone(), ctx)?.is_truthy() {
                    Ok(EvalStep::Done(LispExp::nil()))
                } else {
                    let body = &args[1..];
                    if body.is_empty() {
                        Ok(EvalStep::Done(LispExp::nil()))
                    } else {
                        for e in &body[0..body.len() - 1] {
                            eval(e, env.clone(), ctx)?;
                        }
                        Ok(EvalStep::TailCall(
                            body.last()
                                .expect("Failed to get the last unless expression")
                                .clone(),
                            env.clone(),
                        ))
                    }
                }
            }
        }

        // (dolist (VAR LIST-FORM [RESULT-FORM]) BODY...)
        "dolist" => {
            if args.is_empty() {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: 1,
                    got: 0,
                });
            }
            let spec = if let LispExp::Form(spec) = &args[0] {
                spec
            } else {
                return Err(EvalError::DolistInvalidBinding);
            };

            if spec.len() < 2 || spec.len() > 3 {
                return Err(EvalError::DolistInvalidBinding);
            }
            let var_name = if let LispExp::Symbol(name) = &spec[0] {
                name.to_string()
            } else {
                return Err(EvalError::DolistInvalidBinding);
            };

            let list_val = eval(&spec[1], env.clone(), ctx)?;
            let items: Vec<LispExp<T>> = match &list_val {
                LispExp::Cons(_) => list_val.iter().collect(),
                other => {
                    if other.is_nil() {
                        vec![]
                    } else {
                        return Err(EvalError::WrongArgumentType {
                            expected: "List".into(),
                            got: other.clone(),
                        });
                    }
                }
            };

            let loop_env = Env::new_child(&env);
            loop_env.set_variable(var_name.clone(), LispExp::nil());
            let body = &args[1..];
            for item in items {
                loop_env.update_variable(&var_name, item);
                for e in body {
                    eval(e, loop_env.clone(), ctx)?;
                }
            }

            if spec.len() == 3 {
                Ok(EvalStep::Done(eval(&spec[2], loop_env, ctx)?))
            } else {
                Ok(EvalStep::Done(LispExp::nil()))
            }
        }

        // (dotimes (VAR COUNT-FORM [RESULT-FORM]) BODY...)
        "dotimes" => {
            if args.is_empty() {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: 1,
                    got: 0,
                });
            }
            let spec = if let LispExp::Form(spec) = &args[0] {
                spec
            } else {
                return Err(EvalError::DotimesInvalidBinding);
            };
            if spec.len() < 2 || spec.len() > 3 {
                return Err(EvalError::DotimesInvalidBinding);
            }
            let var_name = if let LispExp::Symbol(name) = &spec[0] {
                name.to_string()
            } else {
                return Err(EvalError::DotimesInvalidBinding);
            };

            let count_val = eval(&spec[1], env.clone(), ctx)?;
            let count = if let LispExp::Number(n) = count_val {
                n as i64
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: count_val.clone(),
                });
            };

            let loop_env = Env::new_child(&env);
            loop_env.set_variable(var_name.clone(), LispExp::number(0.0));
            let body = &args[1..];
            let mut i = 0;
            while i < count {
                loop_env.update_variable(&var_name, LispExp::number(i as f64));
                for e in body {
                    eval(e, loop_env.clone(), ctx)?;
                }
                i += 1;
            }

            if spec.len() == 3 {
                Ok(EvalStep::Done(eval(&spec[2], loop_env, ctx)?))
            } else {
                Ok(EvalStep::Done(LispExp::nil()))
            }
        }

        // `defvar` only seeds the variable the first time it runs (a
        // later evaluation of the same `defvar` form is a no-op);
        // `defconst` always (re)initializes it.
        "defvar" | "defconst" => {
            if args.is_empty() {
                Err(EvalError::DefvarNameMustBeASymbol)
            } else {
                let name = if let LispExp::Symbol(name) = &args[0] {
                    name.to_string()
                } else {
                    return Err(EvalError::DefunNameMustBeASymbol);
                };
                if args.len() < 2 {
                    Ok(EvalStep::Done(LispExp::symbol(name)))
                } else {
                    if symbol == "defconst" || env.get_variable(&name).is_none() {
                        let val = eval(&args[1], env.clone(), ctx)?;
                        env.set_variable(name.clone(), val);
                    }
                    Ok(EvalStep::Done(LispExp::symbol(name)))
                }
            }
        }

        // (unwind-protect BODYFORM CLEANUP...) always runs CLEANUP, whether
        // BODYFORM returned normally or raised an error.
        "unwind-protect" => {
            if args.is_empty() {
                Err(EvalError::WrongNumberOfArguments {
                    expected: 1,
                    got: 0,
                })
            } else {
                let body_result = eval(&args[0], env.clone(), ctx);
                // Where the failure happened, so the cleanup forms can be
                // unwound back to it. A failed body leaves its frames standing
                // on purpose -- that is what `backtrace` reads -- but the
                // cleanup's own frames are not part of the failure.
                let depth_after_body = ctx.call_frame_depth();
                for cleanup in &args[1..] {
                    eval(cleanup, env.clone(), ctx)?;
                }
                ctx.truncate_call_frames(depth_after_body);
                Ok(EvalStep::Done(body_result?))
            }
        }

        // (catch TAG BODY...) evaluates TAG, then BODY in order, returning the
        // last body value -- unless a `(throw TAG VALUE)` with a matching tag
        // unwinds through it, in which case it returns VALUE instead.
        "catch" => {
            if args.is_empty() {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: 1,
                    got: 0,
                });
            }
            let tag = eval(&args[0], env.clone(), ctx)?;
            let depth_before = ctx.call_frame_depth();
            let mut result = LispExp::nil();
            for form in &args[1..] {
                // Evaluated here rather than handed back as a `TailCall`: the
                // trampoline would run the form *outside* this Rust frame and
                // the throw would sail straight past. The price is that a
                // `catch` is not tail-call transparent, as in real Elisp.
                match eval(form, env.clone(), ctx) {
                    Ok(value) => result = value,
                    Err(EvalError::Throw { tag: thrown, value }) => {
                        if thrown == tag {
                            ctx.truncate_call_frames(depth_before);
                            return Ok(EvalStep::Done(value));
                        }
                        // Another catch's tag: keep unwinding, payload intact.
                        return Err(EvalError::Throw { tag: thrown, value });
                    }
                    Err(err) => return Err(err),
                }
            }
            Ok(EvalStep::Done(result))
        }

        // (condition-case VAR BODY-FORM HANDLER...) where each HANDLER is
        // (CONDITION HANDLER-BODY...). VAR, unless nil, is bound in the handler
        // to (CONDITION-SYMBOL . DATA), as in Emacs.
        "condition-case" => {
            if args.len() < 2 {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: 2,
                    got: args.len(),
                });
            }
            let var = match &args[0] {
                other if other.is_nil() => None,
                LispExp::Symbol(name) => Some(name.to_string()),
                _ => return Err(EvalError::ConditionCaseInvalidVariable),
            };

            let depth_before = ctx.call_frame_depth();
            // Same reason as `catch`: the protected form must run inside this
            // Rust frame for the handlers to see its failure at all.
            let err = match eval(&args[1], env.clone(), ctx) {
                Ok(value) => return Ok(EvalStep::Done(value)),
                // A throw is a control transfer, not a failure. It belongs to
                // whichever `catch` named its tag and passes through untouched.
                Err(throw @ EvalError::Throw { .. }) => return Err(throw),
                Err(err) => err,
            };

            let symbol = error_symbol(&err);
            for handler in &args[2..] {
                let clause = match handler {
                    LispExp::Form(clause) if !clause.is_empty() => clause,
                    _ => return Err(EvalError::ConditionCaseInvalidHandler),
                };
                if !condition_matches(&clause[0], &symbol) {
                    continue;
                }

                // The protected form died partway and left its frames standing;
                // drop them now the failure is handled, or they surface in the
                // next unrelated backtrace.
                ctx.truncate_call_frames(depth_before);

                let handler_env = Env::new_child(&env);
                if let Some(name) = &var {
                    handler_env.set_variable(
                        name.clone(),
                        LispExp::cons(symbol.clone(), error_data(&err)),
                    );
                }
                if clause.len() == 1 {
                    return Ok(EvalStep::Done(LispExp::nil()));
                }
                for form in &clause[1..clause.len() - 1] {
                    eval(form, handler_env.clone(), ctx)?;
                }
                // Nothing is protected any more, so the handler's last form can
                // go back to the trampoline.
                return Ok(EvalStep::TailCall(
                    clause[clause.len() - 1].clone(),
                    handler_env,
                ));
            }
            Err(err)
        }

        // (defmacro NAME (REQUIRED... [&optional OPTIONAL...] [&rest REST])
        //   BODY...) defines a macro in its own namespace. Unlike `defun`,
        // the arguments are never evaluated: they are bound to the raw,
        // unevaluated call-site AST, and the macro body must produce a
        // new expression to be evaluated in place of the call. The same
        // `&optional`/`&rest` grammar as `defun`/`lambda` applies here.
        "defmacro" => {
            if args.len() < 2 {
                Err(EvalError::DefunNotCorrectExpression)
            } else {
                if let LispExp::Symbol(macro_name) = &args[0] {
                    if let LispExp::Form(params_list) = &args[1] {
                        let (params, optionals, rest) = parse_lambda_params(params_list)?;
                        let lambda = Lambda {
                            params,
                            optionals,
                            rest,
                            body: args[2..].to_vec(),
                            env: env.clone(),
                            doc: None,
                        };
                        env.set_macro(macro_name.to_string(), LispExp::lambda(lambda));
                        Ok(EvalStep::Done(LispExp::symbol(macro_name.to_string())))
                    } else {
                        Err(EvalError::DefunParamsAreNotAList)
                    }
                } else {
                    Err(EvalError::DefunNameMustBeASymbol)
                }
            }
        }

        // The `backquote`/`` ` `` reader macro: builds a template where
        // `,`/`unquote` splices in a single evaluated value and
        // `,@`/`unquote-splicing` splices in the elements of an evaluated
        // list. Only a single level of backquote nesting is supported.
        "backquote" => {
            if args.len() != 1 {
                Err(EvalError::BackquoteNotOneArgument)
            } else {
                Ok(EvalStep::Done(eval_backquote(&args[0], env.clone(), ctx)?))
            }
        }

        _ => eval_macro_or_function_call_step(symbol, args, env, ctx),
    }
}

/// Parses a single `let`/`let*` binding, which is either `(SYMBOL
/// VALUE-FORM)` or a bare `SYMBOL` (which binds to `nil`).
/// Returns the bound name and the value-form to evaluate, if any.
fn parse_let_binding<T: LispContext>(
    binding: &LispExp<T>,
    index: usize,
) -> Result<(String, Option<LispExp<T>>), EvalError<T>> {
    match binding {
        LispExp::Form(pair) if pair.len() == 2 => {
            if let LispExp::Symbol(name) = &pair[0] {
                Ok((name.to_string(), Some(pair[1].clone())))
            } else {
                Err(EvalError::LetUnvalidBindingAt(index))
            }
        }
        LispExp::Symbol(name) => Ok((name.to_string(), None)),
        _ => Err(EvalError::LetUnvalidBindingAt(index)),
    }
}

/// Expands a backquoted template, substituting `(unquote X)` forms with the
/// evaluation of `X` and splicing the evaluated list produced by
/// `(unquote-splicing X)` forms into the surrounding list.
fn eval_backquote<T: LispContext>(
    exp: &LispExp<T>,
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    fn is_tagged<T: LispContext>(list: &[LispExp<T>], tag: &str) -> bool {
        list.len() == 2 && matches!(&list[0], LispExp::Symbol(s) if s.as_str() == tag)
    }

    match exp {
        LispExp::Form(list) => {
            if is_tagged(list, "unquote") {
                return eval(&list[1], env, ctx);
            }

            let mut result = Vec::with_capacity(list.len());
            for item in list.iter() {
                if let LispExp::Form(inner) = item {
                    if is_tagged(inner, "unquote-splicing") {
                        let spliced = eval(&inner[1], env.clone(), ctx)?;
                        match &spliced {
                            LispExp::Cons(_) => result.extend(spliced.iter()),
                            LispExp::Form(spliced_list) => {
                                result.extend(spliced_list.iter().cloned());
                            }
                            other => {
                                if !other.is_nil() {
                                    return Err(EvalError::WrongArgumentType {
                                        expected: "List".into(),
                                        got: other.clone(),
                                    });
                                }
                            }
                        }
                        continue;
                    }
                }
                result.push(eval_backquote(item, env.clone(), ctx)?);
            }
            Ok(LispExp::proper_list(result))
        }
        LispExp::Vector(vec) => {
            let mut result = Vec::with_capacity(vec.len());
            for item in vec.iter() {
                result.push(eval_backquote(item, env.clone(), ctx)?);
            }
            Ok(LispExp::vec(result))
        }
        _ => Ok(exp.clone()),
    }
}

/// Dispatches a call whose head is a bare symbol: if the symbol names a
/// macro, expand it (with its arguments left unevaluated) and evaluate the
/// expansion in the *calling* environment; otherwise fall back to a normal
/// function call.
fn eval_macro_or_function_call_step<T: LispContext>(
    symbol: &str,
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<EvalStep<T>, EvalError<T>> {
    if let Some(LispExp::Lambda(macro_lambda)) = env.get_macro(symbol) {
        let expand_frame = Env::new_child(&macro_lambda.env);
        bind_lambda_args(&macro_lambda, args, &expand_frame)?;

        let mut expansion = LispExp::nil();
        for form in &macro_lambda.body {
            expansion = eval(form, expand_frame.clone(), ctx)?;
        }

        Ok(EvalStep::TailCall(expansion, env))
    } else {
        eval_function_call_step(symbol, args, env, ctx)
    }
}

fn eval_function_call_step<T: LispContext>(
    symbol: &str,
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<EvalStep<T>, EvalError<T>> {
    let mut evaled_args = Vec::new();
    for arg in args {
        evaled_args.push(eval(arg, env.clone(), ctx)?);
    }

    if let Some(func) = env.get_function(symbol) {
        ctx.push_call_frame(symbol);
        if let LispExp::Lambda(lambda) = func {
            let call_frame = Env::new_child(&lambda.env);
            bind_lambda_args(&lambda, &evaled_args, &call_frame)?;

            if lambda.body.is_empty() {
                ctx.pop_call_frame();
                return Ok(EvalStep::Done(LispExp::symbol("nil".into())));
            }

            for arg in &lambda.body[0..lambda.body.len() - 1] {
                eval(arg, call_frame.clone(), ctx)?;
            }

            // About to tail-call into the last body form: this frame is
            // done, the trampoline takes over from here.
            ctx.pop_call_frame();
            Ok(EvalStep::TailCall(
                lambda
                    .body
                    .last()
                    .expect("Failed to get the last expression in the function call")
                    .clone(),
                call_frame,
            ))
        } else if let LispExp::Primitive { pointer, doc: _ } = func {
            let result = pointer(&evaled_args[..], env.clone(), ctx)?;
            ctx.pop_call_frame();
            Ok(EvalStep::Done(result))
        } else {
            Err(EvalError::UncorrectFunctionDefinition)
        }
    } else {
        Err(EvalError::UndefinedFunction(symbol.into()))
    }
}
