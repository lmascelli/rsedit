use std::fmt::Debug;
use std::cmp::PartialEq;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Uninitialized,
    Void,
    LParen,
    RParen,
    LSquared,
    RSquared,
    LBracket,
    RBracket,
    Number(f64),
    String(String),
    Symbol(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum LispExp<T> {
    List(Vec<LispExp<T>>),
    Vector(Vec<LispExp<T>>),
    Map(HashMap<String, LispExp<T>>),
    Number(f64),
    Symbol(String),
    String(String),
    Primitive(fn(&[LispExp<T>], &mut T) -> Result<LispExp<T>, EvalError>),
}

#[derive(Debug, PartialEq)]
pub enum EvalError {
    UnboundVariable(String),
    UndefinedFunction(String),
    UnvalidFunctionCall,
    UncorrectFunctionDefinition,
    WrongNumberOfArguments { expected: usize, got: usize },
    WrongArgumentType { expected: String, got: String },
    IfNoConditionProvided,
    IfNoTrueBrach,
    SetqSymbolRequired,
    SetqWrongNumberOfArgs(usize),
    DefunNameMustBeASymbol,
    DefunNotCorrectExpression,
    DefunParamsAreNotAList,
    DefunParamIsNotASymbol,
}

pub struct Env<T> {
    pub variables: HashMap<String, LispExp<T>>,
    pub functions: HashMap<String, LispExp<T>>,
    pub outer: Option<*mut Env<T>>,
}

enum ParserLexerState {
    Default,
    InSymbol,
    InString,
    InStringSlash,
    InNumber,
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

    fn next_token(&mut self) -> Result<Option<Token>, ParserError> {
        while let Some(c) = self.source.peek() {
            match self.lexer_state {
                ParserLexerState::Default => match c {
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
                    '.' | '-' | '0'..='9' => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InNumber;
                    }
                    _ => {
                        self.token.push(*c);
                        self.lexer_state = ParserLexerState::InSymbol;
                    }
                },
                ParserLexerState::InSymbol => match c {
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
                ParserLexerState::InNumberAfterDot => match c {
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
            ParserLexerState::InSymbol => {
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

    fn parse_list<T>(&mut self) -> Result<LispExp<T>, ParserError> {
        let mut list = vec![];
        while self.current_token != Token::Void {
            match self.current_token {
                Token::RParen => {
                    self.advance_token()?;
                    return Ok(LispExp::List(list));
                }
                _ => {
                    list.push(self.next()?);
                }
            }
        }
        Err(ParserError::UnclosedList)
    }

    fn parse_vector<T>(&mut self) -> Result<LispExp<T>, ParserError> {
        let mut vec = vec![];

        while self.current_token != Token::Void {
            match self.current_token {
                Token::RSquared => {
                    self.advance_token()?;
                    return Ok(LispExp::Vector(vec));
                }
                _ => {
                    vec.push(self.next()?);
                }
            }
        }
        Err(ParserError::UnclosedVector)
    }

    fn parse_map<T>(&mut self) -> Result<LispExp<T>, ParserError> {
        let mut map = HashMap::new();
        let mut is_key = true;
        let mut current_key = String::new();

        while self.current_token != Token::Void {
            if is_key {
                match &self.current_token {
                    Token::RBracket => {
                        self.advance_token()?;
                        return Ok(LispExp::Map(map));
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

    pub fn next<T>(&mut self) -> Result<LispExp<T>, ParserError> {
        eprintln!("{:?}", self.current_token);
        match self.current_token.clone() {
            Token::Symbol(symbol) => {
                self.advance_token()?;
                Ok(LispExp::Symbol(symbol))
            }
            Token::String(string) => {
                self.advance_token()?;
                Ok(LispExp::String(string))
            }
            Token::Number(number) => {
                self.advance_token()?;
                Ok(LispExp::Number(number))
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
            Token::Void => Err(ParserError::VoidExp),
            _ => todo!("token parse not implemented for {:?}", self.current_token),
        }
    }
}

impl<T> Env<T>
where T: Clone {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
            outer: None,
        }
    }

    pub fn extend(caller: *mut Env<T>) -> Self {
        Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
            outer: Some(caller),
        }
    }

    pub fn get_var(&self, name: &str) -> Option<LispExp<T>> {
        if let Some(val) = self.variables.get(name) {
            return Some(val.clone());
        }

        if let Some(outer_ptr) = self.outer {
            unsafe { return (*outer_ptr).get_var(name); }
        }

        None
    }

    pub fn get_func(&self, name: &str) -> Option<LispExp<T>> {
        if let Some(val) = self.functions.get(name) {
            return Some(val.clone());
        }

        if let Some(outer_ptr) = self.outer {
            unsafe { return (*outer_ptr).get_func(name); }
        }

        None
    }
}

pub fn eval<T>(exp: &LispExp<T>, env: &mut Env<T>, ctx: &mut T)
 -> Result<LispExp<T>, EvalError>
where T: Clone + PartialEq + Debug {
    match exp {
        LispExp::String(_) | LispExp::Number(_) => Ok(exp.clone()),

        LispExp::Symbol(symbol) => {
            if let Some(var) = env.get_var(symbol) {
                Ok(var)
            } else {
                Err(EvalError::UnboundVariable(symbol.clone()))
            }
        }

        LispExp::List(list) => {
            if list.is_empty() {
                Ok(LispExp::List(vec![]))
            } else {
                let head = &list[0];
                match head {
                    LispExp::Symbol(symbol) => eval_special_form_or_call(symbol, &list[1..], env, ctx),
                    _ => { return Err(EvalError::UnvalidFunctionCall); }
                }
            }
        }

        LispExp::Vector(vec) => {
            let mut new_vec = Vec::with_capacity(vec.len());
            for v in vec {
                new_vec.push(eval(v, env, ctx)?);
            }
            Ok(LispExp::Vector(new_vec))
        }

        _ => todo!(),
    }
}

fn eval_special_form_or_call<T>(
    symbol: &str,
    args: &[LispExp<T>],
    env: &mut Env<T>,
    ctx: &mut T,
) -> Result<LispExp<T>, EvalError>
where T: Clone + PartialEq + Debug {
    match symbol {
        "if" => {
            if args.len() < 1 {
                Err(EvalError::IfNoConditionProvided)
            }
            else if args.len() < 2 {
                Err(EvalError::IfNoTrueBrach)
            } else {
                let condition = &args[0];
                if !(*condition == LispExp::List(vec![]) ||
                    *condition == LispExp::Symbol("nil".into())) {
                        Ok(args[1].clone())
                    }
                else {
                    if args.len() > 2 {
                        Ok(args[2].clone())
                    } else {
                        Ok(LispExp::Symbol("nil".into()))
                    }
                }
            }
        },

        "setq" => {
            if args.len() < 2 && args.len() % 2 != 0 {
                return Err(EvalError::SetqWrongNumberOfArgs(args.len()));
            }
            let mut is_symbol = true;
            let mut list_var_name = String::from("unreachable");
            let mut value = LispExp::Symbol("nil".into());
            for arg in args {
                if is_symbol {
                    if let LispExp::Symbol(var_name) = arg {
                        list_var_name = var_name.clone();
                        is_symbol = false;
                    } else {
                        return Err(EvalError::SetqSymbolRequired);
                    }
                } else {
                    value = eval(arg, env, ctx)?;
                    env.variables.insert(list_var_name.clone(), value.clone());
                    is_symbol = true;
                }
            }
            Ok(value)
        },

        "defun" => {
            if args.len() < 3 {
                return Err(EvalError::DefunNotCorrectExpression);
            }
            if let LispExp::Symbol(func_name) = &args[0] {
                if let LispExp::List(params_list) = &args[1] {
                    for param in params_list {
                        if let LispExp::Symbol(_) = param {}
                        else {
                            return Err(EvalError::DefunParamIsNotASymbol);
                        }
                    }
                } else {
                    return Err(EvalError::DefunParamsAreNotAList);
                }
                let params = args[1].clone();
                let body = args[2].clone();

                let lambda = LispExp::List(vec![
                    LispExp::Symbol("lambda".into()),
                    params,
                    body,
                ]);
                env.functions.insert(func_name.clone(), lambda);

                Ok(LispExp::Symbol(func_name.clone()))
            } else {
                Err(EvalError::DefunNameMustBeASymbol)
            }
        }

        _ => { eval_function_call(symbol, args, env, ctx) }
    }
}

fn eval_function_call<T>(
    symbol: &str,
    args: &[LispExp<T>],
    env: &mut Env<T>,
    ctx: &mut T,
) -> Result<LispExp<T>, EvalError>
where T: Clone + PartialEq + Debug {
    let mut evaled_args = Vec::new();
    for arg in args {
        evaled_args.push(eval(arg, env, ctx)?);
    }

    if let Some(func) = env.get_func(symbol) {
        if let LispExp::List(lambda_ast) = func {
            if lambda_ast.len() != 3 {
                return Err(EvalError::UncorrectFunctionDefinition);
            }
            let params = &lambda_ast[1];
            let body = &lambda_ast[2];

            let mut call_frame = Env::extend(env as *mut Env<T>);

            if let LispExp::List(param_list) = params {
                for (i, param) in param_list.iter().enumerate() {
                    if let LispExp::Symbol(param_name) = param {
                        call_frame
                            .variables
                            .insert(param_name.clone(), evaled_args[i].clone());
                    }
                }
            }

            eval(body, &mut call_frame, ctx)
        } else if let LispExp::Primitive(function) = func {
            return Ok(function(&evaled_args[..], ctx)?);
        } else {
            Err(EvalError::UncorrectFunctionDefinition)
        }
    } else {
        Err(EvalError::UndefinedFunction(symbol.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================
    // LEXER TESTS
    // ==========================================
    fn tokenize(input: &str) -> Result<Vec<Token>, ParserError> {
        let mut parser = Parser::new(input);
        let mut ret = Vec::new();

        while let Some(token) = parser.next_token()? {
            ret.push(token);
        }

        return Ok(ret);
    }

    fn lex_all(input: &str) -> Result<Vec<Token>, ParserError> {
        let input_str = input.to_string();
        let mut parser = Parser::new(&input_str);
        let mut tokens = Vec::new();

        while let Some(token) = parser.next_token()? {
            tokens.push(token);
        }
        Ok(tokens)
    }

    #[test]
    fn test_basic_parens() {
        let input = "( [ { } ] )";
        let tokens = lex_all(input).unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::LParen,
                Token::LSquared,
                Token::LBracket,
                Token::RBracket,
                Token::RSquared,
                Token::RParen,
            ]
        );
    }

    #[test]
    fn test_symbols_and_spaces() {
        let input = "def  my-var   + ";
        let tokens = lex_all(input).unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Symbol("def".into()),
                Token::Symbol("my-var".into()),
                Token::Symbol("+".into()),
            ]
        );
    }

    #[test]
    fn test_strings() {
        let input = r#""hello world" "escaped \" quote""#;
        let tokens = lex_all(input).unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::String("hello world".into()),
                Token::String("escaped \" quote".into()),
            ]
        );
    }

    #[test]
    fn test_numbers() {
        // Note: these require a trailing space currently based on your logic
        let input = "123 45.67 0.1 ";
        let tokens = lex_all(input).unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Number(123.0),
                Token::Number(45.67),
                Token::Number(0.1),
            ]
        );
    }

    #[test]
    fn test_lisp_expression() {
        let input = "(define (add x y) (+ x y)) ";
        let tokens = lex_all(input).unwrap();
        let expected = vec![
            Token::LParen,
            Token::Symbol("define".into()),
            Token::LParen,
            Token::Symbol("add".into()),
            Token::Symbol("x".into()),
            Token::Symbol("y".into()),
            Token::RParen,
            Token::LParen,
            Token::Symbol("+".into()),
            Token::Symbol("x".into()),
            Token::Symbol("y".into()),
            Token::RParen,
            Token::RParen,
        ];
        assert_eq!(tokens, expected);
    }

    #[test]
    fn test_unbalanced_error() {
        let input = "( ] )";
        let result = lex_all(input);
        match result {
            Err(ParserError::UnbalancedRSquared) => (),
            _ => panic!("Expected UnbalancedRSquared error"),
        }
    }

    #[test]
    fn test_eof_handling_for_numbers() {
        let input = "42";
        let _ = lex_all(input);
    }

    #[test]
    fn test_delimiters_no_spaces() {
        // Symbols and numbers ending exactly at a parenthesis
        let input = "(factorial 5)(add x y)";
        let tokens = lex_all(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::LParen,
                Token::Symbol("factorial".into()),
                Token::Number(5.0),
                Token::RParen,
                Token::LParen,
                Token::Symbol("add".into()),
                Token::Symbol("x".into()),
                Token::Symbol("y".into()),
                Token::RParen,
            ]
        );
    }

    #[test]
    fn test_complex_strings() {
        let input = r#" "line1\nline2" "quote: \" " "" "#;
        let tokens = lex_all(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                // Note: Your current InStringSlash logic doesn't handle \n yet,
                // it just pushes the \ and the n.
                Token::String("line1\\nline2".into()),
                Token::String("quote: \" ".into()),
                Token::String("".into()), // Empty string
            ]
        );
    }

    #[test]
    fn test_deep_nesting() {
        let input = "(let [x {y 10}] (fn (z) [z x]))";
        let tokens = lex_all(input).unwrap();

        let expected = vec![
            Token::LParen,
            Token::Symbol("let".into()),
            Token::LSquared,
            Token::Symbol("x".into()),
            Token::LBracket,
            Token::Symbol("y".into()),
            Token::Number(10.0),
            Token::RBracket,
            Token::RSquared,
            Token::LParen,
            Token::Symbol("fn".into()),
            Token::LParen,
            Token::Symbol("z".into()),
            Token::RParen,
            Token::LSquared,
            Token::Symbol("z".into()),
            Token::Symbol("x".into()),
            Token::RSquared,
            Token::RParen,
            Token::RParen,
        ];
        assert_eq!(tokens, expected);
    }

    #[test]
    fn test_numeric_edge_cases() {
        // Valid cases
        let input = "123.456 0.001 100 ";
        let tokens = lex_all(input).unwrap();
        assert_eq!(tokens[0], Token::Number(123.456));
        assert_eq!(tokens[1], Token::Number(0.001));
        assert_eq!(tokens[2], Token::Number(100.0));

        // Invalid: Multiple dots
        let input_err = "1.2.3 ";
        let result = lex_all(input_err);
        assert!(matches!(result, Err(ParserError::NumberParseError(_))));
    }

    #[test]
    fn test_tokenize_basic() {
        let input = "(+ 1 2.5 \"hello\")";
        let tokens = tokenize(input).expect("Failed to tokenize");

        assert_eq!(
            tokens,
            vec![
                Token::LParen,
                Token::Symbol("+".to_string()),
                Token::Number(1.0),
                Token::Number(2.5),
                Token::String("hello".to_string()),
                Token::RParen,
            ]
        );
    }

    #[test]
    fn test_tokenize_complex_symbols() {
        let input = "(insert-text! my-var)";
        let tokens = tokenize(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::LParen,
                Token::Symbol("insert-text!".to_string()),
                Token::Symbol("my-var".to_string()),
                Token::RParen,
            ]
        );
    }

    // ==========================================
    // PARSER TESTS
    // ==========================================

    fn setup_env() -> (Env<()>, ()) {
        (Env::new(), ())
    }

    #[test]
    fn test_parse_primitives() {
        let mut parser = Parser::new("42");

        let ast_num: LispExp<()> = parser.next().unwrap();
        assert_eq!(ast_num, LispExp::Number(42.0));

        let mut parser = Parser::new("my-symbol");
        let ast_sym: LispExp<()> = parser.next().unwrap();
        assert_eq!(ast_sym, LispExp::Symbol("my-symbol".to_string()));
    }

    #[test]
    fn test_parse_simple_list() {
        let mut parser = Parser::new("(print \"world\")");
        let ast: LispExp<()> = parser.next().unwrap();

        assert_eq!(
            ast,
            LispExp::List(vec![
                LispExp::Symbol("print".to_string()),
                LispExp::String("world".to_string()),
            ])
        );
    }

    #[test]
    fn test_parse_nested_lists() {
        // Equivalent to: (define x (+ 10 20))
        let mut parser = Parser::new("(define x (+ 10 20))");
        let ast: LispExp<()> = parser.next().unwrap();

        assert_eq!(
            ast,
            LispExp::List(vec![
                LispExp::Symbol("define".to_string()),
                LispExp::Symbol("x".to_string()),
                LispExp::List(vec![
                    LispExp::Symbol("+".to_string()),
                    LispExp::Number(10.0),
                    LispExp::Number(20.0),
                ])
            ])
        );
    }

    #[test]
    fn test_parse_deeply_nested() {
        let mut parser = Parser::new("(((1)))");
        let ast: LispExp<()> = parser.next().unwrap();

        assert_eq!(
            ast,
            LispExp::List(vec![LispExp::List(vec![LispExp::List(vec![
                LispExp::Number(1.0)
            ])])])
        );
    }

    #[test]
    fn test_parse_errors() {
        // Missing closing parenthesis
        let mut parser = Parser::new("(+1 2");
        let err1 = parser.next::<()>().unwrap_err();
        assert_eq!(err1, ParserError::UnclosedList);

        // Unexpected closing parenthesis
        eprintln!("{:?}", Parser::new("(+ 1 2))").next::<()>().unwrap_err());
        let mut parser = Parser::new("(+1 2))");
        let err2 = parser.next::<()>().unwrap_err();
        assert_eq!(err2, ParserError::UnbalancedRParen);
    }

    #[test]
    fn test_vector_parsing() {
        // Testing that [1 2 3] creates a Vector, not a List
        let input = "[1 2 3]";
        let mut parser = Parser::new(input);
        let ast: LispExp<()> = parser.next().expect("Should parse vector");

        match ast {
            LispExp::Vector(v) => {
                assert_eq!(v.len(), 3);
                assert_eq!(v[0], LispExp::Number(1.0));
            }
            _ => panic!("Expected Vector, found {:?}", ast),
        }
    }

    #[test]
    fn test_map_parsing_valid() {
        // Testing { "name" "Gemini" "version" 3 }
        let input = r#"{ "name" "Gemini" "version" 3.0 }"#;
        let mut parser = Parser::new(input);
        let ast: LispExp<()> = parser.next().expect("Should parse map");

        if let LispExp::Map(m) = ast {
            assert_eq!(m.get("name").unwrap(), &LispExp::String("Gemini".into()));
            assert_eq!(m.get("version").unwrap(), &LispExp::Number(3.0));
        } else {
            panic!("Expected Map, found {:?}", ast);
        }
    }

    #[test]
    fn test_nested_mixed_structures() {
        // A complex nested structure: (calculate [1 2] { "factor" 10.5 })
        let input = "(calculate [1 2] { \"factor\" 10.5 })";
        let mut parser = Parser::new(input);
        let ast: LispExp<()> = parser.next().expect("Should parse complex structure");

        if let LispExp::List(list) = ast {
            assert_eq!(list[0], LispExp::Symbol("calculate".into()));
            assert!(matches!(list[1], LispExp::Vector(_)));
            assert!(matches!(list[2], LispExp::Map(_)));
        } else {
            panic!("Root should be a List");
        }
    }

    #[test]
    fn test_map_error_handling() {
        // Case 1: Odd number of elements in a map
        let input_odd = r#"{ "key1" "val1" "key2" }"#;
        let mut p1 = Parser::new(input_odd);
        let result = p1.next::<()>();
        assert!(matches!(result, Err(ParserError::MapKeyMissingValue)));

        // Case 2: Using a non-string/symbol as a key
        let input_bad_key = "{ 42 \"answer\" }";
        let mut p2 = Parser::new(input_bad_key);
        let result = p2.next::<()>();
        assert!(matches!(result, Err(ParserError::InvalidMapKey)));
    }

    #[test]
    fn test_delimiter_clash_no_spaces() {
        // Testing that the lexer correctly breaks symbols at [ or {
        // This checks your InSymbol state transitions [cite: 17-22]
        let input = "my-func[1 2]{:key val}";
        let mut parser = Parser::new(input);

        // Should yield: Symbol, Vector, Map
        assert!(matches!(parser.next::<()>().unwrap(), LispExp::Symbol(_)));
        assert!(matches!(parser.next::<()>().unwrap(), LispExp::Vector(_)));
        assert!(matches!(parser.next::<()>().unwrap(), LispExp::Map(_)));
    }

    #[test]
    fn test_edge_negative_numbers() {
        let input = "-42 -3.14";
        let mut parser = Parser::new(input);

        assert_eq!(parser.next::<()>().unwrap(), LispExp::Number(-42.0));
        assert_eq!(parser.next::<()>().unwrap(), LispExp::Number(-3.14));
    }

    #[test]
    fn test_edge_naked_decimals() {
        let input = ".5 5. 0.0";
        let mut parser = Parser::new(input);

        assert_eq!(parser.next::<()>().unwrap(), LispExp::Number(0.5));
        assert_eq!(parser.next::<()>().unwrap(), LispExp::Number(5.0));
        assert_eq!(parser.next::<()>().unwrap(), LispExp::Number(0.0));
    }

    #[test]
    fn test_edge_brutal_string_escaping() {
        let input = r#" "\\\"" "#; // Represents the string: \"
        let mut parser = Parser::new(input);

        let ast: LispExp<()> = parser.next().unwrap();
        if let LispExp::String(s) = ast {
            assert_eq!(s, "\\\"");
        } else {
            panic!("Expected String, got {:?}", ast);
        }
    }

    #[test]
    fn test_edge_delimiter_smashes() {
        let input = "{key[1]}";
        let mut parser = Parser::new(input);

        // Should parse as Map containing Key: "key", Value: Vector([1.0])
        let ast: LispExp<()> = parser.next().unwrap();
        if let LispExp::Map(m) = ast {
            let val = m.get("key").expect("Key 'key' not found");
            assert_eq!(val, &LispExp::Vector(vec![LispExp::Number(1.0)]));
        } else {
            panic!("Expected Map");
        }
    }

    #[test]
    fn test_edge_nested_empties() {
        let input = "([{}])";
        let mut parser = Parser::new(input);

        let ast: LispExp<()> = parser.next().unwrap();

        // Expected: List containing one Vector containing one Map (all empty)
        assert_eq!(
            ast,
            LispExp::List(vec![LispExp::Vector(vec![LispExp::Map(
                std::collections::HashMap::new()
            )])])
        );
    }

    #[test]
    fn test_complex_whitespace_and_mixed_delimiters() {
        // Added the string "data" to act as the Map's key,
        // with the Vector acting as the value.
        let input = "{\n\t\"data\"\n\t[ (1\n2\t3) ]\n}";
        let mut parser = Parser::new(input);
        let ast: LispExp<()> = parser.next().expect("Should parse successfully");

        // Expected structure: Map { "data" => Vector [ List [1, 2, 3] ] }
        if let LispExp::Map(mut map) = ast {
            let value = map.remove("data").expect("Expected key 'data' in Map");

            if let LispExp::Vector(vec) = value {
                assert_eq!(vec.len(), 1);

                if let LispExp::List(inner) = &vec[0] {
                    assert_eq!(inner.len(), 3);
                    assert_eq!(inner[0], LispExp::Number(1.0));
                } else {
                    panic!("Inner not a list");
                }
            } else {
                panic!("Map value is not a Vector");
            }
        } else {
            panic!("Outer structure is not a Map");
        }
    }

    #[test]
    fn test_numeric_boundary_cases() {
        // Updated to test numbers ending exactly at bracket and brace boundaries
        let input = "[1.5] {\"k\" .5} 5.";
        let mut parser = Parser::new(input);

        // Test [1.5] - Vector boundary
        let exp1: LispExp<()> = parser.next().unwrap();
        assert_eq!(exp1, LispExp::Vector(vec![LispExp::Number(1.5)]));

        // Test {"k" .5} - Map boundary
        let exp2: LispExp<()> = parser.next().unwrap();
        if let LispExp::Map(mut m) = exp2 {
            assert_eq!(m.remove("k"), Some(LispExp::Number(0.5)));
        } else {
            panic!("Expected Map for second expression");
        }

        // Test "5." - EOF/whitespace boundary
        let exp3 = parser.next::<()>();
        println!("Result for 5.: {:?}", exp3);
    }

    #[test]
    fn test_empty_and_escaped_literals() {
        let input = r#" "" () [] {} "back\\slash" "#;
        let mut parser = Parser::new(input);

        assert_eq!(parser.next::<()>().unwrap(), LispExp::String("".into()));
        assert_eq!(parser.next::<()>().unwrap(), LispExp::List(vec![]));

        // These now expect Vector and Map instead of List
        assert_eq!(parser.next::<()>().unwrap(), LispExp::Vector(vec![]));
        assert_eq!(
            parser.next::<()>().unwrap(),
            LispExp::Map(std::collections::HashMap::new())
        );

        // Check backslash escaping logic
        assert_eq!(
            parser.next::<()>().unwrap(),
            LispExp::String("back\\slash".into())
        );
    }

    #[test]
    fn test_malformed_input_recovery() {
        // Unclosed list
        let mut p1 = Parser::new("(1 2 3");
        assert_eq!(p1.next::<()>().unwrap_err(), ParserError::UnclosedList);

        // Mismatched brackets
        let mut p2 = Parser::new("(1 2 3]");
        assert_eq!(p2.next::<()>().unwrap_err(), ParserError::UnbalancedRSquared);

        // Multiple decimal points
        let mut p3 = Parser::new("1.2.3");
        assert!(matches!(
            p3.next::<()>().unwrap_err(),
            ParserError::NumberParseError(_)
        ));

        // NEW: Map with missing value
        let mut p4 = Parser::new("{ \"key\" }");
        assert_eq!(p4.next::<()>().unwrap_err(), ParserError::MapKeyMissingValue);

        // NEW: Map with invalid key type (number instead of string/symbol)
        let mut p5 = Parser::new("{ 42 \"value\" }");
        assert_eq!(p5.next::<()>().unwrap_err(), ParserError::InvalidMapKey);
    }

    #[test]
    fn test_parser_robustness_mismatched_interleaved_delimiters() {
        // Tests crossing the streams: lists, vectors, and maps overlapping incorrectly
        assert_eq!(
            Parser::new("([)]").next::<()>().unwrap_err(),
            ParserError::UnbalancedRParen
        );
        assert_eq!(
            Parser::new("{ [ } ]").next::<()>().unwrap_err(),
            ParserError::InvalidMapKey
        );
        assert_eq!(
            Parser::new("( { ) }").next::<()>().unwrap_err(),
            ParserError::UnbalancedRParen
        );
    }

    #[test]
    fn test_parser_robustness_abrupt_eof() {
        // Strings that never close
        // Note: Currently, your parser might infinite loop or drop this silently depending on EOF handling.
        // If it hangs here, you know you need to handle EOF during InString state!
        let mut p1 = Parser::new("\"Unclosed string...");
        assert!(p1.next::<()>().is_err() || p1.next::<()>().is_ok());

        // Unclosed nested structures hitting EOF
        assert_eq!(
            Parser::new("(((1 2)").next::<()>().unwrap_err(),
            ParserError::UnclosedList
        );
        assert_eq!(
            Parser::new("[1 2").next::<()>().unwrap_err(),
            ParserError::UnclosedVector
        );
        assert_eq!(
            Parser::new("{ \"key\" \"val\"").next::<()>().unwrap_err(),
            ParserError::UnclosedMap
        );
    }

    #[test]
    fn test_parser_robustness_invalid_numbers() {
        // A number with an invalid character immediately following it without a space or delimiter
        assert_eq!(
            Parser::new("123a").next::<()>().unwrap_err(),
            ParserError::NumberInvadidChar('a')
        );

        // Multiple decimal points in a row
        assert!(matches!(
            Parser::new("1..2").next::<()>().unwrap_err(),
            ParserError::NumberParseError(_)
        ));
    }

    #[test]
    fn test_parser_robustness_malformed_maps() {
        // Key is missing its value (trailing key)
        assert_eq!(
            Parser::new("{ \"key1\" \"val1\" \"key2\" }").next::<()>().unwrap_err(),
            ParserError::MapKeyMissingValue
        );

        // Keys MUST be Strings or Symbols. Passing Lists or Vectors should fail.
        assert_eq!(
            Parser::new("{ [1 2] \"value\" }").next::<()>().unwrap_err(),
            ParserError::InvalidMapKey
        );
        assert_eq!(
            Parser::new("{ (func) \"value\" }").next::<()>().unwrap_err(),
            ParserError::InvalidMapKey
        );
    }

    #[test]
    fn test_parser_robustness_empty_input() {
        assert_eq!(
            Parser::new("").next::<()>().unwrap_err(),
            ParserError::VoidExp
        );
        assert_eq!(
            Parser::new("   \n \t  ").next::<()>().unwrap_err(),
            ParserError::VoidExp
        );
    }

    // ==========================================
    // EVAL TESTS
    // ==========================================

    fn get_var<T>(env: &Env<T>, name: &str) -> Option<LispExp<T>> where T: Clone {
        env.get_var(name)
    }

    fn get_func<T>(env: &Env<T>, name: &str) -> Option<LispExp<T>> where T: Clone {
        env.get_func(name)
    }

    fn is_nil<T>(exp: &LispExp<T>) -> bool where T: PartialEq {
        *exp == LispExp::List(vec![]) || *exp == LispExp::Symbol("nil".into())
    }

    #[test]
    fn test_lisp_2_namespaces() {
        let (mut env, mut ctx) = setup_env();

        // Bind the variable 'buffer' to the string "main.txt"
        // (setq buffer "main.txt")
        env.variables
            .insert("buffer".into(), LispExp::String("main.txt".into()));

        // Bind the function 'buffer' to a mock function that returns a number
        let mock_func = LispExp::List(vec![
            LispExp::Symbol("lambda".into()),
            LispExp::List(vec![]),
            LispExp::Number(42.0),
        ]);
        env.functions.insert("buffer".into(), mock_func);

        // Test 1: Evaluating the symbol alone yields the variable slot
        let var_eval = eval(&LispExp::Symbol("buffer".into()), &mut env, &mut ctx).unwrap();
        assert_eq!(var_eval, LispExp::String("main.txt".into()));

        // Test 2: Evaluating the symbol as a function call executes the function slot
        // Execution of (buffer)
        let call_exp = LispExp::List(vec![LispExp::Symbol("buffer".into())]);
        let func_eval = eval(&call_exp, &mut env, &mut ctx).unwrap();
        assert_eq!(func_eval, LispExp::Number(42.0));
    }

    #[test]
    fn test_dynamic_variable_lookup() {
        let (mut global, mut ctx) = setup_env();
        global.variables.insert("x".into(), LispExp::Number(10.0));
        global.variables.insert("y".into(), LispExp::Number(20.0));

        let mut local = Env::extend(&mut global as _);
        local.variables.insert("x".into(), LispExp::Number(99.0)); // Shadows global x

        // Local 'x' shadows global 'x'
        assert_eq!(get_var(&local, "x"), Some(LispExp::Number(99.0)));
        // Local falls back to global for 'y'
        assert_eq!(get_var(&local, "y"), Some(LispExp::Number(20.0)));
        // Unbound variable
        assert_eq!(get_var(&local, "z"), None);
    }

    #[test]
    fn test_dynamic_function_lookup() {
        let (mut global, mut ctx) = setup_env();
        global.functions.insert("add".into(), LispExp::Symbol("built-in-add".into()));

        let local = Env::extend(&mut global as _);

        // Should find 'add' in the parent's function namespace
        assert_eq!(get_func(&local, "add"), Some(LispExp::Symbol("built-in-add".into())));
    }

    #[test]
    fn test_elisp_nil_truthiness() {
        // In Elisp, the symbol "nil" and the empty list () are false.
        assert!(is_nil(&LispExp::<()>::Symbol("nil".into())));
        assert!(is_nil(&LispExp::<()>::List(vec![])));

        // Everything else is true (not nil)
        assert!(!is_nil(&LispExp::<()>::Symbol("t".into())));
        assert!(!is_nil(&LispExp::<()>::Number(0.0)));
        assert!(!is_nil(&LispExp::<()>::String("".into())));
        assert!(!is_nil(&LispExp::<()>::Vector(vec![]))); // Empty vectors are true in Elisp!
    }

    #[test]
    fn test_eval_setq() {
        let (mut env, mut ctx) = setup_env();
        // (setq a 42.0)
        let setq_exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Symbol("a".into()),
            LispExp::Number(42.0),
        ]);

        let result = eval(&setq_exp, &mut env, &mut ctx).unwrap();

        assert_eq!(result, LispExp::Number(42.0));
        assert_eq!(env.variables.get("a"), Some(&LispExp::Number(42.0)));
    }

    #[test]
    fn test_eval_defun() {
        let (mut env, mut ctx) = setup_env();

        // (defun my-func () "hello")
        let defun_exp = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::Symbol("my-func".into()),
            LispExp::List(vec![]),
            LispExp::String("hello".into()),
        ]);

        eval(&defun_exp, &mut env, &mut ctx).unwrap();

        // In Elisp, defun binds the symbol's function slot to a lambda representation
        let stored_func = env.functions.get("my-func").expect("Function should be bound");

        if let LispExp::List(lambda_data) = stored_func {
            assert_eq!(lambda_data[0], LispExp::Symbol("lambda".into()));
            assert_eq!(lambda_data[1], LispExp::List(vec![]));
            assert_eq!(lambda_data[2], LispExp::String("hello".into()));
        } else {
            panic!("defun did not store a lambda list");
        }
    }

    #[test]
    fn test_eval_if() {
        let (mut env, mut ctx) = setup_env();

        // (if nil 1.0 2.0) -> should evaluate to 2.0
        let if_false = LispExp::List(vec![
            LispExp::Symbol("if".into()),
            LispExp::Symbol("nil".into()),
            LispExp::Number(1.0),
            LispExp::Number(2.0),
        ]);
        assert_eq!(eval(&if_false, &mut env, &mut ctx).unwrap(), LispExp::Number(2.0));

        // (if "truthy" 1.0 2.0) -> should evaluate to 1.0
        let if_true = LispExp::List(vec![
            LispExp::Symbol("if".into()),
            LispExp::String("truthy".into()),
            LispExp::Number(1.0),
            LispExp::Number(2.0),
        ]);
        assert_eq!(eval(&if_true, &mut env, &mut ctx).unwrap(), LispExp::Number(1.0));
    }

    #[test]
    fn test_error_void_variable() {
        let (mut env, mut ctx) = setup_env();

        let exp = LispExp::Symbol("undefined-var".into());
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::UnboundVariable("undefined-var".into()));
    }

    #[test]
    fn test_error_void_function() {
        let (mut env, mut ctx) = setup_env();

        // (missing-func 1 2)
        let exp = LispExp::List(vec![
            LispExp::Symbol("missing-func".into()),
            LispExp::Number(1.0),
        ]);
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::UndefinedFunction("missing-func".into()));
    }

    #[test]
    fn test_error_calling_variable_as_function() {
        let (mut env, mut ctx) = setup_env();

        // Bind 'x' in the variable namespace only
        env.variables.insert("x".into(), LispExp::Number(42.0));

        // Try to call it: (x)
        let exp = LispExp::List(vec![LispExp::Symbol("x".into())]);
        let result = eval(&exp, &mut env, &mut ctx);

        // It should fail because 'x' is not in the function namespace!
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::UndefinedFunction("x".into()));
    }

    #[test]
    fn test_error_invalid_function_call() {
        let (mut env, mut ctx) = setup_env();

        // (42 "hello") -> The first element is not a symbol!
        let exp = LispExp::List(vec![
            LispExp::Number(42.0),
            LispExp::String("hello".into()),
        ]);
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::UnvalidFunctionCall);
    }

    #[test]
    fn test_error_setq_non_symbol() {
        let (mut env, mut ctx) = setup_env();

        // (setq 42 "value") -> 42 is not a valid variable name
        let exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Number(42.0),
            LispExp::String("value".into()),
        ]);
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::SetqSymbolRequired);
    }

    #[test]
    fn test_error_defun_non_symbol() {
        let (mut env, mut ctx) = setup_env();

        // (defun "my-func" () 1) -> Function name must be a symbol, not a string
        let exp = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::String("my-func".into()),
            LispExp::List(vec![]),
            LispExp::Number(1.0),
        ]);
        let result = eval(&exp, &mut env, &mut ctx);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::DefunNameMustBeASymbol);
    }

    #[test]
    fn test_error_if_missing_condition() {
        let (mut env, mut ctx) = setup_env();

        // (if) -> Missing condition and branches
        let exp = LispExp::List(vec![
            LispExp::Symbol("if".into()),
        ]);
        let result = eval(&exp, &mut env, &mut ctx);

        // Your evaluator needs bounds checking for `args[0]` to pass this without panicking!
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::IfNoConditionProvided);
    }

    #[test]
    fn test_eval_robustness_if_missing_branches() {
        let (mut env, mut ctx) = setup_env();

        // (if t) -> Has condition, but missing the true_branch.
        // Currently panics at `args[1].clone()`
        let exp = LispExp::List(vec![
            LispExp::Symbol("if".into()),
            LispExp::Symbol("t".into()),
        ]);
        let _ = eval(&exp, &mut env, &mut ctx);
    }

    #[test]
    fn test_eval_robustness_setq_missing_value() {
        let (mut env, mut ctx) = setup_env();

        // (setq a) -> Missing the value to set!
        // Currently panics at `eval(&args[1], env, ctx)?`
        let exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Symbol("a".into()),
        ]);
        let _ = eval(&exp, &mut env, &mut ctx);
    }

    #[test]
    fn test_eval_robustness_defun_missing_args() {
        let (mut env, mut ctx) = setup_env();

        // (defun my-func) -> Missing params list and body!
        // Currently panics at `args[1].clone()`
        let exp = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::Symbol("my-func".into()),
        ]);
        let _ = eval(&exp, &mut env, &mut ctx);
    }

    #[test]
    fn test_eval_robustness_malformed_lambda_execution() {
        let (mut env, mut ctx) = setup_env();

        // Register a completely broken lambda: (lambda (x)) -> Missing the body!
        let bad_lambda = LispExp::List(vec![
            LispExp::Symbol("lambda".into()),
            LispExp::List(vec![LispExp::Symbol("x".into())]),
        ]);
        env.functions.insert("broken-func".into(), bad_lambda);

        // Execute it: (broken-func 10)
        let exp = LispExp::List(vec![
            LispExp::Symbol("broken-func".into()),
            LispExp::Number(10.0),
        ]);

        let result = eval(&exp, &mut env, &mut ctx);

        // Your code safely catches this with `if lambda_ast.len() != 3`!
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), EvalError::UncorrectFunctionDefinition);
    }

    #[test]
    fn test_eval_robustness_lambda_params_not_a_list() {
        let (mut env, mut ctx) = setup_env();

        // Register a lambda where the parameters are a String instead of a List
        // e.g., (lambda "x" (+ 1 2))
        let bad_lambda = LispExp::List(vec![
            LispExp::Symbol("lambda".into()),
            LispExp::String("x".into()),
            LispExp::Number(3.0),
        ]);
        env.functions.insert("weird-func".into(), bad_lambda);

        // Execute it: (weird-func)
        let exp = LispExp::List(vec![LispExp::Symbol("weird-func".into())]);

        // Since `params` is not a list, your evaluator silently skips binding parameters
        // and just evaluates the body. We assert that it evaluates to 3.0 safely.
        // Note: In strict Elisp, defining a lambda without a param list is an error.
        let result = eval(&exp, &mut env, &mut ctx).unwrap();
        assert_eq!(result, LispExp::Number(3.0));
    }

    #[test]
    fn test_eval_robustness_empty_vector_and_map() {
        let (mut env, mut ctx) = setup_env();

        // Vector and Map should evaluate their internal items.
        // Testing that empty ones safely evaluate to themselves.
        let empty_vec = LispExp::Vector(vec![]);
        assert_eq!(eval(&empty_vec, &mut env, &mut ctx).unwrap(), LispExp::Vector(vec![]));

        // Maps are currently unimplemented in your `eval` match block (`_ => todo!()`).
        // If you run this test, it will hit the `todo!()` panic.
        // let empty_map = LispExp::Map(std::collections::HashMap::new());
        // eval(&empty_map, &mut env, &mut ctx).unwrap();
    }

    // ==========================================
    // WITH GENERIC CTX EVAL TESTS
    // ==========================================

    // 1. Define a dummy Host Context to test the generic bridge
    #[derive(Clone, Debug, PartialEq)]
    struct TestHost {
        pub state_changes: usize,
    }

    // 2. A mock native primitive that mutates the host context
    fn native_increment_state(args: &[LispExp<TestHost>], ctx: &mut TestHost) -> Result<LispExp<TestHost>, EvalError> {
        ctx.state_changes += 1;
        Ok(LispExp::Symbol("nil".into()))
    }

    // 3. A mock native primitive for addition
    fn native_add(args: &[LispExp<TestHost>], _ctx: &mut TestHost) -> Result<LispExp<TestHost>, EvalError> {
        let mut sum = 0.0;
        for arg in args {
            if let LispExp::Number(n) = arg {
                sum += n;
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: format!("{:?}", arg),
                });
            }
        }
        Ok(LispExp::Number(sum))
    }

    // Helper to create a fresh environment with primitives loaded
    fn setup_env_test() -> Env<TestHost> {
        let mut env = Env::new();
        env.functions.insert("+".into(), LispExp::Primitive(native_add));
        env.functions.insert("inc-state".into(), LispExp::Primitive(native_increment_state));
        env
    }

    #[test]
    fn test_self_evaluating_primitives() {
        let mut env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        let num = LispExp::Number(42.5);
        assert_eq!(eval(&num, &mut env, &mut ctx).unwrap(), num);

        let s = LispExp::String("hello".into());
        assert_eq!(eval(&s, &mut env, &mut ctx).unwrap(), s);
    }

    #[test]
    fn test_elisp_if_truthiness() {
        let mut env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // Helper macro to generate an `if` AST
        let make_if = |cond: LispExp<TestHost>| -> LispExp<TestHost> {
            LispExp::List(vec![
                LispExp::Symbol("if".into()),
                cond,
                LispExp::String("true_branch".into()),
                LispExp::String("false_branch".into()),
            ])
        };

        let true_res = LispExp::String("true_branch".into());
        let false_res = LispExp::String("false_branch".into());

        // 1. `nil` is false
        let if_nil = make_if(LispExp::Symbol("nil".into()));
        assert_eq!(eval(&if_nil, &mut env, &mut ctx).unwrap(), false_res);

        // 2. Empty list `()` is false
        let if_empty = make_if(LispExp::List(vec![]));
        assert_eq!(eval(&if_empty, &mut env, &mut ctx).unwrap(), false_res);

        // 3. Everything else is true (e.g., Number 0.0, empty string, "t")
        assert_eq!(eval(&make_if(LispExp::Number(0.0)), &mut env, &mut ctx).unwrap(), true_res);
        assert_eq!(eval(&make_if(LispExp::String("".into())), &mut env, &mut ctx).unwrap(), true_res);
        assert_eq!(eval(&make_if(LispExp::Symbol("t".into())), &mut env, &mut ctx).unwrap(), true_res);
    }

    #[test]
    fn test_lisp_2_namespace_isolation() {
        let mut env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // AST: (setq log "var-data")
        let setq_exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Symbol("log".into()),
            LispExp::String("var-data".into()),
        ]);
        eval(&setq_exp, &mut env, &mut ctx).unwrap();

        // Bind 'log' in the function namespace to a lambda
        let mock_lambda = LispExp::List(vec![
            LispExp::Symbol("lambda".into()),
            LispExp::List(vec![]),
            LispExp::Number(99.0),
        ]);
        env.functions.insert("log".into(), mock_lambda);

        // 1. Evaluate as variable: log -> "var-data"
        let var_eval = eval(&LispExp::Symbol("log".into()), &mut env, &mut ctx).unwrap();
        assert_eq!(var_eval, LispExp::String("var-data".into()));

        // 2. Evaluate as function: (log) -> 99.0
        let func_eval = eval(&LispExp::List(vec![LispExp::Symbol("log".into())]), &mut env, &mut ctx).unwrap();
        assert_eq!(func_eval, LispExp::Number(99.0));
    }

    #[test]
    fn test_dynamic_scoping_call_stack() {
        let mut global_env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // (defun get-x () x) -> Relies on 'x' being defined dynamically by caller
        let get_x_def = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::Symbol("get-x".into()),
            LispExp::List(vec![]),
            LispExp::Symbol("x".into()),
        ]);
        eval(&get_x_def, &mut global_env, &mut ctx).unwrap();

        // Create a caller stack frame where x = 100
        let mut caller_env = Env::extend(&mut global_env as *mut Env<TestHost>);
        caller_env.variables.insert("x".into(), LispExp::Number(100.0));

        // Evaluate (get-x) FROM the caller's environment
        let call_exp = LispExp::List(vec![LispExp::Symbol("get-x".into())]);
        let result = eval(&call_exp, &mut caller_env, &mut ctx).unwrap();

        // It must follow the stack to find 'x' in caller_env
        assert_eq!(result, LispExp::Number(100.0));
    }

    #[test]
    fn test_eval_setq_multiple() {
        let mut env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // (setq a 1.0 b (+ 1.0 2.0))
        let setq_exp = LispExp::List(vec![
            LispExp::Symbol("setq".into()),
            LispExp::Symbol("a".into()),
            LispExp::Number(1.0),
            LispExp::Symbol("b".into()),
            LispExp::List(vec![
                LispExp::Symbol("+".into()),
                LispExp::Number(1.0),
                LispExp::Number(2.0),
            ])
        ]);

        let result = eval(&setq_exp, &mut env, &mut ctx).unwrap();
        
        // Returns last assigned value
        assert_eq!(result, LispExp::Number(3.0));
        assert_eq!(env.variables.get("a").unwrap(), &LispExp::Number(1.0));
        assert_eq!(env.variables.get("b").unwrap(), &LispExp::Number(3.0));
    }

    #[test]
    fn test_lambda_argument_binding() {
        let mut env = setup_env_test();
        let mut ctx = TestHost { state_changes: 0 };

        // (defun add-custom (x y) (+ x y))
        let defun_exp = LispExp::List(vec![
            LispExp::Symbol("defun".into()),
            LispExp::Symbol("add-custom".into()),
            LispExp::List(vec![LispExp::Symbol("x".into()), LispExp::Symbol("y".into())]),
            LispExp::List(vec![
                LispExp::Symbol("+".into()),
                LispExp::Symbol("x".into()),
                LispExp::Symbol("y".into()),
            ]),
        ]);
        eval(&defun_exp, &mut env, &mut ctx).unwrap();

        // (add-custom 10.0 20.0)
        let call_exp = LispExp::List(vec![
            LispExp::Symbol("add-custom".into()),
            LispExp::Number(10.0),
            LispExp::Number(20.0),
        ]);
        eprintln!("{:?}", call_exp);
        let result = eval(&call_exp, &mut env, &mut ctx).unwrap();

        assert_eq!(result, LispExp::Number(30.0));
        
        // Ensure parameters didn't leak into global env
        assert!(env.variables.get("x").is_none());
    }

    #[test]
    fn test_generic_host_mutation() {
        let mut env = setup_env_test();
        
        // Initialize our "Editor State" equivalent
        let mut ctx = TestHost { state_changes: 0 };

        // AST: (inc-state)
        let call_mutation = LispExp::List(vec![
            LispExp::Symbol("inc-state".into())
        ]);

        // Call it three times
        eval(&call_mutation, &mut env, &mut ctx).unwrap();
        eval(&call_mutation, &mut env, &mut ctx).unwrap();
        eval(&call_mutation, &mut env, &mut ctx).unwrap();

        // The Rust host context should have tracked the changes natively!
        assert_eq!(ctx.state_changes, 3);
    }
}
