use super::*;

primitive!(quit, _args, _env, ctx, {
    ctx.quit();
    Ok(ELispExp::nil())
});

primitive!(eval_file, args, env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else if let Some(ELispExp::String(file)) = args.first() {
        Ok(ctx.eval_file(file, env.clone())?)
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: format!("{:?}", args.first()),
        })
    }
});

primitive!(define_key, args, env, ctx, {
    if args.len() != 3 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        })
    } else {
        if let (mode, ELispExp::String(key_str), ast) = (&args[0], &args[1], &args[2]) {
            let mode_name: Option<String> = match mode {
                ELispExp::Symbol(mode_name) | ELispExp::String(mode_name) => {
                    if mode_name.as_str() == "nil" {
                        None
                    } else {
                        Some(mode_name.to_string())
                    }
                }
                ELispExp::List(list) => {
                    if *list == std::sync::Arc::new(vec![]) {
                        None
                    } else {
                        return Err(EvalError::WrongArgumentType {
                            expected: "Symbol, String, Symbol".into(),
                            got: format!("{:?}, {:?}, {:?}", &args[0], &args[1], &args[2]),
                        });
                    }
                }
                _ => {
                    return Err(EvalError::WrongArgumentType {
                        expected: "Symbol, String, Symbol".into(),
                        got: format!("{:?}, {:?}, {:?}", &args[0], &args[1], &args[2]),
                    });
                }
            };
            let actual_ast = if let ELispExp::Symbol(symbol_name) = ast {
                if let Some(_) = env.get_function(symbol_name) {
                    ELispExp::list(vec![ast.clone()])
                } else {
                    ast.clone()
                }
            } else {
                ast.clone()
            };
            if let Some(key_event) = parse_key_sequence(key_str) {
                if let Some(mode_name) = mode_name {
                    let mut mode_registry_lock = ctx
                        .mode_registry
                        .write()
                        .expect("Failed to acquire write lock on mode_registry");
                    if let Some(mode) = mode_registry_lock.get_mut(&mode_name) {
                        mode.keymaps.insert(key_event, actual_ast);
                        Ok(ELispExp::t())
                    } else {
                        ctx.log_diagnostic(&format!(
                            "[ERROR]: define-key no mode named {mode_name}"
                        ));
                        Ok(ELispExp::nil())
                    }
                } else {
                    let mut keymaps = ctx
                        .keymaps
                        .write()
                        .expect("Failed to acquire write lock on keymaps");
                    keymaps.insert(key_event, actual_ast);
                    Ok(ELispExp::t())
                }
            } else {
                ctx.log_diagnostic(&format!("Invalid key sequence: {}", key_str));
                Ok(ELispExp::nil())
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String, Symbol".into(),
                got: format!("{:?}, {:?}", &args[0], &args[1]),
            })
        }
    }
});

primitive!(log, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        if let ELispExp::String(message) = &args[0] {
            ctx.log_diagnostic(&message);
            Ok(ELispExp::nil())
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: format!("{:?}", args.get(0)),
            })
        }
    }
});
