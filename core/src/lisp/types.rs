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

/// One suspended block of a fiber: the forms it had left, and where it was.
///
/// # What a continuation is, here
///
/// A fiber that yields has to come back to the middle of its own program, and
/// the evaluator is an ordinary recursive Rust function -- the place it was is
/// the Rust stack, which is exactly what unwinding destroys. So on the way out
/// each block that *can* say where it was says so, and the list of those
/// answers is the continuation. Resuming replays them from the inside out.
///
/// # Why this is enough, and what it costs
///
/// A block can only record its position when the value of what it was running
/// is going to be thrown away -- a statement in a body, an iteration of a
/// loop. In *those* positions, coming back means "run the rest of these
/// forms", which is what this holds. In any other position -- an argument
/// half-way through being evaluated, a condition being decided, a primitive
/// part-way through its own Rust code -- coming back would mean delivering a
/// value into a computation that no longer exists, and nothing short of
/// rewriting the evaluator as a state machine can do that.
///
/// So `(yield)` is refused there rather than half-supported. See
/// `EvalError::YieldNotAllowed`. That is the same boundary Lua draws around
/// its coroutines, and it is drawn here by construction rather than by a list
/// of checks: permission to yield is *granted* by the positions that can
/// record, and consumed by the next evaluation, so a position that was never
/// taught to record never has it to give away.
#[derive(Debug)]
pub enum Frame<T: LispContext> {
    /// The rest of a body -- a `progn`, a `let`, a function -- run for effect
    /// in `env`, starting at `from`.
    Body {
        forms: Vec<LispExp<T>>,
        from: usize,
        env: Arc<Env<T>>,
    },
    /// The rest of a `while` iteration, starting at `from`, and then the loop
    /// itself: the condition is tested again and the body runs from the top,
    /// exactly as if nothing had interrupted it.
    While {
        condition: LispExp<T>,
        forms: Vec<LispExp<T>>,
        from: usize,
        env: Arc<Env<T>>,
    },
}

/// Identity, for the reason [`SharedAtom`]'s is identity: a frame is a
/// position in a running program, and two of them are the same only when they
/// are the same one. Comparing them structurally would mean comparing the
/// captured `Env`s, whose own `PartialEq` is `unreachable!()`.
///
/// It exists at all only because `EvalError` derives `PartialEq` and carries a
/// suspension, and tests compare `EvalError`s.
impl<T: LispContext> PartialEq for Frame<T> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

#[derive(Debug)]
pub struct FiberState<T: LispContext> {
    pub body: Vec<LispExp<T>>,
    pub env: Arc<Env<T>>,
    pub is_done: bool,
    /// Where this fiber stopped, when it stopped in the middle of a form.
    ///
    /// Empty for a fiber that has never yielded, which is every fiber the
    /// editor had before `(yield)` existed -- so `resume` still means "run the
    /// next form of the body" for those, and means "carry on from where you
    /// were" for the rest. The two are not alternatives: a fiber whose third
    /// form yields twice resumes twice inside that form and then goes on to
    /// the fourth.
    pub pending: Vec<Frame<T>>,
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
