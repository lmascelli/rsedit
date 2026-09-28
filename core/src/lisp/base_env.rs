use crate::lisp::SharedAtom;
use crate::lisp::{
    Env, EvalError, LispContext, LispExp, Parser, VARIABLE_DOCUMENTATION, bind_lambda_args, eval,
    resume_frames,
};
use std::sync::{Arc, RwLock};

macro_rules! nil {
    () => {
        LispExp::nil()
    };
}

// -------------------------------- HELPER FUNCTIONS ---------------------------
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
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
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
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
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

const FUNCALL_DOC: &str = "(funcall FUNCTION &rest ARGS): Call FUNCTION with ARGS. \
                 FUNCTION may be a lambda, a primitive, or a symbol naming a \
                 function in the function namespace -- same symbol \
                 resolution as `apply`/`mapcar`.\n\n\
                 Example:\n\
                 (funcall (lambda (x y) (+ x y)) 2 3) => 5\n\
                 (funcall 'car '(1 2))                => 1";

fn primitive_funcall<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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

const EVAL_DOC: &str = "(eval FORM): Evaluate FORM -- an already-parsed \
                 Lisp expression, such as a quoted list -- in the current \
                 environment and return the result. Any arguments after \
                 FORM are ignored; unlike real Emacs Lisp's `eval`, there \
                 is no optional LEXICAL argument.\n\n\
                 Example:\n\
                 (eval '(+ 1 2)) => 3\n\
                 (setq form '(* 2 3)) (eval form) => 6";

fn primitive_eval<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.is_empty() {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        })
    } else {
        eval(&args[0], env.clone(), ctx)
    }
}

const EVAL_STRING_DOC: &str = "(eval-string STRING): Parse STRING as Lisp \
                 source and evaluate the first top-level expression it \
                 contains in the current environment, returning the \
                 result. Only the first expression is evaluated -- wrap \
                 multiple forms in an explicit (progn ...) if STRING needs \
                 to contain more than one. Not a standard Elisp primitive; \
                 real Emacs Lisp achieves the same effect with \
                 (eval (read STRING)).\n\n\
                 Example:\n\
                 (eval-string \"(+ 1 2)\") => 3\n\
                 (eval-string \"(progn (setq x 10) (* x 2))\") => 20";

fn primitive_eval_string<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.is_empty() {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        })
    } else {
        if let LispExp::String(source) = &args[0] {
            let mut parser = Parser::new(source);
            match parser.next() {
                Ok(ast) => eval(&ast, env.clone(), ctx),
                Err(parse_error) => Err(EvalError::RuntimeMessage(format!(
                    "eval-string: Parser Error {:?}",
                    parse_error
                ))),
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: args[0].clone(),
            })
        }
    }
}

const EVAL_STRING_SAFE_DOC: &str = "(eval-string-safe STRING): Like `eval-string', but never \
                 propagates an error out of the call. Parses and evaluates \
                 STRING and returns a two-element list: (t RESULT) if \
                 parsing and evaluation both succeeded, or (nil DESCRIPTION) \
                 if either failed, where DESCRIPTION is a human-readable \
                 string describing what went wrong. Meant for interactive \
                 evaluation (a REPL, a minibuffer, ...) where a bad \
                 expression -- an unbound variable, a typo, wrong argument \
                 counts -- shouldn't abort whatever triggered the \
                 evaluation.\n\n\
                 Example:\n\
                 (eval-string-safe \"(+ 1 2)\") => (t 3)\n\
                 (eval-string-safe \"(1 2 3)\")  => (nil \"UnvalidFunctionCall\")";

fn primitive_eval_string_safe<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.is_empty() {
        Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        })
    } else if let LispExp::String(source) = &args[0] {
        // A failed nested `eval` below may leave frames pushed (per the
        // `push_call_frame` protocol, a failed call's frame stays put).
        // Since we're about to swallow that failure into a return value
        // instead of propagating it, restore the frame stack to how it
        // was before we started, or those frames would leak into whatever
        // *actually* uncaught error is reported next.
        let depth_before = ctx.call_frame_depth();
        let mut parser = Parser::new(source);
        let result = match parser.next() {
            Ok(ast) => eval(&ast, env.clone(), ctx),
            Err(parse_error) => Err(EvalError::RuntimeMessage(format!(
                "Parser Error {:?}",
                parse_error
            ))),
        };
        ctx.truncate_call_frames(depth_before);
        Ok(match result {
            Ok(value) => LispExp::proper_list(vec![LispExp::t(), value]),
            Err(err) => {
                LispExp::proper_list(vec![LispExp::nil(), LispExp::string(format!("{:?}", err))])
            }
        })
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[0].clone(),
        })
    }
}

const FUNCTION_DOC_DOC: &str = "(function-doc SYMBOL): Return the documentation string of the \
                 function bound to SYMBOL, or \"Undocumented function\" if it \
                 has none. Returns nil and logs a diagnostic if SYMBOL names no \
                 function. Not a standard Elisp primitive -- the closest real \
                 Elisp equivalent is `documentation`.\n\n\
                 Example:\n\
                 (function-doc 'car) => \"(car LIST): Return the first \
                 element of LIST, or nil if LIST is nil.\"\n\
                 (function-doc 'no-such-fn) => nil";

fn primitive_function_doc<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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
                    LispExp::Primitive { pointer: _, doc } => Ok(LispExp::string(doc.to_string())),
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
                got: args[0].clone(),
            });
            ctx.log_diagnostic(&format!("{err:?}"));
            err
        }
    }
}

const VARIABLE_DOC_DOC: &str = "(variable-doc SYMBOL): Return the documentation string \
                 `defvar\' or `defconst\' was given for SYMBOL, or \"Undocumented variable\" if it \
                 is bound and has none. Returns nil for a name that is neither bound nor \
                 documented.\n\n\
                 The documentation belongs to the symbol rather than to the binding, so a `let\' \
                 that shadows a variable does not shadow what it means, and a variable can be \
                 documented before it has any value at all.\n\n\
                 The same string is readable as an ordinary property: (get SYMBOL \
                 \'variable-documentation).\n\n\
                 Example:\n\
                 (defvar fill-column 70 \"Where lines are wrapped.\")\n\
                 (variable-doc \'fill-column) => \"Where lines are wrapped.\"";

fn primitive_variable_doc<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        let err = Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
        ctx.log_diagnostic(&format!("{err:?}"));
        return err;
    }
    let LispExp::Symbol(name) = &args[0] else {
        let err = Err(EvalError::WrongArgumentType {
            expected: "Symbol".into(),
            got: args[0].clone(),
        });
        ctx.log_diagnostic(&format!("{err:?}"));
        return err;
    };
    // The property first, and on its own: a documented variable that has not
    // been given a value yet -- `(defvar x nil "...")' aside, `(defvar x)'
    // binds nothing -- still has documentation, and answering "Undocumented"
    // because nothing is bound would be answering a different question.
    if let Some(doc) = env.get_property(name, VARIABLE_DOCUMENTATION) {
        return Ok(doc);
    }
    if env.get_variable(name).is_some() {
        return Ok(LispExp::string("Undocumented variable".to_string()));
    }
    ctx.log_diagnostic(&format!(
        "{} names neither a bound nor a documented variable",
        name.as_str()
    ));
    Ok(nil!())
}

// ------------------------- Asking the environment ----------------------------
//
// What a session knows is not a property of the language -- half of it is
// compiled in and the other half is whatever the loaded modules defined, and
// both change as the program grows. Anything that writes that list down is
// writing a snapshot that is wrong by the next commit, so everything here asks
// instead: a completion source offering a name, an `apropos' searching them,
// and a help command asking what one of them means.
//
// All of it is answerable from an `Env` alone, which is why it lives here
// rather than beside the editor's own primitives. The rule the file follows:
// a question an environment can answer is a primitive of this Lisp; a question
// that needs a buffer is a primitive of whatever is hosting it.

const ALL_FUNCTIONS_DOC: &str = "(all-functions): Return the names of every function visible \
     from the current environment, as a sorted list of strings -- primitives and Lisp \
     definitions alike, since nothing calling them can tell the difference.\n\n\
     A name bound in an inner scope appears once, not twice: this is the set of names that \
     would resolve, not the set of bindings that exist.\n\n\
     Macros are *not* included; see `all-macros'. Keeping them apart is what lets a caller \
     treat a macro as syntax and a function as a call.\n\n\
     Example:\n\
     (all-functions) => (\"1+\" \"abs\" \"append\" ...)";

fn primitive_all_functions<T: LispContext>(
    _args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    Ok(LispExp::proper_list(
        env.function_names()
            .into_iter()
            .map(LispExp::string)
            .collect(),
    ))
}

const ALL_MACROS_DOC: &str = "(all-macros): Return the names of every macro visible from the \
     current environment, as a sorted list of strings.\n\n\
     Special forms are not macros and are not listed: `if' and `let' are built into the \
     evaluator and have no binding to enumerate.\n\n\
     Example:\n\
     (all-macros) => (\"defcommand\")";

fn primitive_all_macros<T: LispContext>(
    _args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    Ok(LispExp::proper_list(
        env.macro_names().into_iter().map(LispExp::string).collect(),
    ))
}

const ALL_VARIABLES_DOC: &str = "(all-variables): Return the names of every variable visible \
     from the current environment, as a sorted list of strings.\n\n\
     Called inside a `let', its bindings are in the list too -- they are variables, and they \
     would resolve. What comes back describes the environment it was asked in.\n\n\
     Example:\n\
     (all-variables) => (\"case-fold-search\" \"frame-width\" ...)";

fn primitive_all_variables<T: LispContext>(
    _args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    Ok(LispExp::proper_list(
        env.variable_names()
            .into_iter()
            .map(LispExp::string)
            .collect(),
    ))
}

const BOUNDP_DOC: &str = "(boundp SYMBOL): Return t if SYMBOL has a value in the current \
     environment, nil otherwise.\n\n\
     The other half of `functionp', which asks the same thing of the function namespace. A \
     symbol can be bound in one and not the other, or in neither and still be documented -- \
     see `variable-doc'.\n\n\
     Worth asking before `symbol-value', which refuses a name that is bound to nothing rather \
     than inventing nil for it.\n\n\
     Example:\n\
     (boundp 'case-fold-search) => t\n\
     (boundp 'no-such-variable) => nil";

fn primitive_boundp<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let LispExp::Symbol(name) = &args[0] else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol".into(),
            got: args[0].clone(),
        });
    };
    Ok(LispExp::boolean(env.get_variable(name).is_some()))
}

const SYMBOL_VALUE_DOC: &str = "(symbol-value SYMBOL): Return what SYMBOL is bound to, without \
     evaluating anything else.\n\n\
     What a caller holding a *name* needs. Evaluating the symbol works when it was written down \
     in the source; it does not when the name arrived as data -- from `all-variables', from a \
     prompt, from a list being walked -- and this is the way to ask in that case.\n\n\
     Signals rather than answering nil when SYMBOL is bound to nothing, because nil is a value a \
     variable can legitimately have and the two must not read the same. Ask `boundp' first.\n\n\
     Example:\n\
     (symbol-value 'case-fold-search) => t\n\
     (mapcar 'symbol-value '(frame-width frame-height)) => (80 24)";

fn primitive_symbol_value<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let LispExp::Symbol(name) = &args[0] else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol".into(),
            got: args[0].clone(),
        });
    };
    env.get_variable(name)
        .ok_or_else(|| EvalError::UnboundVariable(name.to_string()))
}

const FUNCTION_ARGLIST_DOC: &str = "(function-arglist SYMBOL): Return how the function bound to \
     SYMBOL is called, as a list of strings -- its required parameters, then \"&optional\" and \
     the optional ones, then \"&rest\" and the rest parameter -- exactly as they were written \
     in its parameter list.\n\n\
     Returns nil for a primitive: one is a Rust function and has no parameter list to read. \
     What a primitive's call looks like is in the first line of its documentation, which is \
     where `function-doc' finds it.\n\n\
     Returns nil, and logs, for a name that is no function at all.\n\n\
     Example:\n\
     (defun greet (who &optional loudly) nil)\n\
     (function-arglist 'greet) => (\"who\" \"&optional\" \"loudly\")";

fn primitive_function_arglist<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        let err = Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
        ctx.log_diagnostic(&format!("{err:?}"));
        return err;
    }
    let LispExp::Symbol(name) = &args[0] else {
        let err = Err(EvalError::WrongArgumentType {
            expected: "Symbol".into(),
            got: args[0].clone(),
        });
        ctx.log_diagnostic(&format!("{err:?}"));
        return err;
    };
    let Some(function) = env.get_function(name) else {
        ctx.log_diagnostic(&format!("{} names no function", name.as_str()));
        return Ok(nil!());
    };
    // A primitive is a Rust function pointer. It has an arity, but no names
    // to report -- so nil, rather than a made-up list that would read as a
    // Lisp signature and be wrong about every name in it.
    let LispExp::Lambda(lambda) = function else {
        return Ok(nil!());
    };
    let mut parts: Vec<LispExp<T>> = lambda.params.iter().cloned().map(LispExp::string).collect();
    if !lambda.optionals.is_empty() {
        parts.push(LispExp::string("&optional".to_string()));
        parts.extend(lambda.optionals.iter().cloned().map(LispExp::string));
    }
    if let Some(rest) = &lambda.rest {
        parts.push(LispExp::string("&rest".to_string()));
        parts.push(LispExp::string(rest.clone()));
    }
    Ok(LispExp::proper_list(parts))
}

// --------------------------------- Symbols -----------------------------------

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
    if args.len() != 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }
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
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let (Some(symbol), Some(key)) = (property_name(&args[0]), property_name(&args[1])) else {
        return Err(EvalError::WrongArgumentType {
            expected: "Symbol".into(),
            got: args[0].clone(),
        });
    };
    Ok(env.get_property(&symbol, &key).unwrap_or_else(LispExp::nil))
}

// ----------------------------------- Lists -----------------------------------

const ADD_TO_LIST_DOC: &str = "(add-to-list LIST-VAR &rest ELEMENTS): Prepend each of \
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
                 my-list                  => (1 2 3)";

fn primitive_add_to_list<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() < 2 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        })
    } else {
        if let LispExp::Symbol(list_name) = &args[0] {
            // `nil` is now the empty list, so `(defvar my-list nil)`
            // followed by `add-to-list` works -- it did not before, when
            // only the `Form` variant was accepted here.
            if let Some(list) = env.get_variable(&list_name).filter(is_list_value) {
                let existing: Vec<LispExp<T>> = list.iter().collect();
                let mut new_list = existing.clone();
                for i in 1..args.len() {
                    if !existing.contains(&args[i]) {
                        new_list.insert(0, args[i].clone());
                    }
                }
                env.set_variable(list_name.to_string(), LispExp::proper_list(new_list));
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
                got: args[0].clone(),
            })
        }
    }
}

const APPEND_TO_LIST_DOC: &str = "(append-to-list LIST-VAR &rest ELEMENTS): Like `add-to-list`, \
                 but appends each of ELEMENTS not already present to the end \
                 of the list bound to LIST-VAR instead of the front. Not a \
                 standard Elisp primitive.\n\n\
                 Example:\n\
                 (defvar my-list '(1 2))\n\
                 (append-to-list 'my-list 3) => my-list\n\
                 my-list                     => (1 2 3)";

fn primitive_append_to_list<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() < 2 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        })
    } else {
        if let LispExp::Symbol(list_name) = &args[0] {
            if let Some(list) = env.get_variable(&list_name).filter(is_list_value) {
                let existing: Vec<LispExp<T>> = list.iter().collect();
                let mut new_list = existing.clone();
                for i in 1..args.len() {
                    if !existing.contains(&args[i]) {
                        new_list.push(args[i].clone());
                    }
                }
                env.set_variable(list_name.to_string(), LispExp::proper_list(new_list));
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
                got: args[0].clone(),
            })
        }
    }
}

const REMOVE_FROM_LIST_DOC: &str = "(remove-from-list LIST-VAR &rest ELEMENTS): Rebind LIST-VAR \
                 to a copy of its list with every element `equal` to one of \
                 ELEMENTS removed. Returns LIST-VAR. Not a standard Elisp \
                 primitive.\n\n\
                 Example:\n\
                 (defvar my-list '(1 2 3 4))\n\
                 (remove-from-list 'my-list 2 4) => my-list\n\
                 my-list                         => (1 3)";

fn primitive_remove_from_list<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() < 2 {
        Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        })
    } else {
        if let LispExp::Symbol(list_name) = &args[0] {
            if let Some(list) = env.get_variable(&list_name).filter(is_list_value) {
                let mut new_list = vec![];
                for el in list.iter() {
                    let mut el_found = false;
                    for arg in &args[1..] {
                        if &el == arg {
                            el_found = true;
                            break;
                        }
                    }
                    if !el_found {
                        new_list.push(el);
                    }
                }
                env.set_variable(list_name.to_string(), LispExp::proper_list(new_list));
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
                got: args[0].clone(),
            })
        }
    }
}

const CAR_DOC: &str = "(car LIST): Return the first element of LIST, or nil if LIST \
                 is nil.\n\n\
                 Example:\n\
                 (car '(1 2 3)) => 1\n\
                 (car nil)      => nil";

fn primitive_car<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    match &args[0] {
        LispExp::Cons(cell) => Ok(cell.car.clone()),
        other if other.is_nil() => Ok(LispExp::nil()),
        other => Err(EvalError::WrongArgumentType {
            expected: "List".into(),
            got: other.clone(),
        }),
    }
}

const CDR_DOC: &str = "(cdr LIST): Return LIST with its first element removed, or \
                 nil if LIST is nil or has one element. On a dotted pair, \
                 returns the tail.\n\n\
                 Example:\n\
                 (cdr '(1 2 3))   => (2 3)\n\
                 (cdr '(1))       => nil\n\
                 (cdr (cons 1 2)) => 2";

fn primitive_cdr<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    match &args[0] {
        LispExp::Cons(cell) => Ok(cell.cdr.clone()),
        other if other.is_nil() => Ok(LispExp::nil()),
        other => Err(EvalError::WrongArgumentType {
            expected: "List".into(),
            got: other.clone(),
        }),
    }
}

const CONS_DOC: &str = "(cons CAR CDR): Construct a new cons cell with CAR as its \
                 first element and CDR as its rest. If CDR is a list, the \
                 result is a proper list; if CDR is anything else (other than \
                 nil), the result is a dotted pair.\n\n\
                 Example:\n\
                 (cons 1 '(2 3)) => (1 2 3)\n\
                 (cons 1 2)      => (1 . 2)";

fn primitive_cons<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    // One allocation, no copying of the tail: this is the whole point of
    // the cons-cell representation. It used to rebuild the entire spine.
    Ok(LispExp::cons(args[0].clone(), args[1].clone()))
}

const LIST_DOC: &str = "(list &rest ARGS): Return a newly built list containing \
                 ARGS.\n\n\
                 Example:\n\
                 (list 1 2 3) => (1 2 3)\n\
                 (list)       => nil";

fn primitive_list<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    Ok(LispExp::proper_list(args.to_vec()))
}

const THROW_DOC: &str = "(throw TAG VALUE): Jump to the innermost enclosing \
                 `catch' whose tag equals TAG, making that `catch' return \
                 VALUE. Signals an error if no such `catch' is active.\n\n\
                 A `catch' with a different tag does not intercept it, an \
                 `unwind-protect' in between still runs its cleanups, and a \
                 `condition-case' deliberately does not catch it -- a throw is \
                 a control transfer, not a failure.\n\n\
                 Example:\n\
                 (catch 'done (dolist (x '(1 2 3)) (if (= x 2) (throw 'done x))))\n\
                 => 2";

fn primitive_throw<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    Err(EvalError::Throw {
        tag: args[0].clone(),
        value: args[1].clone(),
    })
}

const SIGNAL_DOC: &str = "(signal SYMBOL DATA): Raise the error condition SYMBOL, carrying \
                 DATA. A `condition-case' handler for SYMBOL (or for `error', \
                 which matches everything) receives (SYMBOL . DATA) in its \
                 variable.\n\n\
                 Example:\n\
                 (condition-case e (signal 'my-error '(1 2)) (my-error (cdr e)))\n\
                 => (1 2)";

fn primitive_signal<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    Err(EvalError::Signal {
        symbol: args[0].clone(),
        data: args[1].clone(),
    })
}

const NTH_DOC: &str = "(nth N LIST): Return the Nth element of LIST (zero-indexed), \
                 or nil if N is negative or past the end of LIST.\n\n\
                 Example:\n\
                 (nth 0 '(a b c)) => a\n\
                 (nth 2 '(a b c)) => c\n\
                 (nth 5 '(a b c)) => nil";

fn primitive_nth<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let n = expect_number(&args[0])?;
    if n < 0.0 {
        return Ok(LispExp::nil());
    }
    // Previously this cloned the entire backing vector via `expect_list`
    // and then indexed it. Walking the chain is the same O(n) without the
    // allocation and the n refcount bumps.
    // A `Form` is syntax, not a list of data, and `iter()` yields nothing for
    // one -- so without this check `(nth 0 some-form)` quietly returned nil
    // where every sibling (car, cdr, length, mapcar) raises. A silent nil
    // turns a mistake into wrong data instead of a diagnosable failure.
    match &args[1] {
        LispExp::Cons(_) => {}
        other if other.is_nil() => return Ok(LispExp::nil()),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "List".into(),
                got: other.clone(),
            });
        }
    }

    let n = n as usize;
    let mut walked = 0usize;
    let found = args[1].iter().inspect(|_| walked += 1).nth(n);
    charge(ctx, walked)?;
    Ok(found.unwrap_or_else(LispExp::nil))
}

const NTHCDR_DOC: &str = "(nthcdr N LIST): Return LIST with its first N elements \
                 removed. A negative N is treated as 0. Returns nil once N is \
                 at or past the end of LIST.\n\n\
                 Example:\n\
                 (nthcdr 2 '(1 2 3 4)) => (3 4)\n\
                 (nthcdr 99 '(1 2 3))  => nil";

fn primitive_nthcdr<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let n = expect_number(&args[0])?.max(0.0) as usize;
    // Walking to the nth cell and returning it shares the tail instead of
    // copying it, so `nthcdr` is now allocation-free.
    let mut cursor = args[1].clone();
    for _ in 0..n {
        // Charged a cell at a time rather than `n` up front: `n` may be far
        // past the end of a short list, and only cells actually walked are work.
        charge(ctx, 1)?;
        match &cursor {
            LispExp::Cons(cell) => cursor = cell.cdr.clone(),
            other if other.is_nil() => return Ok(LispExp::nil()),
            other => {
                return Err(EvalError::WrongArgumentType {
                    expected: "List".into(),
                    got: other.clone(),
                });
            }
        }
    }
    Ok(cursor)
}

const LENGTH_DOC: &str = "(length SEQUENCE): Return the number of elements in \
                 SEQUENCE, which may be a list, vector, string, or nil (0).\n\n\
                 Example:\n\
                 (length '(1 2 3)) => 3\n\
                 (length \"abc\")    => 3\n\
                 (length nil)      => 0";

fn primitive_length<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let len = match &args[0] {
        LispExp::Cons(_) => args[0].iter().count(),
        LispExp::Vector(v) => v.len(),
        LispExp::String(s) => s.chars().count(),
        other if other.is_nil() => 0,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Sequence".into(),
                got: other.clone(),
            });
        }
    };
    // `length` is O(n) in the sequence for every representation above, so it
    // is priced as such rather than as the single step the evaluator charged.
    charge(ctx, len)?;
    Ok(LispExp::number(len as f64))
}

const APPEND_DOC: &str = "(append &rest SEQUENCES): Concatenate all the given \
                 SEQUENCES into a list. If the final SEQUENCE is not a proper \
                 list, the result is a dotted list ending in that value. With \
                 no arguments, returns nil.\n\n\
                 Example:\n\
                 (append '(1 2) '(3 4)) => (1 2 3 4)\n\
                 (append '(1 2) 3)      => (1 2 . 3)\n\
                 (append)               => nil";

fn primitive_append<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.is_empty() {
        return Ok(LispExp::nil());
    }
    let mut result = Vec::new();
    for arg in &args[0..args.len() - 1] {
        result.extend(expect_list(arg, ctx)?);
    }
    Ok(LispExp::improper_list(result, args[args.len() - 1].clone()))
}

const REVERSE_DOC: &str = "(reverse SEQUENCE): Return a new sequence with the elements \
                 of SEQUENCE (a list, vector, or string) in reverse order.\n\n\
                 Example:\n\
                 (reverse '(1 2 3)) => (3 2 1)\n\
                 (reverse \"abc\")    => \"cba\"";

fn primitive_reverse<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    match &args[0] {
        LispExp::Cons(_) => {
            let mut v: Vec<LispExp<T>> = args[0].iter().collect();
            charge(ctx, v.len())?;
            v.reverse();
            Ok(LispExp::proper_list(v))
        }
        LispExp::Vector(v) => {
            charge(ctx, v.len())?;
            let mut v = (**v).clone();
            v.reverse();
            Ok(LispExp::vec(v))
        }
        LispExp::String(s) => {
            charge(ctx, s.chars().count())?;
            Ok(LispExp::string(s.chars().rev().collect()))
        }
        other if other.is_nil() => Ok(LispExp::nil()),
        other => Err(EvalError::WrongArgumentType {
            expected: "Sequence".into(),
            got: other.clone(),
        }),
    }
}

const MEMBER_DOC: &str = "(member ELEMENT LIST): Return the first sublist of LIST whose \
                 car is `equal` to ELEMENT, or nil if not found.\n\n\
                 Example:\n\
                 (member 2 '(1 2 3)) => (2 3)\n\
                 (member 9 '(1 2 3)) => nil";

fn primitive_member<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    find_member(args, ctx)
}

const MEMQ_DOC: &str = "(memq ELEMENT LIST): Return the first sublist of LIST whose \
                 car matches ELEMENT, or nil if not found. Since this \
                 implementation's `eq` is structural rather than \
                 identity-based, `memq` currently behaves the same as \
                 `member`.\n\n\
                 Example:\n\
                 (memq 'b '(a b c)) => (b c)";

fn primitive_memq<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    find_member(args, ctx)
}

const ASSOC_DOC: &str = "(assoc KEY ALIST): Return the first element of ALIST (an \
                 association list of cons cells or lists) whose car is `equal` \
                 to KEY, or nil if not found.\n\n\
                 Example:\n\
                 (assoc 'b '((a . 1) (b . 2))) => (b . 2)\n\
                 (assoc 'z '((a . 1) (b . 2))) => nil";

fn primitive_assoc<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    find_assoc(args, ctx)
}

const ASSQ_DOC: &str = "(assq KEY ALIST): Like `assoc`, but intended to compare KEY \
                 with `eq`. As with `memq`, this currently behaves the same as \
                 `assoc` since `eq` is structural here.\n\n\
                 Example:\n\
                 (assq 'b '((a . 1) (b . 2))) => (b . 2)";

fn primitive_assq<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    find_assoc(args, ctx)
}

const ELT_DOC: &str = "(elt SEQUENCE N): Return the Nth element of SEQUENCE (a list, \
                 vector, or string), or nil if N is negative or past the end \
                 of SEQUENCE.\n\n\
                 Example:\n\
                 (elt '(a b c) 1) => b\n\
                 (elt \"abc\" 1)    => \"b\"";

fn primitive_elt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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
        LispExp::Cons(_) => {
            let mut walked = 0usize;
            let found = args[0].iter().inspect(|_| walked += 1).nth(n);
            charge(ctx, walked)?;
            Ok(found.unwrap_or_else(LispExp::nil))
        }
        LispExp::Vector(v) => Ok(v.get(n).cloned().unwrap_or_else(LispExp::nil)),
        LispExp::String(s) => Ok(s
            .chars()
            .nth(n)
            .map(|c| LispExp::string(c.to_string()))
            .unwrap_or_else(LispExp::nil)),
        other if other.is_nil() => Ok(LispExp::nil()),
        other => Err(EvalError::WrongArgumentType {
            expected: "Sequence".into(),
            got: other.clone(),
        }),
    }
}

const MAPCAR_DOC: &str = "(mapcar FUNCTION LIST): Apply FUNCTION to each element of \
                 LIST in turn and return a list of the results. FUNCTION may \
                 be a lambda, a primitive, or a symbol naming a function.\n\n\
                 Example:\n\
                 (mapcar '1+ '(1 2 3))              => (2 3 4)\n\
                 (mapcar (lambda (x) (* x x)) '(1 2 3)) => (1 4 9)";

fn primitive_mapcar<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let list = expect_list(&args[1], ctx)?;
    let mut result = Vec::with_capacity(list.len());
    for item in list {
        result.push(call_callable(&args[0], &[item], env.clone(), ctx)?);
    }
    Ok(LispExp::proper_list(result))
}

const MAPC_DOC: &str = "(mapc FUNCTION LIST): Apply FUNCTION to each element of LIST \
                 for its side effects and return LIST unchanged.\n\n\
                 Example:\n\
                 (mapc (lambda (x) (log (number-to-string x))) '(1 2 3))\n\
                 ; logs \"1\", \"2\", \"3\" and returns (1 2 3)";

fn primitive_mapc<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let list = expect_list(&args[1], ctx)?;
    for item in &list {
        call_callable(&args[0], std::slice::from_ref(item), env.clone(), ctx)?;
    }
    Ok(args[1].clone())
}

const APPLY_DOC: &str = "(apply FUNCTION &rest ARGS LIST): Call FUNCTION with ARGS \
                 followed by the elements of the final LIST argument, all \
                 spliced together. FUNCTION may be a lambda, a primitive, or a \
                 symbol naming a function.\n\n\
                 Example:\n\
                 (apply '+ '(1 2 3))       => 6\n\
                 (apply '+ 1 2 '(3 4))     => 10";

fn primitive_apply<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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
    call_args.extend(expect_list(&args[args.len() - 1], ctx)?);
    call_callable(func, &call_args, env, ctx)
}

const IDENTITY_DOC: &str = "(identity ARG): Return ARG unchanged.\n\n\
                 Example:\n\
                 (identity 42) => 42";

fn primitive_identity<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(args[0].clone())
}

// ----------------------------- Predicates ------------------------------------

fn primitive_equal_impl<T: LispContext>(args: &[LispExp<T>]) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(args[0] == args[1]))
}

fn primitive_eq_impl<T: LispContext>(args: &[LispExp<T>]) -> Result<LispExp<T>, EvalError<T>> {
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
        (LispExp::Form(a), LispExp::Form(b)) => Arc::ptr_eq(a, b),
        // Two chains are `eq` when they are the same cell -- sharing a
        // tail is not enough, exactly as in real Elisp.
        (LispExp::Cons(a), LispExp::Cons(b)) => Arc::ptr_eq(a, b),
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
) -> Result<LispExp<T>, EvalError<T>> {
    primitive_eq_impl(args)
}

fn primitive_eql<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    primitive_eq_impl(args)
}

const EQUAL_DOC: &str = "(equal A B): Return t if A and B have the same structure and \
                 contents (deep, structural comparison), nil otherwise.\n\n\
                 Example:\n\
                 (equal '(1 2) '(1 2)) => t\n\
                 (equal \"ab\" \"ab\")    => t\n\
                 (equal '(1 2) '(1 3)) => nil";

fn primitive_equal<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    primitive_equal_impl(args)
}

const NULL_DOC: &str = "(null OBJECT): Return t if OBJECT is nil, nil otherwise.\n\n\
                 Example:\n\
                 (null nil)   => t\n\
                 (null '(1))  => nil";

fn primitive_null<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(args[0].is_nil()))
}

const NOT_DOC: &str = "(not OBJECT): Return t if OBJECT is nil, nil otherwise. \
                 Identical to `null`.\n\n\
                 Example:\n\
                 (not nil) => t\n\
                 (not t)   => nil";

fn primitive_not<T: LispContext>(
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    // `not` and `null` are the same operation in Elisp.
    primitive_null(args, env, ctx)
}

const CONSP_DOC: &str = "(consp OBJECT): Return t if OBJECT is a cons cell -- a \
                 non-empty list or a dotted pair -- nil otherwise.\n\n\
                 Example:\n\
                 (consp '(1 2)) => t\n\
                 (consp nil)    => nil\n\
                 (consp 5)      => nil";

fn primitive_consp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let is_cons = matches!(&args[0], LispExp::Cons(_));
    Ok(LispExp::boolean(is_cons))
}

fn primitive_listp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(is_list_value(&args[0])))
}

const STRINGP_DOC: &str = "(stringp OBJECT): Return t if OBJECT is a string, nil \
                 otherwise.\n\n\
                 Example:\n\
                 (stringp \"hi\") => t\n\
                 (stringp 5)    => nil";

fn primitive_stringp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(matches!(&args[0], LispExp::String(_))))
}

const NUMBERP_DOC: &str = "(numberp OBJECT): Return t if OBJECT is a number, nil \
                 otherwise.\n\n\
                 Example:\n\
                 (numberp 5)     => t\n\
                 (numberp \"5\")   => nil";

fn primitive_numberp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(matches!(&args[0], LispExp::Number(_))))
}

const SYMBOLP_DOC: &str = "(symbolp OBJECT): Return t if OBJECT is a symbol, nil \
                 otherwise.\n\n\
                 Example:\n\
                 (symbolp 'foo) => t\n\
                 (symbolp \"foo\") => nil";

fn primitive_symbolp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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
    env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let is_function = match &args[0] {
        LispExp::Lambda(_) | LispExp::Primitive { .. } => true,
        LispExp::Symbol(name) => env.get_function(name).is_some(),
        _ => false,
    };
    Ok(LispExp::boolean(is_function))
}

const VECTORP_DOC: &str = "(vectorp OBJECT): Return t if OBJECT is a vector, nil \
                 otherwise.\n\n\
                 Example:\n\
                 (vectorp [1 2 3]) => t\n\
                 (vectorp '(1 2))  => nil";

fn primitive_vectorp<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::boolean(matches!(&args[0], LispExp::Vector(_))))
}

const ZEROP_DOC: &str = "(zerop NUMBER): Return t if NUMBER is zero, nil otherwise.\n\n\
                 Example:\n\
                 (zerop 0) => t\n\
                 (zerop 1) => nil";

fn primitive_zerop<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let is_cons = matches!(&args[0], LispExp::Cons(_));
    Ok(LispExp::boolean(!is_cons))
}

// --------------------------------- Math --------------------------------------

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
) -> Result<LispExp<T>, EvalError<T>> {
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

const N_1PLUS_DOC: &str = "(1+ NUMBER): Return NUMBER plus one.\n\n\
                 Example:\n\
                 (1+ 4) => 5";

fn primitive_1plus<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
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
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::number(expect_number(&args[0])? - 1.0))
}

// ------------------------------- Comparisons ---------------------------------

const COMPARE_DOC: &str = "(= NUMBER &rest NUMBERS): Return t if all arguments are \
                 numerically equal, nil otherwise. Requires at least one \
                 argument.\n\n\
                 Example:\n\
                 (= 1 1 1) => t\n\
                 (= 1 1 2) => nil\n\
                 (= 3)     => t";

fn primitive_compare<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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
                got: arg.clone(),
            });
        }
    }
    Ok(LispExp::boolean(numbers.windows(2).all(|w| w[0] == w[1])))
}

const LT_DOC: &str = "(< NUMBER &rest NUMBERS): Return t if the arguments are in \
                 strictly increasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (< 1 2 3) => t\n\
                 (< 1 3 2) => nil";

fn primitive_lt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    compare_chain(args, |a, b| a < b)
}

const GT_DOC: &str = "(> NUMBER &rest NUMBERS): Return t if the arguments are in \
                 strictly decreasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (> 3 2 1) => t\n\
                 (> 3 1 2) => nil";

fn primitive_gt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    compare_chain(args, |a, b| a > b)
}

const LE_DOC: &str = "(<= NUMBER &rest NUMBERS): Return t if the arguments are in \
                 non-decreasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (<= 1 1 2) => t\n\
                 (<= 2 1)   => nil";

fn primitive_le<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    compare_chain(args, |a, b| a <= b)
}

const GE_DOC: &str = "(>= NUMBER &rest NUMBERS): Return t if the arguments are in \
                 non-increasing numeric order, nil otherwise.\n\n\
                 Example:\n\
                 (>= 2 1 1) => t\n\
                 (>= 1 2)   => nil";

fn primitive_ge<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    compare_chain(args, |a, b| a >= b)
}

const MAX_DOC: &str = "(max NUMBER &rest NUMBERS): Return the largest of the \
                 arguments.\n\n\
                 Example:\n\
                 (max 1 5 3) => 5";

fn primitive_max<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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

const MIN_DOC: &str = "(min NUMBER &rest NUMBERS): Return the smallest of the \
                 arguments.\n\n\
                 Example:\n\
                 (min 1 5 3) => 1";

fn primitive_min<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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

const ABS_DOC: &str = "(abs NUMBER): Return the absolute value of NUMBER.\n\n\
                 Example:\n\
                 (abs -5) => 5\n\
                 (abs 5)  => 5";

fn primitive_abs<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::number(expect_number(&args[0])?.abs()))
}

// ----------------------------------- Strings ---------------------------------

const FLOOR_DOC: &str = "(floor NUMBER): Return the largest integer not greater than NUMBER. \
                 Negative numbers round away from zero, so (floor -1.5) is -2.\n\n\
                 Needed because `/' here is division, not integer division: (/ 1 3) is \
                 0.3333333, and anything counting rows and columns wants the 0.\n\n\
                 Example:\n\
                 (floor (/ 7 3)) => 2";

fn primitive_floor<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::number(expect_number(&args[0])?.floor()))
}

const MAKE_STRING_DOC: &str = "(make-string COUNT STRING): Return COUNT copies of STRING's first \
                 character, joined. A COUNT of zero or less gives the empty string.\n\n\
                 Chiefly for padding: laying anything out in columns means writing the spaces \
                 between them, and building those a character at a time in Lisp costs a cons \
                 per space.\n\n\
                 Example:\n\
                 (concat \"a\" (make-string 3 \" \") \"b\") => \"a   b\"";

fn primitive_make_string<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let count = expect_number(&args[0])?;
    let fill = expect_string(&args[1])?;
    // The first character, as `self-insert` takes it -- this Lisp has no
    // character type, so a one-character string is how one is written.
    let Some(c) = fill.chars().next() else {
        return Ok(LispExp::string(String::new()));
    };
    let count = if count.is_finite() {
        count.max(0.0) as usize
    } else {
        0
    };
    Ok(LispExp::string(String::from(c).repeat(count)))
}

const CONCAT_DOC: &str = "(concat &rest STRINGS): Concatenate STRINGS into a single \
                 string.\n\n\
                 Example:\n\
                 (concat \"foo\" \"-\" \"bar\") => \"foo-bar\"";

fn primitive_concat<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    let mut result = String::new();
    for arg in args {
        result.push_str(&expect_string(arg)?);
    }
    Ok(LispExp::string(result))
}

const STRING_EQ_DOC: &str = "(string= STRING1 STRING2): Return t if STRING1 and STRING2 \
                 have the same contents, nil otherwise.\n\n\
                 Example:\n\
                 (string= \"foo\" \"foo\") => t\n\
                 (string= \"foo\" \"bar\") => nil";

fn primitive_string_eq<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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

const STRING_LT_DOC: &str = "(string< STRING1 STRING2): Return t if STRING1 sorts before \
                 STRING2 lexicographically, nil otherwise.\n\n\
                 Example:\n\
                 (string< \"abc\" \"abd\") => t\n\
                 (string< \"abd\" \"abc\") => nil";

fn primitive_string_lt<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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

const SUBSTRING_DOC: &str = "(substring STRING &optional START END): Return the substring \
                 of STRING from START (inclusive, default 0) to END \
                 (exclusive, default the length of STRING). Negative indices \
                 count from the end of STRING. Signals a runtime error if \
                 START is after END.\n\n\
                 Example:\n\
                 (substring \"hello\" 1 3)  => \"el\"\n\
                 (substring \"hello\" -3)   => \"llo\"\n\
                 (substring \"hello\" -3 -1) => \"ll\"";

fn primitive_substring<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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

const UPCASE_DOC: &str = "(upcase STRING): Return a copy of STRING with all letters \
                 uppercased.\n\n\
                 Example:\n\
                 (upcase \"hello\") => \"HELLO\"";

fn primitive_upcase<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::string(expect_string(&args[0])?.to_uppercase()))
}

const DOWNCASE_DOC: &str = "(downcase STRING): Return a copy of STRING with all letters \
                 lowercased.\n\n\
                 Example:\n\
                 (downcase \"HELLO\") => \"hello\"";

fn primitive_downcase<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::string(expect_string(&args[0])?.to_lowercase()))
}

const NUMBER_TO_STRING_DOC: &str = "(number-to-string NUMBER): Return the decimal string \
                 representation of NUMBER, omitting the decimal point for \
                 integral values.\n\n\
                 Example:\n\
                 (number-to-string 5)   => \"5\"\n\
                 (number-to-string 5.5) => \"5.5\"";

fn primitive_number_to_string<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::string(format_number(expect_number(&args[0])?)))
}

const STRING_TO_NUMBER_DOC: &str = "(string-to-number STRING): Parse STRING as a number and \
                 return it, ignoring leading/trailing whitespace. Returns 0 if \
                 STRING cannot be parsed.\n\n\
                 Example:\n\
                 (string-to-number \"42\")     => 42\n\
                 (string-to-number \"nope\")   => 0";

fn primitive_string_to_number<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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

const SYMBOL_NAME_DOC: &str = "(symbol-name SYMBOL): Return the name of SYMBOL as a \
                 string.\n\n\
                 Example:\n\
                 (symbol-name 'foo) => \"foo\"";

fn primitive_symbol_name<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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
            got: args[0].clone(),
        })
    }
}

const INTERN_DOC: &str = "(intern STRING): Return the symbol named STRING.\n\n\
                 Example:\n\
                 (intern \"foo\") => foo";

fn primitive_intern<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    Ok(LispExp::symbol(expect_string(&args[0])?))
}

const SPLIT_STRING_DOC: &str = "(split-string STRING &optional SEPARATORS): Split STRING into \
                 a list of substrings. With no SEPARATORS, splits on \
                 whitespace and discards empty pieces. With an empty \
                 SEPARATORS string, splits into individual characters. \
                 Otherwise splits on literal occurrences of SEPARATORS.\n\n\
                 Example:\n\
                 (split-string \"  a  b c \") => (\"a\" \"b\" \"c\")\n\
                 (split-string \"a,b,c\" \",\") => (\"a\" \"b\" \"c\")\n\
                 (split-string \"ab\" \"\")     => (\"a\" \"b\")";

fn primitive_split_string<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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
    Ok(LispExp::proper_list(parts))
}

const FORMAT_DOC: &str = "(format STRING &rest OBJECTS): Format OBJECTS according to \
                 the directives in STRING and return the result. Supports \
                 %s/%S (display), %d (integer), %f (float), and %% (literal \
                 percent). Signals an error if there are fewer OBJECTS than \
                 directives require.\n\n\
                 Example:\n\
                 (format \"%s is %d\" \"age\" 30) => \"age is 30\"\n\
                 (format \"100%%\")               => \"100%\"";

fn primitive_format<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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

    let next_arg = |idx: &mut usize| -> Result<&LispExp<T>, EvalError<T>> {
        let val = args.get(*idx).ok_or(EvalError::WrongNumberOfArguments {
            expected: *idx + 1,
            got: args.len(),
        })?;
        *idx += 1;
        Ok(val)
    };

    while let Some(c) = chars.next() {
        if c == '%' {
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
        } else if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some(other) => {
                    return Err(EvalError::RuntimeMessage(format!(
                        "Wrong escape character \\{other}"
                    )));
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
            continue;
        }
    }
    Ok(LispExp::string(result))
}

// -------------------------------- MULTI-THREADING ----------------------------

fn primitive_make_atom<T: LispContext>(
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
        Ok(LispExp::Atom(SharedAtom(Arc::new(RwLock::new(
            args[0].clone(),
        )))))
    }
}

const DEREF_DOC: &str = "(deref ATOM): Return the current value stored in ATOM.\n\n\
                 Example:\n\
                 (setq counter (atom 0))\n\
                 (deref counter) => 0";

fn primitive_deref<T: LispContext>(
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
        if let LispExp::Atom(atom_lock) = &args[0] {
            let guard = atom_lock
                .0
                .read()
                .map_err(|_| EvalError::UncorrectFunctionDefinition)?;
            Ok(guard.clone())
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Atom".into(),
                got: args[0].clone(),
            })
        }
    }
}

const RESET_DOC: &str = "(reset ATOM NEWVAL): Set ATOM's stored value to NEWVAL and \
                 return NEWVAL.\n\n\
                 Example:\n\
                 (setq counter (atom 0))\n\
                 (reset counter 42) => 42\n\
                 (deref counter)    => 42";

fn primitive_reset<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
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
                got: args[0].clone(),
            })
        }
    }
}

// -------------------------------- CONCURRENCY --------------------------------

const RESUME_DOC: &str = "(resume FIBER): Carry FIBER on from wherever it stopped, and return \
                 the value it stopped with. Returns nil once FIBER has nothing left to run.\n\n\
                 A fiber runs until it reaches a `yield\' or until its body is finished, \
                 whichever comes first. A `yield\' stops it *inside* a form, and the next \
                 `resume\' picks up at that exact spot with its variables as it left them.\n\n\
                 The body is a `progn\': `(fiber A B C)\' runs all three in one `resume\' unless \
                 one of them yields. Put a `yield\' between them to stop after each.\n\n\
                 An error inside a fiber ends it: the position it stopped at is gone, so there is \
                 nothing left to come back to, and `resume\' would otherwise silently restart it \
                 somewhere it had already been.\n\n\
                 Example:\n\
                 (setq f (fiber (log \"a\") (yield 1) (log \"b\") 2))\n\
                 (resume f) ; logs \"a\", => 1\n\
                 (resume f) ; logs \"b\", => 2\n\
                 (resume f) => nil ; fiber is done";

fn primitive_resume<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    let Some(LispExp::Fiber(shared_fiber)) = args.first() else {
        return match args.first() {
            None => Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: 0,
            }),
            Some(other) => Err(EvalError::WrongArgumentType {
                expected: "Fiber".into(),
                got: other.clone(),
            }),
        };
    };

    // # Why the stack is taken out rather than borrowed
    //
    // Running it means running arbitrary Lisp, and this used to do that with
    // the fiber's write lock still held. That was survivable while a fiber
    // could only run one whole form to completion; it is not now. The first
    // thing that Lisp does that reaches its own fiber -- `(resume f)' on
    // itself, or asking whether it has finished -- deadlocks on a lock nobody
    // can see. So the lock answers one question, is dropped, and is taken
    // again to record the answer.
    //
    // Taking it also leaves the fiber holding *nothing* while it runs, which
    // is the honest state: a second `resume' arriving from another thread
    // finds an empty stack and does nothing, rather than starting the same
    // program a second time alongside the first.
    let frames = {
        let mut fiber = shared_fiber
            .0
            .write()
            .map_err(|_| EvalError::UncorrectFunctionDefinition)?;
        if fiber.is_done {
            return Ok(LispExp::nil());
        }
        std::mem::take(&mut fiber.pending)
    };
    if frames.is_empty() {
        let mut fiber = shared_fiber
            .0
            .write()
            .map_err(|_| EvalError::UncorrectFunctionDefinition)?;
        fiber.is_done = true;
        return Ok(LispExp::nil());
    }

    let outcome = resume_frames(frames, ctx);

    let mut fiber = shared_fiber
        .0
        .write()
        .map_err(|_| EvalError::UncorrectFunctionDefinition)?;
    match outcome {
        // Suspended again: the new stack is where it stopped.
        Err(EvalError::Yielded { value, frames }) => {
            fiber.pending = frames;
            Ok(value)
        }
        // Nothing left on the stack, so nothing left to come back to.
        Ok(value) => {
            fiber.is_done = true;
            Ok(value)
        }
        Err(other) => {
            // See the docstring: unwinding destroyed the position, so there is
            // no coming back. Resuming again would restart the program
            // somewhere it had already been.
            fiber.is_done = true;
            fiber.pending.clear();
            Err(other)
        }
    }
}

const FIBER_DONE_P_DOC: &str = "(fiber-done-p FIBER): t when FIBER has nothing left to run, nil \
                 while it still has. Signals if FIBER is not a fiber.\n\n\
                 What a scheduler asks so it can stop resuming one. A fiber that is merely \
                 *suspended* is not done -- it is in the middle of a form, waiting to be resumed \
                 -- so this is the only honest way to tell a finished task from a parked one; \
                 `resume\' answering nil cannot, since a fiber may perfectly well yield nil.\n\n\
                 Example:\n\
                 (while (not (fiber-done-p task)) (resume task))";

fn primitive_fiber_done_p<T: LispContext>(
    args: &[LispExp<T>],
    _env: Arc<Env<T>>,
    _ctx: &T,
) -> Result<LispExp<T>, EvalError<T>> {
    let Some(exp) = args.first() else {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: 0,
        });
    };
    match exp.fiber_is_done() {
        Some(done) => Ok(LispExp::boolean(done)),
        None => Err(EvalError::WrongArgumentType {
            expected: "Fiber".into(),
            got: exp.clone(),
        }),
    }
}

// -------------------------------- CONSTRUCTOR --------------------------------
pub fn setup_base_env<T: LispContext>(env: std::sync::Arc<Env<T>>) {
    // ------------------------------- Functions  ------------------------------
    // Functions
    env.set_function(
        "funcall".into(),
        LispExp::primitive(primitive_funcall, Some(FUNCALL_DOC.into())),
    );
    env.set_function(
        "eval".into(),
        LispExp::primitive(primitive_eval, Some(EVAL_DOC.into())),
    );
    env.set_function(
        "throw".into(),
        LispExp::primitive(primitive_throw, Some(THROW_DOC.into())),
    );
    env.set_function(
        "signal".into(),
        LispExp::primitive(primitive_signal, Some(SIGNAL_DOC.into())),
    );
    env.set_function(
        "eval-string".into(),
        LispExp::primitive(primitive_eval_string, Some(EVAL_STRING_DOC.into())),
    );
    env.set_function(
        "eval-string-safe".into(),
        LispExp::primitive(
            primitive_eval_string_safe,
            Some(EVAL_STRING_SAFE_DOC.into()),
        ),
    );
    env.set_function(
        "function-doc".into(),
        LispExp::primitive(primitive_function_doc, Some(FUNCTION_DOC_DOC.into())),
    );
    env.set_function(
        "variable-doc".into(),
        LispExp::primitive(primitive_variable_doc, Some(VARIABLE_DOC_DOC.into())),
    );
    env.set_function(
        "function-arglist".into(),
        LispExp::primitive(
            primitive_function_arglist,
            Some(FUNCTION_ARGLIST_DOC.into()),
        ),
    );
    env.set_function(
        "boundp".into(),
        LispExp::primitive(primitive_boundp, Some(BOUNDP_DOC.into())),
    );
    env.set_function(
        "symbol-value".into(),
        LispExp::primitive(primitive_symbol_value, Some(SYMBOL_VALUE_DOC.into())),
    );
    env.set_function(
        "all-functions".into(),
        LispExp::primitive(primitive_all_functions, Some(ALL_FUNCTIONS_DOC.into())),
    );
    env.set_function(
        "all-macros".into(),
        LispExp::primitive(primitive_all_macros, Some(ALL_MACROS_DOC.into())),
    );
    env.set_function(
        "all-variables".into(),
        LispExp::primitive(primitive_all_variables, Some(ALL_VARIABLES_DOC.into())),
    );
    // Multithreading
    env.set_function(
        "make-atom".into(),
        LispExp::primitive(primitive_make_atom, None),
    );
    env.set_function(
        "deref".into(),
        LispExp::primitive(primitive_deref, Some(DEREF_DOC.into())),
    );
    env.set_function(
        "reset".into(),
        LispExp::primitive(primitive_reset, Some(RESET_DOC.into())),
    );
    env.set_function(
        "fiber-done-p".into(),
        LispExp::primitive(primitive_fiber_done_p, Some(FIBER_DONE_P_DOC.into())),
    );
    env.set_function(
        "resume".into(),
        LispExp::primitive(primitive_resume, Some(RESUME_DOC.into())),
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
        LispExp::primitive(primitive_compare, Some(COMPARE_DOC.into())),
    );
    env.set_function(
        "*".into(),
        LispExp::primitive(primitive_mul, Some(MUL_DOC.into())),
    );
    env.set_function(
        "/".into(),
        LispExp::primitive(primitive_div, Some(DIV_DOC.into())),
    );
    env.set_function("mod".into(), LispExp::primitive(primitive_mod, None));
    env.set_function("%".into(), LispExp::primitive(primitive_mod, None));
    env.set_function(
        "1+".into(),
        LispExp::primitive(primitive_1plus, Some(N_1PLUS_DOC.into())),
    );
    env.set_function(
        "1-".into(),
        LispExp::primitive(primitive_1minus, Some(N_1MINUS_DOC.into())),
    );
    env.set_function(
        "<".into(),
        LispExp::primitive(primitive_lt, Some(LT_DOC.into())),
    );
    env.set_function(
        ">".into(),
        LispExp::primitive(primitive_gt, Some(GT_DOC.into())),
    );
    env.set_function(
        "<=".into(),
        LispExp::primitive(primitive_le, Some(LE_DOC.into())),
    );
    env.set_function(
        ">=".into(),
        LispExp::primitive(primitive_ge, Some(GE_DOC.into())),
    );
    env.set_function(
        "max".into(),
        LispExp::primitive(primitive_max, Some(MAX_DOC.into())),
    );
    env.set_function(
        "min".into(),
        LispExp::primitive(primitive_min, Some(MIN_DOC.into())),
    );
    env.set_function(
        "abs".into(),
        LispExp::primitive(primitive_abs, Some(ABS_DOC.into())),
    );

    // Symbol plist
    env.set_function(
        "put".into(),
        LispExp::primitive(primitive_put, Some(PUT_DOC.into())),
    );
    env.set_function(
        "get".into(),
        LispExp::primitive(primitive_get, Some(GET_DOC.into())),
    );

    // List manipulation
    env.set_function(
        "add-to-list".into(),
        LispExp::primitive(primitive_add_to_list, Some(ADD_TO_LIST_DOC.into())),
    );
    env.set_function(
        "append-to-list".into(),
        LispExp::primitive(primitive_append_to_list, Some(APPEND_TO_LIST_DOC.into())),
    );
    env.set_function(
        "remove-from-list".into(),
        LispExp::primitive(
            primitive_remove_from_list,
            Some(REMOVE_FROM_LIST_DOC.into()),
        ),
    );
    env.set_function(
        "car".into(),
        LispExp::primitive(primitive_car, Some(CAR_DOC.into())),
    );
    env.set_function(
        "cdr".into(),
        LispExp::primitive(primitive_cdr, Some(CDR_DOC.into())),
    );
    env.set_function(
        "cons".into(),
        LispExp::primitive(primitive_cons, Some(CONS_DOC.into())),
    );
    env.set_function(
        "list".into(),
        LispExp::primitive(primitive_list, Some(LIST_DOC.into())),
    );
    env.set_function(
        "nth".into(),
        LispExp::primitive(primitive_nth, Some(NTH_DOC.into())),
    );
    env.set_function(
        "nthcdr".into(),
        LispExp::primitive(primitive_nthcdr, Some(NTHCDR_DOC.into())),
    );
    env.set_function(
        "length".into(),
        LispExp::primitive(primitive_length, Some(LENGTH_DOC.into())),
    );
    env.set_function(
        "append".into(),
        LispExp::primitive(primitive_append, Some(APPEND_DOC.into())),
    );
    env.set_function(
        "reverse".into(),
        LispExp::primitive(primitive_reverse, Some(REVERSE_DOC.into())),
    );
    env.set_function(
        "member".into(),
        LispExp::primitive(primitive_member, Some(MEMBER_DOC.into())),
    );
    env.set_function(
        "memq".into(),
        LispExp::primitive(primitive_memq, Some(MEMQ_DOC.into())),
    );
    env.set_function(
        "assoc".into(),
        LispExp::primitive(primitive_assoc, Some(ASSOC_DOC.into())),
    );
    env.set_function(
        "assq".into(),
        LispExp::primitive(primitive_assq, Some(ASSQ_DOC.into())),
    );
    env.set_function(
        "elt".into(),
        LispExp::primitive(primitive_elt, Some(ELT_DOC.into())),
    );
    env.set_function(
        "mapcar".into(),
        LispExp::primitive(primitive_mapcar, Some(MAPCAR_DOC.into())),
    );
    env.set_function(
        "mapc".into(),
        LispExp::primitive(primitive_mapc, Some(MAPC_DOC.into())),
    );
    env.set_function(
        "apply".into(),
        LispExp::primitive(primitive_apply, Some(APPLY_DOC.into())),
    );
    env.set_function(
        "identity".into(),
        LispExp::primitive(primitive_identity, Some(IDENTITY_DOC.into())),
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
        LispExp::primitive(primitive_equal, Some(EQUAL_DOC.into())),
    );
    env.set_function(
        "null".into(),
        LispExp::primitive(primitive_null, Some(NULL_DOC.into())),
    );
    env.set_function(
        "not".into(),
        LispExp::primitive(primitive_not, Some(NOT_DOC.into())),
    );
    env.set_function(
        "consp".into(),
        LispExp::primitive(primitive_consp, Some(CONSP_DOC.into())),
    );
    env.set_function("listp".into(), LispExp::primitive(primitive_listp, None));
    env.set_function(
        "stringp".into(),
        LispExp::primitive(primitive_stringp, Some(STRINGP_DOC.into())),
    );
    env.set_function(
        "numberp".into(),
        LispExp::primitive(primitive_numberp, Some(NUMBERP_DOC.into())),
    );
    env.set_function(
        "symbolp".into(),
        LispExp::primitive(primitive_symbolp, Some(SYMBOLP_DOC.into())),
    );
    env.set_function(
        "functionp".into(),
        LispExp::primitive(primitive_functionp, None),
    );
    env.set_function(
        "vectorp".into(),
        LispExp::primitive(primitive_vectorp, Some(VECTORP_DOC.into())),
    );
    env.set_function(
        "zerop".into(),
        LispExp::primitive(primitive_zerop, Some(ZEROP_DOC.into())),
    );

    env.set_function(
        "atom".into(),
        LispExp::primitive(primitive_atom_predicate, None),
    );

    // ------------------------------- Strings ------------------------------
    env.set_function(
        "floor".into(),
        LispExp::primitive(primitive_floor, Some(FLOOR_DOC.into())),
    );
    env.set_function(
        "make-string".into(),
        LispExp::primitive(primitive_make_string, Some(MAKE_STRING_DOC.into())),
    );
    env.set_function(
        "concat".into(),
        LispExp::primitive(primitive_concat, Some(CONCAT_DOC.into())),
    );
    env.set_function(
        "string=".into(),
        LispExp::primitive(primitive_string_eq, Some(STRING_EQ_DOC.into())),
    );
    env.set_function(
        "string<".into(),
        LispExp::primitive(primitive_string_lt, Some(STRING_LT_DOC.into())),
    );
    env.set_function(
        "substring".into(),
        LispExp::primitive(primitive_substring, Some(SUBSTRING_DOC.into())),
    );
    env.set_function(
        "upcase".into(),
        LispExp::primitive(primitive_upcase, Some(UPCASE_DOC.into())),
    );
    env.set_function(
        "downcase".into(),
        LispExp::primitive(primitive_downcase, Some(DOWNCASE_DOC.into())),
    );
    env.set_function(
        "format".into(),
        LispExp::primitive(primitive_format, Some(FORMAT_DOC.into())),
    );
    env.set_function(
        "number-to-string".into(),
        LispExp::primitive(
            primitive_number_to_string,
            Some(NUMBER_TO_STRING_DOC.into()),
        ),
    );
    env.set_function(
        "string-to-number".into(),
        LispExp::primitive(
            primitive_string_to_number,
            Some(STRING_TO_NUMBER_DOC.into()),
        ),
    );
    env.set_function(
        "symbol-name".into(),
        LispExp::primitive(primitive_symbol_name, Some(SYMBOL_NAME_DOC.into())),
    );
    env.set_function(
        "intern".into(),
        LispExp::primitive(primitive_intern, Some(INTERN_DOC.into())),
    );
    env.set_function(
        "split-string".into(),
        LispExp::primitive(primitive_split_string, Some(SPLIT_STRING_DOC.into())),
    );
}
