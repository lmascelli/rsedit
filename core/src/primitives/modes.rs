use super::*;

primitive!(make_mode, args, _env, ctx, {
    if args.len() != 1 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        })
    } else {
        if let Some(ELispExp::Symbol(mode_name)) = args.get(0) {
            ctx.mode_registry
                .write()
                .expect("Failed to acquire write lock on mode_registry")
                .insert(mode_name.to_string(), MajorMode::new(mode_name));
            Ok(ELispExp::symbol("t".into()))
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: format!("{:?}", args.get(0)),
            })
        }
    }
});

primitive!(add_hook, args, _env, ctx, {
    if args.len() != 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }

    if let (ELispExp::Symbol(mode_name), ELispExp::String(hook_name), ELispExp::Symbol(func_name)) =
        (&args[0], &args[1], &args[2])
    {
        let mut registry = ctx
            .mode_registry
            .write()
            .expect("Failed to acquire write lock on mode_registry");

        if let Some(mode) = registry.get_mut(mode_name.as_str()) {
            let hook_list = mode
                .hooks
                .entry(hook_name.to_string())
                .or_insert_with(Vec::new);
            hook_list.push(ELispExp::symbol(func_name.to_string()));

            Ok(ELispExp::symbol("t".into()))
        } else {
            ctx.log_diagnostic(&format!("Mode {} does not exist", mode_name));
            Ok(ELispExp::nil())
        }
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "Symbol, String, Symbol".into(),
            got: "other".into(),
        })
    }
});

primitive!(add_syntax_rule, args, _env, ctx, {
    if args.len() != 3 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        })
    } else {
        if let (
            ELispExp::Symbol(mode_name),
            ELispExp::String(regex_str),
            ELispExp::Symbol(face_sym),
        ) = (&args[0], &args[1], &args[2])
        {
            let face = match face_sym.as_str() {
                "keyword" => Face::Keyword,
                "type" => Face::Type,
                "string" => Face::String,
                "comment" => Face::Comment,
                "function" => Face::Function,
                "builtin" => Face::Builtin,
                face_sym_str => {
                    ctx.log_diagnostic(&format!(
                        "Unknown face: {}. Used Face::Default",
                        face_sym_str
                    ));
                    Face::Default
                }
            };

            match regex::Regex::new(regex_str) {
                Ok(pattern) => {
                    let mut registry = ctx
                        .mode_registry
                        .write()
                        .expect("Failed to acquire read lock on mode_registry");
                    if let Some(mode) = registry.get_mut(mode_name.as_str()) {
                        mode.syntax_rules.push(SyntaxRule { pattern, face });

                        Ok(ELispExp::t())
                    } else {
                        Ok(ELispExp::nil())
                    }
                }
                Err(e) => {
                    ctx.log_diagnostic(&format!("Invalid Regex {}: {}", regex_str, e));
                    Ok(ELispExp::nil())
                }
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Symbol, String, Symbol".into(),
                got: format!("{:?}", args),
            })
        }
    }
});
