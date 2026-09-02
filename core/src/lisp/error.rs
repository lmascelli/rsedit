use super::{LispContext, LispExp};

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
    /// A `(signal SYMBOL DATA)` raised from Lisp. `symbol` is the condition a
    /// handler matches on; `data` is whatever the caller attached.
    Signal {
        symbol: LispExp<T>,
        data: LispExp<T>,
    },
    RuntimeMessage(String),
}
