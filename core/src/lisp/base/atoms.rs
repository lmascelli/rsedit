//! The one mutable thing that crosses a thread.
//!
//! An atom is a box several threads may read and write. Everything else a
//! thread touches is its own, which is the whole of this interpreter's
//! concurrency story -- and the reason that story fits in three primitives.
use super::*;

const MAKE_ATOM_DOC: &str = "(make-atom VALUE): Return a new atom holding VALUE -- a box \
                 that several threads may read and write safely.\n\n\
                 Read it with `deref' and replace what is in it with `reset'. This is the only \
                 mutable thing that crosses a thread boundary; everything else a thread touches \
                 is its own.\n\n\
                 Example:\n\
                 (setq counter (make-atom 0))\n\
                 (reset counter 1)\n\
                 (deref counter) => 1";

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

/// Register this module's primitives: the one mutable thing that crosses a thread.
///
/// Called by [`super::setup_base_env`]. Here rather than there because a
/// primitive's name, its implementation and its docstring are one fact in three
/// pieces, and they were a thousand lines apart.
pub(super) fn install<T: LispContext>(into: &Registry<T>) {
    // Multithreading
    into.function("make-atom", primitive_make_atom, MAKE_ATOM_DOC);
    into.function("deref", primitive_deref, DEREF_DOC);
    into.function("reset", primitive_reset, RESET_DOC);
}
