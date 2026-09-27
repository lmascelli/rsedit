use super::{Frame, LispContext, LispExp};

#[derive(Debug, PartialEq)]
pub enum EvalError<T: LispContext> {
    UnboundVariable(String),
    UndefinedFunction(String),
    UnvalidFunctionCall,
    UncorrectFunctionDefinition,
    WrongNumberOfArguments {
        expected: usize,
        got: usize,
    },
    WrongArgumentType {
        expected: String,
        got: LispExp<T>,
    },
    QuoteNotOneArgument,
    IfNoConditionProvided,
    IfNoTrueBrach,
    SetqSymbolRequired,
    SetqWrongNumberOfArgs(usize),
    DefunNameMustBeASymbol,
    DefunNotCorrectExpression,
    DefunParamsAreNotAList,
    DefunParamIsNotASymbol,
    DefunRestMustHaveExactlyOneParam,
    DefunMisplacedParamMarker,
    LetUnvalidBindingAt(usize),
    LetUnvalidBindingList,
    LetNoBindingsProvided,
    CondInvalidClause,
    DolistInvalidBinding,
    DotimesInvalidBinding,
    DefvarNameMustBeASymbol,
    BackquoteNotOneArgument,
    OutOfFuel,
    ConditionCaseInvalidVariable,
    ConditionCaseInvalidHandler,
    /// A `(throw TAG VALUE)` in flight, looking for its `catch`. Carrying the
    /// payload here rather than out of band is the whole reason this enum is
    /// generic over the context type.
    Throw {
        tag: LispExp<T>,
        value: LispExp<T>,
    },
    /// A `(yield VALUE)` in flight, looking for the `resume` that will catch
    /// it.
    ///
    /// Not an error, and on the error channel anyway -- the same arrangement
    /// `Throw` above uses, and for the same reason: unwinding is how a Rust
    /// evaluator leaves the middle of a computation, and `?` already
    /// propagates it through every frame without a single call site being
    /// rewritten.
    ///
    /// The difference from `Throw` is what rides along. A throw is going
    /// somewhere and will not come back, so it carries only what to deliver; a
    /// yield *is* coming back, so it carries the way back -- see [`Frame`].
    /// The frames are innermost first, in the order the blocks pushed them on
    /// the way out.
    ///
    /// Nothing but `resume` may catch this. `condition-case` does not, and
    /// does not need to refuse it either: a yield cannot originate inside one,
    /// because the permission that makes a yield legal is never granted
    /// there.
    Yielded {
        value: LispExp<T>,
        frames: Vec<Frame<T>>,
    },
    /// A `(yield ...)` in a position the evaluator could not record a way back
    /// to: an argument being evaluated, a condition being decided, a
    /// `dolist` body, a `condition-case` or an `unwind-protect`, or anywhere
    /// inside a primitive that called back into Lisp.
    ///
    /// Refused rather than half-supported, and refused *before* the value form
    /// runs, so a program that yields somewhere impossible does not get its
    /// side effects half-done. See [`Frame`] for where the boundary is and
    /// why it is there.
    YieldNotAllowed,
    /// A `(signal SYMBOL DATA)` raised from Lisp. `symbol` is the condition a
    /// handler matches on; `data` is whatever the caller attached.
    Signal {
        symbol: LispExp<T>,
        data: LispExp<T>,
    },
    RuntimeMessage(String),
}
