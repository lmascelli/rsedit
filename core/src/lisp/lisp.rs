use std::{
    cmp::PartialEq,
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, RwLock},
};

pub trait LispContext: Clone + PartialEq + Debug + Send + Sync + 'static {
    /// Consumes a given amount of execution ticks.
    /// Returns `Err(EvalError::OutOfFuel)` if the host-defined budget is exhausted.
    fn consume_fuel(&self, amount: u32) -> Result<(), EvalError>;

    /// Allows the VM to bubble up non-fatal diagnostic logs, trace statements,
    /// or debugging notices to the host without knowing how the host presents them.
    fn log_diagnostic(&self, msg: &str);
}

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
    Number(f64),
    String(String),
    Symbol(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lambda<T: LispContext> {
    pub params: Vec<String>,
    pub body: Vec<LispExp<T>>,
    pub env: Arc<Env<T>>,
    pub doc: Option<Arc<String>>,
}

#[derive(Clone, Debug)]
pub struct SharedAtom<T: LispContext>(pub Arc<RwLock<LispExp<T>>>);

impl<T: LispContext> PartialEq for SharedAtom<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

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

#[derive(Clone, Debug, PartialEq)]
// Primitive comparison has no meaning and will probably never done
#[allow(unpredictable_function_pointer_comparisons)]
pub enum LispExp<T: LispContext> {
    List(Arc<Vec<LispExp<T>>>),
    Vector(Arc<Vec<LispExp<T>>>),
    Map(Arc<HashMap<String, LispExp<T>>>),
    Number(f64),
    Symbol(Arc<String>),
    String(Arc<String>),
    Lambda(Arc<Lambda<T>>),
    Primitive(fn(&[LispExp<T>], Arc<Env<T>>, &T) -> Result<LispExp<T>, EvalError>),
    Atom(SharedAtom<T>),
    Fiber(SharedFiber<T>),
}

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
    LetUnvalidBindingAt(usize),
    LetUnvalidBindingList,
    LetNoBindingsProvided,
    OutOfFuel,
    RuntimeMessage(String),
}

#[derive(Debug)]
pub struct Env<T: LispContext> {
    pub variables: RwLock<HashMap<String, LispExp<T>>>,
    pub functions: RwLock<HashMap<String, LispExp<T>>>,
    pub parent: Option<Arc<Env<T>>>,
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
}

pub struct Parser<'source> {
    source: std::iter::Peekable<std::str::Chars<'source>>,
    token: String,
    current_token: Token,
    parens_stack: Vec<Token>,
    lexer_state: ParserLexerState,
}

impl<T: LispContext> LispExp<T> {
    pub fn symbol(value: String) -> LispExp<T> {
        LispExp::Symbol(Arc::new(value))
    }

    pub fn string(value: String) -> LispExp<T> {
        LispExp::String(Arc::new(value))
    }

    pub fn number(value: f64) -> LispExp<T> {
        LispExp::Number(value)
    }

    pub fn list(value: Vec<LispExp<T>>) -> LispExp<T> {
        LispExp::List(Arc::new(value))
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
                    '.' | '0'..='9' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InNumber;
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
                        return Err(ParserError::NumberInvadidChar(*c));
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
                return Ok(LispExp::list(vec![
                    LispExp::symbol("quote".into()),
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
            parent: None,
        })
    }

    pub fn new_child(parent: &Arc<Env<T>>) -> Arc<Self> {
        Arc::new(Self {
            variables: RwLock::new(HashMap::new()),
            functions: RwLock::new(HashMap::new()),
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

        LispExp::Symbol(symbol) => {
            if let Some(var) = env.get_variable(symbol) {
                Ok(EvalStep::Done(var))
            } else {
                Err(EvalError::UnboundVariable(symbol.to_string()))
            }
        }

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

                        if lambda.params.len() != evaled_args.len() {
                            return Err(EvalError::WrongNumberOfArguments {
                                expected: lambda.params.len(),
                                got: evaled_args.len(),
                            });
                        }

                        let call_frame = Env::new_child(&lambda.env);

                        for (i, param_name) in lambda.params.iter().enumerate() {
                            call_frame.set_variable(param_name.clone(), evaled_args[i].clone());
                        }

                        if lambda.body.is_empty() {
                            return Ok(EvalStep::Done(LispExp::symbol("nil".into())));
                        }

                        for arg in &lambda.body[0..lambda.body.len() - 1] {
                            eval(arg, call_frame.clone(), ctx)?;
                        }

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

        _ => todo!(),
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
                if !(condition == LispExp::list(vec![])
                    || condition == LispExp::symbol("nil".into()))
                {
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
                if cond_val == LispExp::symbol("nil".into()) || cond_val == LispExp::list(vec![]) {
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

        "defun" => {
            if args.len() < 3 {
                return Err(EvalError::DefunNotCorrectExpression);
            }
            if let LispExp::Symbol(func_name) = &args[0] {
                let mut params_vec = vec![];
                let mut body_index = 2;
                let mut doc = None;
                if let LispExp::List(params_list) = &args[1] {
                    for param in params_list.iter() {
                        if let LispExp::Symbol(param_name) = param {
                            params_vec.push((**param_name).clone());
                        } else {
                            return Err(EvalError::DefunParamIsNotASymbol);
                        }
                    }
                } else {
                    return Err(EvalError::DefunParamsAreNotAList);
                }

                if let LispExp::String(doc_string) = &args[2]
                    && args.len() > 3
                {
                    doc = Some(Arc::new(doc_string.to_string()));
                    body_index = 3;
                }

                let lambda = Lambda {
                    params: params_vec,
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

        "lambda" => {
            if args.is_empty() {
                return Err(EvalError::DefunNotCorrectExpression);
            }

            let mut params_vec = vec![];

            if let LispExp::List(params_list) = &args[0] {
                for param in params_list.iter() {
                    if let LispExp::Symbol(p_name) = param {
                        params_vec.push(p_name.to_string());
                    } else {
                        return Err(EvalError::DefunParamIsNotASymbol);
                    }
                }
            } else {
                return Err(EvalError::DefunParamsAreNotAList);
            }

            let body = args[1..].to_vec();

            Ok(EvalStep::Done(LispExp::lambda(Lambda {
                params: params_vec,
                body,
                env: env.clone(),
                doc: None,
            })))
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
                    if let LispExp::List(pair) = binding {
                        if pair.len() == 2 {
                            if let LispExp::Symbol(name) = &pair[0] {
                                let val = eval(&pair[1], env.clone(), ctx)?;
                                let_env.set_variable(name.to_string(), val);
                            } else {
                                return Err(EvalError::LetUnvalidBindingAt(i));
                            }
                        } else {
                            return Err(EvalError::LetUnvalidBindingAt(i));
                        }
                    } else {
                        return Err(EvalError::LetUnvalidBindingAt(i));
                    }
                }
            } else if args[0] != LispExp::symbol("nil".into()) {
                return Err(EvalError::LetUnvalidBindingList);
            }

            let body = &args[1..];
            if body.is_empty() {
                return Ok(EvalStep::Done(LispExp::symbol("nil".into())));
            }

            for arg in &args[0..body.len() - 1] {
                eval(arg, let_env.clone(), ctx)?;
            }

            Ok(EvalStep::TailCall(
                body.last()
                    .expect("Failed to get the last let expression")
                    .clone(),
                let_env,
            ))
        }

        _ => eval_function_call_step(symbol, args, env, ctx),
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
        if let LispExp::Lambda(lambda) = func {
            if lambda.params.len() != evaled_args.len() {
                return Err(EvalError::WrongNumberOfArguments {
                    expected: lambda.params.len(),
                    got: evaled_args.len(),
                });
            }

            let call_frame = Env::new_child(&lambda.env);

            for (i, param_name) in lambda.params.iter().enumerate() {
                call_frame.set_variable(param_name.clone(), evaled_args[i].clone());
            }

            if lambda.body.is_empty() {
                return Ok(EvalStep::Done(LispExp::symbol("nil".into())));
            }

            for arg in &lambda.body[0..lambda.body.len() - 1] {
                eval(arg, call_frame.clone(), ctx)?;
            }

            Ok(EvalStep::TailCall(
                lambda
                    .body
                    .last()
                    .expect("Failed to get the last expression in the function call")
                    .clone(),
                call_frame,
            ))
        } else if let LispExp::Primitive(function) = func {
            Ok(EvalStep::Done(function(
                &evaled_args[..],
                env.clone(),
                ctx,
            )?))
        } else {
            Err(EvalError::UncorrectFunctionDefinition)
        }
    } else {
        Err(EvalError::UndefinedFunction(symbol.into()))
    }
}
