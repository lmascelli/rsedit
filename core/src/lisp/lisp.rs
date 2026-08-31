use std::{
    borrow::Cow,
    cell::Cell,
    cmp::PartialEq,
    collections::HashMap,
    fmt::Debug,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU32, Ordering},
    },
};

// ========================================================================== //
//               +------------------------------------------+
//               |  Execution metering for the interpreter. |
//               +------------------------------------------+
// The evaluator charges the host on every step through
// `LispContext::consume_fuel`, but how large a budget is, and when it
// refills, is host *policy*. This module supplies the *mechanism* that policy
// needs, so that each embedder does not have to re-derive the thread-local
// bookkeeping, the nesting rules, and the arithmetic edge cases for itself.
//
// It knows nothing about context. A host decides what constitutes
// one metered unit of work by wrapping it in [`FuelMeter::begin`].
// ========================================================================== //

/// Default steps one metered scope may run before it is aborted.
///
/// This is a runaway-loop guard, not a resource quota: it exists so a mistyped
/// `(while t ...)` cannot hang its host forever. Sized from the interpreter
/// performance suite's measured cost per eval step -- roughly 100ns in release
/// and 500ns in debug -- so a runaway scope costs about a second of
/// unresponsiveness in release and a few seconds in debug before control comes
/// back. Ordinary work uses a vanishing fraction of it.
pub const DEFAULT_FUEL: u32 = 10_000_000;

thread_local! {
    /// Steps remaining for the metered scope running on *this* thread.
    ///
    /// Thread-local rather than shared state on the meter, for two reasons.
    /// Correctness: the `(spawn ...)` special form evaluates on a second
    /// thread while sharing the host context, so a shared counter would let
    /// that thread spend the foreground scope's budget -- and let a new
    /// foreground scope refill it underneath a running background one. Speed:
    /// only the owning thread can reach its own copy, so no synchronisation is
    /// required at all, making this a plain `Cell` load/store rather than an
    /// atomic read-modify-write on a line other cores may be contending for.
    ///
    /// The `const` initialiser avoids the hidden "has this thread initialised
    /// it yet?" branch a non-const one would add to every access. It starts at
    /// `DEFAULT_FUEL` rather than zero so that a thread which never opens a
    /// scope is still *bounded* -- failing safe -- instead of either dying on
    /// its first step or running unmetered.
    static FUEL: Cell<u32> = const { Cell::new(DEFAULT_FUEL) };

    /// Nesting depth of [`FuelMeter::begin`] on this thread. Only the 0 -> 1
    /// transition refills, so code that re-enters the evaluator cannot hand
    /// itself a fresh budget partway through an existing scope.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Guard returned by [`FuelMeter::begin`]; closes the scope when dropped.
///
/// RAII rather than paired begin/end calls so that an early return or a `?`
/// cannot leave the depth stuck above zero, which would permanently prevent
/// any further refill.
#[must_use = "the metered scope ends as soon as this guard is dropped"]
pub struct FuelScope<'a> {
    _meter: &'a FuelMeter,
}

impl Drop for FuelScope<'_> {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// A refillable execution budget shared by every thread of one host.
///
/// The configured budget lives here (shared, read once per scope); the
/// remaining count lives in thread-local storage (private, touched every
/// step). That split is deliberate: the hot counter never needs
/// synchronisation, while the setting stays global and adjustable.
#[derive(Debug)]
pub struct FuelMeter {
    budget: AtomicU32,
}

impl FuelMeter {
    pub fn new(budget: u32) -> Self {
        Self {
            budget: AtomicU32::new(budget),
        }
    }

    pub fn consume(&self, amount: u32) -> Result<(), EvalError> {
        FUEL.with(|fuel| match fuel.get().checked_sub(amount) {
            Some(remaining) => {
                fuel.set(remaining);
                Ok(())
            }
            None => Err(EvalError::OutOfFuel),
        })
    }

    /// Open a metered scope, refilling this thread's budget if it is the
    /// outermost one. Nested calls only deepen the count.
    pub fn begin(&self) -> FuelScope<'_> {
        DEPTH.with(|depth| {
            if depth.get() == 0 {
                FUEL.set(self.budget.load(Ordering::Relaxed));
            }
            depth.set(depth.get() + 1);
        });
        FuelScope { _meter: self }
    }

    /// Arm the calling thread's budget without opening a scope, for a freshly
    /// spawned thread that has no enclosing scope to nest inside. Without this
    /// such a thread would run on the compile-time `DEFAULT_FUEL` rather than
    /// the host's configured budget.
    pub fn arm_thread(&self) {
        FUEL.set(self.budget.load(Ordering::Relaxed));
    }

    /// Set the budget future scopes receive, and top the current thread's
    /// remaining fuel up to it -- so code that knows it will be expensive can
    /// raise its own ceiling as its first act rather than having to restart.
    pub fn set_budget(&self, budget: u32) {
        self.budget.store(budget, Ordering::Relaxed);
        FUEL.set(budget);
    }
}

// ========================================================================== //
//                 +------------------------------------------+
//                 |  Context that can embed the interpreter  |
//                 +------------------------------------------+
// ========================================================================== //

pub trait LispContext: Clone + PartialEq + Debug + Send + Sync + 'static {
    /// Consumes a given amount of execution ticks.
    /// Returns `Err(EvalError::OutOfFuel)` if the host-defined budget is exhausted.
    fn consume_fuel(&self, amount: u32) -> Result<(), EvalError>;

    /// Allows the VM to bubble up non-fatal diagnostic logs, trace statements,
    /// or debugging notices to the host without knowing how the host presents them.
    fn log_diagnostic(&self, msg: &str);

    /// Called by the evaluator when it begins evaluating on a **newly created
    /// thread** -- currently only the `(spawn ...)` special form.
    ///
    /// A host that meters execution needs this because metering state is
    /// naturally *per-thread*: a budget belongs to one line of execution, and
    /// nothing about a parent thread's remaining allowance is meaningful to a
    /// child that runs concurrently with it. Since the evaluator is the only
    /// thing that knows a thread was just created, only the evaluator can tell
    /// the host to arm it.
    ///
    /// Unlike the scope-based entry points a host drives itself, this takes no
    /// guard and needs no matching "end" call: a fresh thread has no enclosing
    /// scope to nest inside or unwind back to, and its state dies with it.
    ///
    /// Default: a no-op, so hosts that do not meter pay nothing.
    fn begin_thread_evaluation(&self) {}

    /// Called by the evaluator right before running the body of a function
    /// call -- a named function, a primitive, or an inline lambda
    /// application -- with a short description of the frame (typically the
    /// function's name, or "<lambda>" for an anonymous one). A host that
    /// wants a call stack for backtraces overrides this (and
    /// `pop_call_frame`); the default is a no-op, so hosts that don't care
    /// pay nothing.
    ///
    /// The intended protocol: pop the frame when the call *succeeds*, but
    /// leave it in place when it fails, so that by the time an error has
    /// finished propagating out to whoever is watching for it, the frames
    /// still standing are exactly the chain of calls that were active at
    /// the moment things went wrong -- a backtrace frozen at throw time,
    /// not inspected after the stack has already unwound. `eval` follows
    /// this protocol at every call site; a host only needs to store and
    /// clear the frames.
    ///
    /// One caveat worth knowing: this reflects genuine call nesting, not
    /// full Lisp call semantics. A call in tail position is evaluated by
    /// the trampoline in `eval` *after* its caller's frame has already
    /// been popped -- that's the whole point of tail-call elimination, the
    /// caller's frame is gone, not still waiting -- so a chain of tail
    /// calls won't show up as a chain of frames, only the innermost one
    /// still "on the stack" will. Every non-tail call (argument
    /// evaluation, all but the last form in a body, and any nested
    /// `eval`/`funcall` performed by a primitive) is captured correctly.
    fn push_call_frame(&self, _frame: &str) {}
    /// Pop the most recently pushed frame, undoing one `push_call_frame`.
    fn pop_call_frame(&self) {}

    /// How many frames are currently pushed. Combined with
    /// `truncate_call_frames`, this lets a primitive that *catches* an
    /// error -- swallowing it into a returned value instead of letting it
    /// propagate, e.g. `eval-string-safe` -- restore the frame stack to
    /// how it was before its own (now-discarded) nested evaluation. That
    /// nested evaluation may have pushed frames that never got to pop
    /// (their call failed, per the `push_call_frame` protocol); without
    /// this they'd leak into whatever *actually* uncaught error is
    /// reported next, showing an unrelated, already-handled failure in a
    /// fresh backtrace.
    fn call_frame_depth(&self) -> usize {
        0
    }
    /// Pop frames until exactly `depth` remain (a no-op if already at or
    /// below `depth`).
    fn truncate_call_frames(&self, _depth: usize) {}
}

// ========================================================================== //
//                    +----------------------------------------+
//                    |  Parsing system for the lisp language  |
//                    +----------------------------------------+
// ========================================================================== //

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Token {
    Uninitialized,
    Void,
    LParen,
    RParen,
    LSquared,
    RSquared,
    LBracket,
    RBracket,
    Quote,
    BackQuote,
    Comma,
    CommaAt,
    Dot,
    Number(f64),
    String(String),
    Symbol(String),
}

enum ParserLexerState {
    Default,
    InComment,
    InSymbol,
    InString,
    InStringSlash,
    InNumber,
    InNumberMinusStart,
    InNumberAfterDot,
    InDotStart,
}

#[derive(Debug, PartialEq)]
pub enum ParserError {
    // Lexing
    UnbalancedRParen,
    UnbalancedRSquared,
    UnbalancedRBracket,
    NumberParseError(String),
    NumberInvadidChar(char),
    UnclosedString,
    // Parsing
    VoidExp,
    UnclosedList,
    UnclosedVector,
    UnclosedMap,
    InvalidMapKey,
    MapKeyMissingValue,
    UnexpectedDot,
    MalformedDottedList,
}

pub struct Parser<'source> {
    source: std::iter::Peekable<std::str::Chars<'source>>,
    token: String,
    current_token: Token,
    parens_stack: Vec<Token>,
    lexer_state: ParserLexerState,
}

// ========================================================================== //
//                       +--------------------------------+
//                       |  Lisp language implementation  |
//                       +--------------------------------+
// ========================================================================== //

#[derive(Debug, PartialEq)]
pub enum EvalError {
    UnboundVariable(String),
    UndefinedFunction(String),
    UnvalidFunctionCall,
    UncorrectFunctionDefinition,
    WrongNumberOfArguments { expected: usize, got: usize },
    WrongArgumentType { expected: String, got: String },
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
    RuntimeMessage(String),
}

// -------------------------------  Environment --------------------------------

#[derive(Debug)]
pub struct Env<T: LispContext> {
    pub variables: RwLock<HashMap<String, LispExp<T>>>,
    pub functions: RwLock<HashMap<String, LispExp<T>>>,
    pub macros: RwLock<HashMap<String, LispExp<T>>>,
    pub parent: Option<Arc<Env<T>>>,
}

// ---------------------------------  Cons cell  -------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ConsCell<T: LispContext> {
    pub car: LispExp<T>,
    pub cdr: LispExp<T>,
}

pub struct ConsIter<T: LispContext> {
    cursor: LispExp<T>,
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

pub type LispPrimitive<T> = fn(&[LispExp<T>], Arc<Env<T>>, &T) -> Result<LispExp<T>, EvalError>;

// --------------------------------  LispExp  ----------------------------------

#[derive(Clone, Debug, PartialEq)]
// Primitive comparison has no meaning and will probably never done
#[allow(unpredictable_function_pointer_comparisons)]
pub enum LispExp<T: LispContext> {
    /// A *syntax* node: the vector-backed form the reader produces and
    /// `eval` dispatches on. Never a runtime value.
    List(Arc<Vec<LispExp<T>>>),

    /// A *data* list. `nil` terminates a proper list; anything else
    /// terminates an improper (dotted) one, so `DottedList` is gone.
    Cons(Arc<ConsCell<T>>),

    DottedList(Arc<Vec<LispExp<T>>>, Arc<LispExp<T>>), // TODO remove
    Vector(Arc<Vec<LispExp<T>>>),
    Map(Arc<HashMap<String, LispExp<T>>>),
    Number(f64),
    Symbol(Arc<String>),
    String(Arc<String>),
    Lambda(Arc<Lambda<T>>),
    Primitive {
        pointer: LispPrimitive<T>,
        doc: Arc<Cow<'static, str>>,
    },
    Atom(SharedAtom<T>),
    Fiber(SharedFiber<T>),
}

// ========================================================================== //
//                           +--------------------------+
//                           |  Methods implementation  |
//                           +--------------------------+
// ========================================================================== //

/// Convert a form produced by the reader into the data it denotes.
/// Runs once, at read time, on quoted structure only.
pub fn form_to_data<T: LispContext>(exp: &LispExp<T>) -> LispExp<T> {
    match exp {
        LispExp::List(items) => {
            LispExp::proper_list(items.iter().map(form_to_data).collect())
        }
        LispExp::Vector(items) => {
            LispExp::vec(items.iter().map(form_to_data).collect())
        }
        LispExp::Map(m) => LispExp::map(
            m.iter().map(|(k, v)| (k.clone(), form_to_data(v))).collect(),
        ),
        other => other.clone(),
    }
}

impl<'source> Parser<'source> {
    pub fn new(source: &'source str) -> Self {
        Self {
            source: source.chars().peekable(),
            token: String::new(),
            current_token: Token::Uninitialized,
            parens_stack: Vec::new(),
            lexer_state: ParserLexerState::Default,
        }
    }

    pub(super) fn next_token(&mut self) -> Result<Option<Token>, ParserError> {
        while let Some(c) = self.source.peek() {
            match self.lexer_state {
                ParserLexerState::Default => match c {
                    ';' => {
                        self.lexer_state = ParserLexerState::InComment;
                    }
                    '\'' => {
                        self.source.next();
                        return Ok(Some(Token::Quote));
                    }
                    '`' => {
                        self.source.next();
                        return Ok(Some(Token::BackQuote));
                    }
                    ',' => {
                        self.source.next();
                        if self.source.peek() == Some(&'@') {
                            self.source.next();
                            return Ok(Some(Token::CommaAt));
                        }
                        return Ok(Some(Token::Comma));
                    }
                    '(' => {
                        self.parens_stack.push(Token::LParen);
                        self.source.next();
                        return Ok(Some(Token::LParen));
                    }
                    ')' => {
                        if Some(Token::LParen) == self.parens_stack.pop() {
                            self.source.next();
                            return Ok(Some(Token::RParen));
                        } else {
                            return Err(ParserError::UnbalancedRParen);
                        }
                    }
                    '[' => {
                        self.parens_stack.push(Token::LSquared);
                        self.source.next();
                        return Ok(Some(Token::LSquared));
                    }
                    ']' => {
                        if Some(Token::LSquared) == self.parens_stack.pop() {
                            self.source.next();
                            return Ok(Some(Token::RSquared));
                        } else {
                            return Err(ParserError::UnbalancedRSquared);
                        }
                    }
                    '{' => {
                        self.parens_stack.push(Token::LBracket);
                        self.source.next();
                        return Ok(Some(Token::LBracket));
                    }
                    '}' => {
                        if Some(Token::LBracket) == self.parens_stack.pop() {
                            self.source.next();
                            return Ok(Some(Token::RBracket));
                        } else {
                            return Err(ParserError::UnbalancedRBracket);
                        }
                    }
                    ' ' | '\t' | '\n' => {}
                    '"' => {
                        self.lexer_state = ParserLexerState::InString;
                    }
                    '-' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InNumberMinusStart;
                    }
                    '0'..='9' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InNumber;
                    }
                    '.' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InDotStart;
                    }
                    _ => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InSymbol;
                    }
                },
                ParserLexerState::InComment => match c {
                    '\n' => {
                        self.lexer_state = ParserLexerState::Default;
                    }
                    _ => {}
                },
                ParserLexerState::InSymbol => match c {
                    ';' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        self.lexer_state = ParserLexerState::InComment;
                        self.source.next();
                        return Ok(Some(Token::Symbol(token_string)));
                    }
                    ' ' | '\t' | '\n' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        self.lexer_state = ParserLexerState::Default;
                        self.source.next();
                        return Ok(Some(Token::Symbol(token_string)));
                    }
                    '(' | '[' | '{' | ')' | ']' | '}' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        self.lexer_state = ParserLexerState::Default;
                        return Ok(Some(Token::Symbol(token_string)));
                    }
                    _ => {
                        self.token.push(*c);
                    }
                },
                ParserLexerState::InString => match c {
                    '"' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        self.lexer_state = ParserLexerState::Default;
                        self.source.next();
                        return Ok(Some(Token::String(token_string)));
                    }
                    '\\' => {
                        self.lexer_state = ParserLexerState::InStringSlash;
                    }
                    _ => {
                        self.token.push(*c);
                    }
                },
                ParserLexerState::InStringSlash => match c {
                    '"' | '\\' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InString;
                    }
                    _ => {
                        self.token.push('\\');
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InString;
                    }
                },
                ParserLexerState::InNumber => match c {
                    ';' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        if let Ok(number) = token_string.parse() {
                            self.lexer_state = ParserLexerState::InComment;
                            return Ok(Some(Token::Number(number)));
                        } else {
                            return Err(ParserError::NumberParseError(token_string));
                        }
                    }
                    '0'..='9' => {
                        self.token.push(*c);
                    }
                    '.' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InNumberAfterDot;
                    }
                    '(' | '[' | '{' | ')' | ']' | '}' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        if let Ok(number) = token_string.parse() {
                            self.lexer_state = ParserLexerState::Default;
                            return Ok(Some(Token::Number(number)));
                        } else {
                            return Err(ParserError::NumberParseError(token_string));
                        }
                    }
                    ' ' | '\t' | '\n' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        if let Ok(number) = token_string.parse() {
                            self.lexer_state = ParserLexerState::Default;
                            self.source.next();
                            return Ok(Some(Token::Number(number)));
                        } else {
                            return Err(ParserError::NumberParseError(token_string));
                        }
                    }
                    _ => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InSymbol;
                    }
                },
                ParserLexerState::InNumberMinusStart => match c {
                    ';' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        self.lexer_state = ParserLexerState::InComment;
                        return Ok(Some(Token::Symbol(token_string)));
                    }
                    '0'..='9' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InNumber;
                    }
                    '.' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InNumberAfterDot;
                    }
                    '(' | '[' | '{' | ')' | ']' | '}' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        self.lexer_state = ParserLexerState::Default;
                        return Ok(Some(Token::Symbol(token_string)));
                    }
                    ' ' | '\t' | '\n' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        self.lexer_state = ParserLexerState::Default;
                        return Ok(Some(Token::Symbol(token_string)));
                    }
                    _ => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InSymbol;
                    }
                },
                ParserLexerState::InNumberAfterDot => match c {
                    ';' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        if let Ok(number) = token_string.parse() {
                            self.lexer_state = ParserLexerState::InComment;
                            return Ok(Some(Token::Number(number)));
                        } else {
                            return Err(ParserError::NumberParseError(token_string));
                        }
                    }
                    '0'..='9' => {
                        self.token.push(*c);
                    }
                    '(' | '[' | '{' | ')' | ']' | '}' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        if let Ok(number) = token_string.parse() {
                            self.lexer_state = ParserLexerState::Default;
                            return Ok(Some(Token::Number(number)));
                        } else {
                            return Err(ParserError::NumberParseError(token_string));
                        }
                    }
                    ' ' | '\t' | '\n' => {
                        let mut token_string = String::new();
                        core::mem::swap(&mut token_string, &mut self.token);
                        if let Ok(number) = token_string.parse() {
                            self.lexer_state = ParserLexerState::Default;
                            self.source.next();
                            return Ok(Some(Token::Number(number)));
                        } else {
                            return Err(ParserError::NumberParseError(token_string));
                        }
                    }
                    _ => {
                        self.token.push(*c);
                        return Err(ParserError::NumberParseError(self.token.clone()));
                    }
                },

                ParserLexerState::InDotStart => match c {
                    '0'..'9' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InNumberAfterDot;
                    }
                    ';' => {
                        self.token.clear();
                        self.lexer_state = ParserLexerState::InComment;
                        return Ok(Some(Token::Dot));
                    }
                    '(' | '[' | '{' | ')' | ']' | '}' => {
                        self.token.clear();
                        self.lexer_state = ParserLexerState::Default;
                        return Ok(Some(Token::Dot));
                    }
                    ' ' | '\t' | '\n' => {
                        self.token.clear();
                        self.lexer_state = ParserLexerState::Default;
                        return Ok(Some(Token::Dot));
                    }
                    _ => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InSymbol;
                    }
                },
            }
            self.source.next();
        }

        // handle pending token
        match self.lexer_state {
            ParserLexerState::Default => {
                if !self.token.is_empty() {
                    match self.token.as_str() {
                        ")" => {
                            if Some(Token::LParen) == self.parens_stack.pop() {
                                self.source.next();
                                return Ok(Some(Token::RParen));
                            } else {
                                return Err(ParserError::UnbalancedRParen);
                            }
                        }
                        "]" => {
                            if Some(Token::LSquared) == self.parens_stack.pop() {
                                self.source.next();
                                return Ok(Some(Token::RSquared));
                            } else {
                                return Err(ParserError::UnbalancedRSquared);
                            }
                        }
                        "}" => {
                            if Some(Token::LBracket) == self.parens_stack.pop() {
                                self.source.next();
                                return Ok(Some(Token::RBracket));
                            } else {
                                return Err(ParserError::UnbalancedRBracket);
                            }
                        }
                        _ => {
                            todo!("{:?}", self.token);
                        }
                    }
                } else {
                    return Ok(None);
                }
            }
            ParserLexerState::InComment => {
                return Ok(None);
            }
            ParserLexerState::InSymbol | ParserLexerState::InNumberMinusStart => {
                let mut token_string = String::new();
                core::mem::swap(&mut token_string, &mut self.token);
                self.source.next();
                return Ok(Some(Token::Symbol(token_string)));
            }
            ParserLexerState::InNumber | ParserLexerState::InNumberAfterDot => {
                let mut token_string = String::new();
                core::mem::swap(&mut token_string, &mut self.token);
                if let Ok(number) = token_string.parse() {
                    self.lexer_state = ParserLexerState::Default;
                    self.source.next();
                    return Ok(Some(Token::Number(number)));
                } else {
                    return Err(ParserError::NumberParseError(token_string));
                }
            }
            ParserLexerState::InDotStart => {
                self.token.clear();
                return Ok(Some(Token::Dot));
            }
            ParserLexerState::InString | ParserLexerState::InStringSlash => {
                return Err(ParserError::UnclosedString);
            }
        }
    }

    fn advance_token(&mut self) -> Result<(), ParserError> {
        self.current_token = if let Some(token) = self.next_token()? {
            token
        } else {
            Token::Void
        };
        Ok(())
    }

    fn parse_list<T: LispContext>(&mut self) -> Result<LispExp<T>, ParserError> {
        let mut list = vec![];
        while self.current_token != Token::Void {
            match self.current_token {
                Token::RParen => {
                    self.advance_token()?;
                    return Ok(LispExp::list(list));
                }
                Token::Dot => {
                    if list.is_empty() {
                        return Err(ParserError::UnexpectedDot);
                    }
                    self.advance_token()?;
                    if self.current_token == Token::RParen || self.current_token == Token::Void {
                        return Err(ParserError::UnexpectedDot);
                    }
                    let tail = self.next()?;
                    if self.current_token != Token::RParen {
                        return Err(ParserError::MalformedDottedList);
                    }
                    self.advance_token()?;
                    return Ok(LispExp::improper_list(list, tail));
                }
                _ => {
                    list.push(self.next()?);
                }
            }
        }
        Err(ParserError::UnclosedList)
    }

    fn parse_vector<T: LispContext>(&mut self) -> Result<LispExp<T>, ParserError> {
        let mut vec = vec![];

        while self.current_token != Token::Void {
            match self.current_token {
                Token::RSquared => {
                    self.advance_token()?;
                    return Ok(LispExp::vec(vec));
                }
                _ => {
                    vec.push(self.next()?);
                }
            }
        }
        Err(ParserError::UnclosedVector)
    }

    fn parse_map<T: LispContext>(&mut self) -> Result<LispExp<T>, ParserError> {
        let mut map = HashMap::new();
        let mut is_key = true;
        let mut current_key = String::new();

        while self.current_token != Token::Void {
            if is_key {
                match &self.current_token {
                    Token::RBracket => {
                        self.advance_token()?;
                        return Ok(LispExp::map(map));
                    }
                    Token::Symbol(symbol) => {
                        is_key = false;
                        current_key = symbol.clone();
                        self.advance_token()?;
                    }
                    Token::String(string) => {
                        is_key = false;
                        current_key = string.clone();
                        self.advance_token()?;
                    }
                    _ => {
                        return Err(ParserError::InvalidMapKey);
                    }
                }
            } else {
                match self.current_token {
                    Token::RBracket => {
                        return Err(ParserError::MapKeyMissingValue);
                    }
                    _ => {
                        map.insert(current_key.clone(), self.next()?);
                        is_key = true;
                    }
                }
            }
        }

        Err(ParserError::UnclosedMap)
    }

    pub fn next<T: LispContext>(&mut self) -> Result<LispExp<T>, ParserError> {
        match self.current_token.clone() {
            Token::Symbol(symbol) => {
                self.advance_token()?;
                Ok(LispExp::symbol(symbol))
            }
            Token::String(string) => {
                self.advance_token()?;
                Ok(LispExp::string(string))
            }
            Token::Number(number) => {
                self.advance_token()?;
                Ok(LispExp::number(number))
            }
            Token::LParen => {
                self.advance_token()?;
                Ok(self.parse_list()?)
            }
            Token::LSquared => {
                self.advance_token()?;
                Ok(self.parse_vector()?)
            }
            Token::LBracket => {
                self.advance_token()?;
                Ok(self.parse_map()?)
            }
            Token::Uninitialized => {
                self.advance_token()?;
                self.next()
            }
            Token::Quote => {
                self.advance_token()?;
                let quoted = self.next()?;
                return Ok(LispExp::list(vec![
                    LispExp::symbol("quote".into()),
                    form_to_data(&quoted),
                ]));
            }
            Token::BackQuote => {
                self.advance_token()?;
                return Ok(LispExp::list(vec![
                    LispExp::symbol("backquote".into()),
                    self.next()?,
                ]));
            }
            Token::Comma => {
                self.advance_token()?;
                return Ok(LispExp::list(vec![
                    LispExp::symbol("unquote".into()),
                    self.next()?,
                ]));
            }
            Token::CommaAt => {
                self.advance_token()?;
                return Ok(LispExp::list(vec![
                    LispExp::symbol("unquote-splicing".into()),
                    self.next()?,
                ]));
            }
            Token::Void => Err(ParserError::VoidExp),
            _ => unreachable!("token parse not implemented for {:?}", self.current_token),
        }
    }
}

impl<T: LispContext> Env<T> {
    pub fn new_root() -> Arc<Self> {
        Arc::new(Self {
            variables: RwLock::new(HashMap::new()),
            functions: RwLock::new(HashMap::new()),
            macros: RwLock::new(HashMap::new()),
            parent: None,
        })
    }

    pub fn new_child(parent: &Arc<Env<T>>) -> Arc<Self> {
        Arc::new(Self {
            variables: RwLock::new(HashMap::new()),
            functions: RwLock::new(HashMap::new()),
            macros: RwLock::new(HashMap::new()),
            parent: Some(parent.clone()),
        })
    }

    pub fn get_variable(&self, name: &str) -> Option<LispExp<T>> {
        if let Some(val) = self
            .variables
            .read()
            .expect("Failed to acquire read lock on env")
            .get(name)
        {
            return Some(val.clone());
        }

        if let Some(parent) = &self.parent {
            return parent.get_variable(name);
        }

        None
    }

    pub fn update_variable(&self, name: &str, val: LispExp<T>) -> bool {
        if self
            .variables
            .read()
            .expect("Failed to acquire read lock on env")
            .contains_key(name)
        {
            self.variables
                .write()
                .expect("Failed to acquire write lock on env")
                .insert(name.to_string(), val);
            return true;
        }
        if let Some(parent) = &self.parent {
            return parent.update_variable(name, val);
        }
        false
    }

    pub fn get_function(&self, name: &str) -> Option<LispExp<T>> {
        if let Some(val) = self
            .functions
            .read()
            .expect("Failed to acquire read lock on env")
            .get(name)
        {
            return Some(val.clone());
        }

        if let Some(parent) = &self.parent {
            return parent.get_function(name);
        }

        None
    }

    pub fn get_macro(&self, name: &str) -> Option<LispExp<T>> {
        if let Some(val) = self
            .macros
            .read()
            .expect("Failed to acquire read lock on env")
            .get(name)
        {
            return Some(val.clone());
        }

        if let Some(parent) = &self.parent {
            return parent.get_macro(name);
        }

        None
    }

    pub fn set_macro(&self, name: String, val: LispExp<T>) {
        let mut map = self
            .macros
            .write()
            .expect("Failed to acquire write lock on env");
        map.insert(name, val);
    }

    pub fn set_variable(&self, name: String, val: LispExp<T>) {
        let mut map = self
            .variables
            .write()
            .expect("Failed to acquire write lock on env");
        map.insert(name, val);
    }

    pub fn set_function(&self, name: String, val: LispExp<T>) {
        let mut map = self
            .functions
            .write()
            .expect("Failed to acquire write lock on env");
        map.insert(name, val);
    }
}

impl<T: LispContext> Clone for Env<T> {
    fn clone(&self) -> Self {
        unreachable!()
    }
}

impl<T: LispContext> PartialEq for Env<T> {
    fn eq(&self, _: &Env<T>) -> bool {
        unreachable!()
    }
}

impl<T: LispContext> LispExp<T> {
    pub fn symbol(value: String) -> LispExp<T> {
        LispExp::Symbol(Arc::new(value))
    }

    pub fn nil() -> LispExp<T> {
        LispExp::symbol("nil".into())
    }

    pub fn t() -> LispExp<T> {
        LispExp::symbol("t".into())
    }

    pub fn boolean(value: bool) -> LispExp<T> {
        if value { Self::t() } else { Self::nil() }
    }

    pub fn is_nil(&self) -> bool {
        match self {
            LispExp::Symbol(s) => s.as_str() == "nil",
            LispExp::List(l) => l.is_empty(),
            _ => false,
        }
    }

    pub fn is_truthy(&self) -> bool {
        !self.is_nil()
    }

    pub fn string(value: String) -> LispExp<T> {
        LispExp::String(Arc::new(value))
    }

    pub fn number(value: f64) -> LispExp<T> {
        LispExp::Number(value)
    }

    pub fn cons(car: LispExp<T>, cdr: LispExp<T>) -> LispExp<T> {
        LispExp::Cons(Arc::new(ConsCell { car, cdr }))
    }

    /// Fold a vector into a proper list, right to left.
    pub fn proper_list(items: Vec<LispExp<T>>) -> LispExp<T> {
        items.into_iter().rev().fold(LispExp::nil(), |cdr, car| LispExp::cons(car, cdr))
    }

    /// Fold a vector into a chain ending in an arbitrary tail.
    pub fn improper_list(items: Vec<LispExp<T>>, tail: LispExp<T>) -> LispExp<T> {
        items.into_iter().rev().fold(tail, |cdr, car| LispExp::cons(car, cdr))
    }

    /// Walk a cons chain, yielding each `car`. Stops at any non-`Cons`
    /// cdr, so it silently treats an improper list as its proper prefix —
    /// callers that care use `split_list` below.
    pub fn iter(&self) -> ConsIter<T> {
        ConsIter { cursor: self.clone() }
    }

    /// Collect a chain into `(elements, final_tail)`. The tail is `nil`
    /// for a proper list.
    pub fn split_list(&self) -> (Vec<LispExp<T>>, LispExp<T>) {
        let mut items = Vec::new();
        let mut cursor = self.clone();
        while let LispExp::Cons(cell) = cursor {
            items.push(cell.car.clone());
            cursor = cell.cdr.clone();
        }
        (items, cursor)
    }

    // TODO REMOVE
    pub fn list(value: Vec<LispExp<T>>) -> LispExp<T> {
        LispExp::List(Arc::new(value))
    }

    // TODO REMOVE
    pub fn dotted_list(elements: Vec<LispExp<T>>, tail: LispExp<T>) -> LispExp<T> {
        LispExp::DottedList(Arc::new(elements), Arc::new(tail))
    }

    pub fn vec(value: Vec<LispExp<T>>) -> LispExp<T> {
        LispExp::Vector(Arc::new(value))
    }

    pub fn map(value: HashMap<String, LispExp<T>>) -> LispExp<T> {
        LispExp::Map(Arc::new(value))
    }

    pub fn lambda(value: Lambda<T>) -> LispExp<T> {
        LispExp::Lambda(Arc::new(value))
    }

    pub fn fiber(value: FiberState<T>) -> LispExp<T> {
        LispExp::Fiber(SharedFiber(Arc::new(RwLock::new(value))))
    }

    /// Build a `Primitive` value from a native function pointer and an
    /// optional docstring. DOC accepts either a `&'static str` -- the
    /// common case for primitives whose documentation is a literal sitting
    /// right next to the function, which costs no allocation -- or an owned
    /// `String`, for documentation that only exists at runtime (e.g. loaded
    /// alongside a primitive from a dynamic library). Either way the
    /// resulting `Primitive` stays cheap to `Clone`: only the `Arc`'s
    /// refcount is bumped, never the string itself.
    pub fn primitive(pointer: LispPrimitive<T>, doc: Option<Cow<'static, str>>) -> LispExp<T> {
        LispExp::Primitive {
            pointer,
            doc: Arc::new(doc.unwrap_or(Cow::Borrowed("No documentation provided."))),
        }
    }
}

// ============================== Utility functions ============================

/// The inverse: reconstitute a form `eval` can dispatch on from a data
/// list. Only reached when data is evaluated — `(eval (list '+ 1 2))`,
/// and macro expansions.
pub fn data_to_form<T: LispContext>(exp: &LispExp<T>) -> Result<LispExp<T>, EvalError> {
    match exp {
        LispExp::Cons(_) => {
            let (items, tail) = exp.split_list();
            if !tail.is_nil() {
                return Err(EvalError::UnvalidFunctionCall);
            }
            let mut form = Vec::with_capacity(items.len());
            for item in &items {
                form.push(data_to_form(item)?);
            }
            Ok(LispExp::list(form))
        }
        LispExp::Vector(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                out.push(data_to_form(item)?);
            }
            Ok(LispExp::vec(out))
        }
        other => Ok(other.clone()),
    }
}

/// Binds ARGS into CALL_FRAME according to LAMBDA's parameter list:
/// required params first (one-to-one), then `&optional` params
/// (defaulting to `nil` once ARGS runs out), then -- if LAMBDA has an
/// `&rest` param -- every remaining argument collected into a single
/// list. Shared by every lambda/macro call site so the required/
/// optional/rest binding rules only need to be implemented once.
pub fn bind_lambda_args<T: LispContext>(
    lambda: &Lambda<T>,
    args: &[LispExp<T>],
    call_frame: &Arc<Env<T>>,
) -> Result<(), EvalError> {
    let min = lambda.params.len();
    let max = min + lambda.optionals.len();
    if args.len() < min || (lambda.rest.is_none() && args.len() > max) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: if args.len() < min { min } else { max },
            got: args.len(),
        });
    }

    let mut idx = 0;
    for name in &lambda.params {
        call_frame.set_variable(name.clone(), args[idx].clone());
        idx += 1;
    }
    for name in &lambda.optionals {
        let value = args.get(idx).cloned().unwrap_or_else(LispExp::nil);
        call_frame.set_variable(name.clone(), value);
        idx += 1;
    }
    if let Some(rest_name) = &lambda.rest {
        // `idx` can run past `args.len()` here -- e.g. `params.len() == 1`,
        // `optionals.len() == 1`, but only the one required argument was
        // supplied -- so it must be clamped before slicing.
        let rest_start = idx.min(args.len());
        call_frame.set_variable(
            rest_name.clone(),
            LispExp::list(args[rest_start..].to_vec()),
        );
    }
    Ok(())
}

/// Parses a `defun`/`lambda`/`defmacro` parameter list into its three
/// buckets, mirroring Emacs Lisp's own grammar: `(REQUIRED...  [&optional
/// OPTIONAL...] [&rest REST])`. `&optional` and `&rest` are markers, not
/// bindable names -- they select which bucket the symbols that follow
/// land in, and are consumed rather than returned.
fn parse_lambda_params<T: LispContext>(
    params_list: &[LispExp<T>],
) -> Result<(Vec<String>, Vec<String>, Option<String>), EvalError> {
    #[derive(PartialEq)]
    enum Mode {
        Required,
        Optional,
        Rest,
        RestDone,
    }

    let mut required = Vec::new();
    let mut optionals = Vec::new();
    let mut rest = None;
    let mut mode = Mode::Required;

    for param in params_list {
        let LispExp::Symbol(name) = param else {
            return Err(EvalError::DefunParamIsNotASymbol);
        };
        match name.as_str() {
            "&optional" => {
                if mode != Mode::Required {
                    return Err(EvalError::DefunMisplacedParamMarker);
                }
                mode = Mode::Optional;
            }
            "&rest" => {
                if mode == Mode::Rest || mode == Mode::RestDone {
                    return Err(EvalError::DefunMisplacedParamMarker);
                }
                mode = Mode::Rest;
            }
            other => match mode {
                Mode::Required => required.push(other.to_string()),
                Mode::Optional => optionals.push(other.to_string()),
                Mode::Rest => {
                    rest = Some(other.to_string());
                    mode = Mode::RestDone;
                }
                Mode::RestDone => return Err(EvalError::DefunRestMustHaveExactlyOneParam),
            },
        }
    }

    // `&rest` with nothing after it.
    if mode == Mode::Rest {
        return Err(EvalError::DefunRestMustHaveExactlyOneParam);
    }

    Ok((required, optionals, rest))
}





// ========================================================================== //
//                           +-----------------------------+
//                           |  Lisp evaluation functions  |
//                           +-----------------------------+
// ========================================================================== //


enum EvalStep<T: LispContext> {
    Done(LispExp<T>),
    TailCall(LispExp<T>, Arc<Env<T>>),
}

pub fn eval<T: LispContext>(
    exp: &LispExp<T>,
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<LispExp<T>, EvalError> {
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
) -> Result<EvalStep<T>, EvalError> {
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

        LispExp::List(list) => {
            if list.is_empty() {
                Ok(EvalStep::Done(LispExp::list(vec![])))
            } else {
                let head = &list[0];
                match head {
                    LispExp::Symbol(symbol) => {
                        eval_special_form_or_call_step(symbol, &list[1..], env.clone(), ctx)
                    }

                    LispExp::List(_) => {
                        let mut new_ast = vec![eval(head, env.clone(), ctx)?];
                        for arg in &list[1..] {
                            new_ast.push(arg.clone());
                        }
                        return Ok(EvalStep::TailCall(LispExp::list(new_ast), env.clone()));
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
        LispExp::DottedList(_, _) => todo!()
    }
}

fn eval_special_form_or_call_step<T: LispContext>(
    symbol: &str,
    args: &[LispExp<T>],
    env: Arc<Env<T>>,
    ctx: &T,
) -> Result<EvalStep<T>, EvalError> {
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

                Ok(EvalStep::Done(LispExp::list(vec![])))
            } else {
                Err(EvalError::WrongArgumentType {
                    expected: "Lambda".into(),
                    got: format!("{:?}", target_closure),
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
                let (params, optionals, rest) = if let LispExp::List(params_list) = &args[1] {
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

            let (params, optionals, rest) = if let LispExp::List(params_list) = &args[0] {
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
            if let LispExp::List(bindings) = &args[0] {
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
                if let LispExp::List(bindings) = &args[0] {
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
                let clause_list = if let LispExp::List(clause_list) = clause {
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
            let spec = if let LispExp::List(spec) = &args[0] {
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
                LispExp::List(items) => (**items).clone(),
                other => {
                    if other.is_nil() {
                        vec![]
                    } else {
                        return Err(EvalError::WrongArgumentType {
                            expected: "List".into(),
                            got: format!("{:?}", other),
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
            let spec = if let LispExp::List(spec) = &args[0] {
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
                    got: format!("{:?}", count_val),
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
                for cleanup in &args[1..] {
                    eval(cleanup, env.clone(), ctx)?;
                }
                Ok(EvalStep::Done(body_result?))
            }
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
                    if let LispExp::List(params_list) = &args[1] {
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
) -> Result<(String, Option<LispExp<T>>), EvalError> {
    match binding {
        LispExp::List(pair) if pair.len() == 2 => {
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
) -> Result<LispExp<T>, EvalError> {
    fn is_tagged<T: LispContext>(list: &[LispExp<T>], tag: &str) -> bool {
        list.len() == 2 && matches!(&list[0], LispExp::Symbol(s) if s.as_str() == tag)
    }

    match exp {
        LispExp::List(list) => {
            if is_tagged(list, "unquote") {
                return eval(&list[1], env, ctx);
            }

            let mut result = Vec::with_capacity(list.len());
            for item in list.iter() {
                if let LispExp::List(inner) = item {
                    if is_tagged(inner, "unquote-splicing") {
                        let spliced = eval(&inner[1], env.clone(), ctx)?;
                        match &spliced {
                            LispExp::Cons(_) => result.extend(spliced.iter()),
                            LispExp::List(spliced_list) => {
                                result.extend(spliced_list.iter().cloned());
                            }
                            other => {
                                if !other.is_nil() {
                                    return Err(EvalError::WrongArgumentType {
                                        expected: "List".into(),
                                        got: format!("{:?}", other),
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
) -> Result<EvalStep<T>, EvalError> {
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
) -> Result<EvalStep<T>, EvalError> {
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
