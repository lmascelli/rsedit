//! Cons cells, and everything made of them.
//!
//! The largest section, and the oldest: a Lisp is its list operations. `car`
//! and `cdr` walk the chain in place; most of the rest materialise it, which
//! is why they charge fuel per element.
use super::*;

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
    exact_arity(args, 1)?;
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
    exact_arity(args, 1)?;
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
    exact_arity(args, 2)?;
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
    exact_arity(args, 2)?;
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
    exact_arity(args, 2)?;
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
    exact_arity(args, 1)?;
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
    exact_arity(args, 1)?;
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
    exact_arity(args, 2)?;
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
    exact_arity(args, 2)?;
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
    exact_arity(args, 2)?;
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
    some_arguments(args)?;
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
    exact_arity(args, 1)?;
    Ok(args[0].clone())
}

/// Register this module's primitives: cons cells, and everything made of them.
///
/// Called by [`super::setup_base_env`]. Here rather than there because a
/// primitive's name, its implementation and its docstring are one fact in three
/// pieces, and they were a thousand lines apart.
pub(super) fn install<T: LispContext>(into: &Registry<T>) {
    // List manipulation
    into.function("add-to-list", primitive_add_to_list, ADD_TO_LIST_DOC);
    into.function(
        "append-to-list",
        primitive_append_to_list,
        APPEND_TO_LIST_DOC,
    );
    into.function(
        "remove-from-list",
        primitive_remove_from_list,
        REMOVE_FROM_LIST_DOC,
    );
    into.function("car", primitive_car, CAR_DOC);
    into.function("cdr", primitive_cdr, CDR_DOC);
    into.function("cons", primitive_cons, CONS_DOC);
    into.function("list", primitive_list, LIST_DOC);
    into.function("nth", primitive_nth, NTH_DOC);
    into.function("nthcdr", primitive_nthcdr, NTHCDR_DOC);
    into.function("length", primitive_length, LENGTH_DOC);
    into.function("append", primitive_append, APPEND_DOC);
    into.function("reverse", primitive_reverse, REVERSE_DOC);
    into.function("member", primitive_member, MEMBER_DOC);
    into.function("memq", primitive_memq, MEMQ_DOC);
    into.function("assoc", primitive_assoc, ASSOC_DOC);
    into.function("assq", primitive_assq, ASSQ_DOC);
    into.function("elt", primitive_elt, ELT_DOC);
    into.function("mapcar", primitive_mapcar, MAPCAR_DOC);
    into.function("mapc", primitive_mapc, MAPC_DOC);
    into.function("apply", primitive_apply, APPLY_DOC);
    into.function("identity", primitive_identity, IDENTITY_DOC);
}
