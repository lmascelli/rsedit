//!
//! Add a general description of the base types here
//!

use super::{Env, EvalError, LispContext, LispExp};
use std::sync::{Arc, RwLock};

// ---------------------------------  Cons cell  -------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ConsCell<T: LispContext> {
    pub car: LispExp<T>,
    pub cdr: LispExp<T>,
}

/// Unlink a cons chain iteratively when it is dropped.
///
/// Without this, dropping a list recurses once per cell -- `Arc<ConsCell>`
/// drops its `cdr`, which drops the next cell, and so on -- so letting go of a
/// list of a few tens of thousands of elements overflows the stack and
/// *aborts* the process. Not a catchable panic: `condition-case` cannot help,
/// and the editor simply dies. Building a list that long is ordinary Lisp, and
/// the crash happens when it goes out of scope, far from anything that looks
/// like a cause.
///
/// The loop walks the chain taking ownership of each cell whose last reference
/// this is, replacing its `cdr` with `nil` before the cell itself is dropped,
/// so no drop ever nests. A cell that is still shared ends the walk: its tail
/// belongs to someone else and is not ours to unlink.
impl<T: LispContext> Drop for ConsCell<T> {
    fn drop(&mut self) {
        let mut next = std::mem::replace(&mut self.cdr, LispExp::nil());
        while let LispExp::Cons(cell) = next {
            match Arc::try_unwrap(cell) {
                Ok(mut owned) => next = std::mem::replace(&mut owned.cdr, LispExp::nil()),
                Err(_) => break,
            }
        }
    }
}

pub struct ConsIter<T: LispContext> {
    pub(super) cursor: LispExp<T>,
}

impl<T: LispContext> Iterator for ConsIter<T> {
    type Item = LispExp<T>;
    fn next(&mut self) -> Option<Self::Item> {
        match &self.cursor {
            LispExp::Cons(cell) => {
                let cell = cell.clone();
                self.cursor = cell.cdr.clone();
                Some(cell.car.clone())
            }
            _ => None,
        }
    }
}

// ----------------------------------  Lambda ----------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Lambda<T: LispContext> {
    /// Required parameters -- a call must supply exactly one argument for
    /// each of these.
    pub params: Vec<String>,
    /// `&optional` parameters. A call may omit any suffix of these;
    /// omitted ones are bound to `nil`.
    pub optionals: Vec<String>,
    /// The `&rest` parameter, if any. Bound to a list of every argument
    /// past `params`/`optionals`. `None` means the lambda has no `&rest`
    /// parameter, so supplying more arguments than `params.len() +
    /// optionals.len()` is an arity error.
    pub rest: Option<String>,
    pub body: Vec<LispExp<T>>,
    pub env: Arc<Env<T>>,
    pub doc: Option<Arc<String>>,
}

// --------------------------------  Shared Atom -------------------------------

#[derive(Clone, Debug)]
pub struct SharedAtom<T: LispContext>(pub Arc<RwLock<LispExp<T>>>);

impl<T: LispContext> PartialEq for SharedAtom<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

// -----------------------------------  Fiber ----------------------------------

#[derive(Debug)]
pub struct FiberState<T: LispContext> {
    pub body: Vec<LispExp<T>>,
    pub env: Arc<Env<T>>,
    pub is_done: bool,
}

#[derive(Clone, Debug)]
pub struct SharedFiber<T: LispContext>(pub Arc<RwLock<FiberState<T>>>);

impl<T: LispContext> PartialEq for SharedFiber<T> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

// --------------------------------  Primitive  --------------------------------

pub type LispPrimitive<T> = fn(&[LispExp<T>], Arc<Env<T>>, &T) -> Result<LispExp<T>, EvalError<T>>;
