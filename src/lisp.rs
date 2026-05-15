use std::rc::Rc;
use std::cell::RefCell;
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
pub enum LispExp {
    List(Vec<LispExp>),
    Vector(Vec<LispExp>),
    Map(HashMap<String, LispExp>),
    Number(f64),
    Symbol(String),
    String(String),
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
                ParserLexerState::Default => {
                    match c {
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
                        ' ' | '\t' | '\n'  => { }
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
                    }
                }
                ParserLexerState::InSymbol => {
                    match c {
                        ' ' | '\t' | '\n' => {
                            let mut token_string = String::new();
                            core::mem::swap(&mut token_string, &mut self.token);
                            self.lexer_state = ParserLexerState::Default;
                            self.source.next();
                            return Ok(Some(Token::Symbol(token_string)));
                        }
                        '(' | '[' | '{' |
                        ')' | ']' | '}' => {
                            let mut token_string = String::new();
                            core::mem::swap(&mut token_string, &mut self.token);
                            self.lexer_state = ParserLexerState::Default;
                            return Ok(Some(Token::Symbol(token_string)));
                        }
                        _ => {
                            self.token.push(*c);
                        }
                    }
                }
                ParserLexerState::InString => {
                    match c {
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
                    }
                }
                ParserLexerState::InStringSlash => {
                    match c {
                        '"' | '\\' => {
                            self.token.push(*c);
                            self.lexer_state = ParserLexerState::InString;
                        }
                        _ => {
                            self.token.push('\\');
                            self.token.push(*c);
                            self.lexer_state = ParserLexerState::InString;
                        }
                    }
                }
                ParserLexerState::InNumber => {
                    match c {
                        '0'..='9' => {
                            self.token.push(*c);
                        }
                        '.' => {
                            self.token.push(*c);
                            self.lexer_state = ParserLexerState::InNumberAfterDot;
                        }
                        '(' | '[' | '{' |
                        ')' | ']' | '}' => {
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
                    }
                }
                ParserLexerState::InNumberAfterDot => {
                    match c {
                        '0'..='9' => {
                            self.token.push(*c);
                        }
                        '(' | '[' | '{' |
                        ')' | ']' | '}' => {
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
                    }
                }
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
                        _ => {todo!("{:?}", self.token);}
                    }
                }
                else {
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
            _ => { todo!("Simbolo alla fine non gestito"); }
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

    fn parse_list(&mut self) -> Result<LispExp, ParserError> {
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

    fn parse_vector(&mut self) -> Result<LispExp, ParserError> {
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

    fn parse_map(&mut self) -> Result<LispExp, ParserError> {
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
                    _ => { return Err(ParserError::InvalidMapKey); }
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

    pub fn next(&mut self) -> Result<LispExp, ParserError> {
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
            },
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
            Token::Void => {
                Err(ParserError::VoidExp)
            } 
            _ => todo!("token parse not implemented for {:?}", self.current_token)
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum EvalError {
    UnboundSymbol(String),
    TypeMismatch(String),
    ArityMismatch(String),
    NotAFunction(String),
}

pub struct Env {
    vars: HashMap<String, LispExp>,
    outer: Option<Rc<RefCell<Env>>>,
}

impl Env {
    pub fn new() -> Self {
        todo!()
    }

    pub fn new_with_outer(outer: Rc<RefCell<Env>>) -> Self {
        todo!()
    }

    pub fn set(&mut self, name: String, value: LispExp) {
        todo!()
    }

    pub fn get(&mut self, name: String) -> Option<LispExp> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(tokens, vec![
            Token::LParen,
            Token::LSquared,
            Token::LBracket,
            Token::RBracket,
            Token::RSquared,
            Token::RParen,
        ]);
    }

    #[test]
    fn test_symbols_and_spaces() {
        let input = "def  my-var   + ";
        let tokens = lex_all(input).unwrap();
        assert_eq!(tokens, vec![
            Token::Symbol("def".into()),
            Token::Symbol("my-var".into()),
            Token::Symbol("+".into()),
        ]);
    }

    #[test]
    fn test_strings() {
        let input = r#""hello world" "escaped \" quote""#;
        let tokens = lex_all(input).unwrap();
        assert_eq!(tokens, vec![
            Token::String("hello world".into()),
            Token::String("escaped \" quote".into()),
        ]);
    }

    #[test]
    fn test_numbers() {
        // Note: these require a trailing space currently based on your logic
        let input = "123 45.67 0.1 ";
        let tokens = lex_all(input).unwrap();
        assert_eq!(tokens, vec![
            Token::Number(123.0),
            Token::Number(45.67),
            Token::Number(0.1),
        ]);
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

        assert_eq!(tokens, vec![
            Token::LParen,
            Token::Symbol("factorial".into()),
            Token::Number(5.0),
            Token::RParen,
            Token::LParen,
            Token::Symbol("add".into()),
            Token::Symbol("x".into()),
            Token::Symbol("y".into()),
            Token::RParen,
        ]);
    }

    #[test]
    fn test_complex_strings() {
        let input = r#" "line1\nline2" "quote: \" " "" "#;
        let tokens = lex_all(input).unwrap();

        assert_eq!(tokens, vec![
            // Note: Your current InStringSlash logic doesn't handle \n yet,
            // it just pushes the \ and the n.
            Token::String("line1\\nline2".into()),
            Token::String("quote: \" ".into()),
            Token::String("".into()), // Empty string
        ]);
    }

    #[test]
    fn test_deep_nesting() {
        let input = "(let [x {y 10}] (fn (z) [z x]))";
        let tokens = lex_all(input).unwrap();

        let expected = vec![
            Token::LParen, Token::Symbol("let".into()),
            Token::LSquared, Token::Symbol("x".into()),
            Token::LBracket, Token::Symbol("y".into()), Token::Number(10.0), Token::RBracket,
            Token::RSquared,
            Token::LParen, Token::Symbol("fn".into()),
            Token::LParen, Token::Symbol("z".into()), Token::RParen,
            Token::LSquared, Token::Symbol("z".into()), Token::Symbol("x".into()), Token::RSquared,
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
    #[test]
    fn test_parse_primitives() {
        let mut parser = Parser::new("42");

        let ast_num = parser.next().unwrap();
        assert_eq!(ast_num, LispExp::Number(42.0));

        let mut parser = Parser::new("my-symbol");
        let ast_sym = parser.next().unwrap();
        assert_eq!(ast_sym, LispExp::Symbol("my-symbol".to_string()));
    }

    #[test]
    fn test_parse_simple_list() {

        let mut parser = Parser::new("(print \"world\")");
        let ast = parser.next().unwrap();

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
        let ast = parser.next().unwrap();

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
        let ast = parser.next().unwrap();

        assert_eq!(
            ast,
            LispExp::List(vec![
                LispExp::List(vec![
                    LispExp::List(vec![
                        LispExp::Number(1.0)
                    ])
                ])
            ])
        );
    }

    #[test]
    fn test_parse_errors() {

        // Missing closing parenthesis
        let mut parser = Parser::new("(+1 2");
        let err1 = parser.next().unwrap_err();
        assert_eq!(err1, ParserError::UnclosedList);

        // Unexpected closing parenthesis
        eprintln!("{:?}", Parser::new("(+ 1 2))").next().unwrap_err());
        let mut parser = Parser::new("(+1 2))");
        let err2 = parser.next().unwrap_err();
        assert_eq!(err2, ParserError::UnbalancedRParen);
    }

    #[test]
    fn test_vector_parsing() {
        // Testing that [1 2 3] creates a Vector, not a List
        let input = "[1 2 3]";
        let mut parser = Parser::new(input);
        let ast = parser.next().expect("Should parse vector");

        match ast {
            LispExp::Vector(v) => {
                assert_eq!(v.len(), 3);
                assert_eq!(v[0], LispExp::Number(1.0));
            },
            _ => panic!("Expected Vector, found {:?}", ast),
        }
    }

    #[test]
    fn test_map_parsing_valid() {
        // Testing { "name" "Gemini" "version" 3 }
        let input = r#"{ "name" "Gemini" "version" 3.0 }"#;
        let mut parser = Parser::new(input);
        let ast = parser.next().expect("Should parse map");

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
        let ast = parser.next().expect("Should parse complex structure");

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
        let result = p1.next();
        assert!(matches!(result, Err(ParserError::MapKeyMissingValue)));

        // Case 2: Using a non-string/symbol as a key
        let input_bad_key = "{ 42 \"answer\" }";
        let mut p2 = Parser::new(input_bad_key);
        let result = p2.next();
        assert!(matches!(result, Err(ParserError::InvalidMapKey)));
    }

    #[test]
    fn test_delimiter_clash_no_spaces() {
        // Testing that the lexer correctly breaks symbols at [ or {
        // This checks your InSymbol state transitions [cite: 17-22]
        let input = "my-func[1 2]{:key val}";
        let mut parser = Parser::new(input);
        
        // Should yield: Symbol, Vector, Map
        assert!(matches!(parser.next().unwrap(), LispExp::Symbol(_)));
        assert!(matches!(parser.next().unwrap(), LispExp::Vector(_)));
        assert!(matches!(parser.next().unwrap(), LispExp::Map(_)));
    }

    #[test]
    fn test_edge_negative_numbers() {
        let input = "-42 -3.14";
        let mut parser = Parser::new(input);
        
        assert_eq!(parser.next().unwrap(), LispExp::Number(-42.0));
        assert_eq!(parser.next().unwrap(), LispExp::Number(-3.14));
    }

    #[test]
    fn test_edge_naked_decimals() {
        let input = ".5 5. 0.0";
        let mut parser = Parser::new(input);

        assert_eq!(parser.next().unwrap(), LispExp::Number(0.5));
        assert_eq!(parser.next().unwrap(), LispExp::Number(5.0));
        assert_eq!(parser.next().unwrap(), LispExp::Number(0.0));
    }

    #[test]
    fn test_edge_brutal_string_escaping() {
        let input = r#" "\\\"" "#; // Represents the string: \"
        let mut parser = Parser::new(input);

        let ast = parser.next().unwrap();
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
        let ast = parser.next().unwrap();
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

        let ast = parser.next().unwrap();
        
        // Expected: List containing one Vector containing one Map (all empty)
        assert_eq!(
            ast,
            LispExp::List(vec![
                LispExp::Vector(vec![
                    LispExp::Map(std::collections::HashMap::new())
                ])
            ])
        );
    }

    #[test]
    fn test_complex_whitespace_and_mixed_delimiters() {
        // Added the string "data" to act as the Map's key, 
        // with the Vector acting as the value.
        let input = "{\n\t\"data\"\n\t[ (1\n2\t3) ]\n}";
        let mut parser = Parser::new(input);
        let ast = parser.next().expect("Should parse successfully");

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
        let exp1 = parser.next().unwrap();
        assert_eq!(exp1, LispExp::Vector(vec![LispExp::Number(1.5)]));

        // Test {"k" .5} - Map boundary
        let exp2 = parser.next().unwrap();
        if let LispExp::Map(mut m) = exp2 {
            assert_eq!(m.remove("k"), Some(LispExp::Number(0.5)));
        } else {
            panic!("Expected Map for second expression");
        }

        // Test "5." - EOF/whitespace boundary
        let exp3 = parser.next();
        println!("Result for 5.: {:?}", exp3);
    }

    #[test]
    fn test_empty_and_escaped_literals() {
        let input = r#" "" () [] {} "back\\slash" "#;
        let mut parser = Parser::new(input);

        assert_eq!(parser.next().unwrap(), LispExp::String("".into()));
        assert_eq!(parser.next().unwrap(), LispExp::List(vec![]));
        
        // These now expect Vector and Map instead of List
        assert_eq!(parser.next().unwrap(), LispExp::Vector(vec![]));
        assert_eq!(parser.next().unwrap(), LispExp::Map(std::collections::HashMap::new()));
        
        // Check backslash escaping logic
        assert_eq!(parser.next().unwrap(), LispExp::String("back\\slash".into()));
    }

    #[test]
    fn test_malformed_input_recovery() {
        // Unclosed list 
        let mut p1 = Parser::new("(1 2 3");
        assert_eq!(p1.next().unwrap_err(), ParserError::UnclosedList);

        // Mismatched brackets
        let mut p2 = Parser::new("(1 2 3]");
        assert_eq!(p2.next().unwrap_err(), ParserError::UnbalancedRSquared);

        // Multiple decimal points 
        let mut p3 = Parser::new("1.2.3");
        assert!(matches!(p3.next().unwrap_err(), ParserError::NumberParseError(_)));

        // NEW: Map with missing value
        let mut p4 = Parser::new("{ \"key\" }");
        assert_eq!(p4.next().unwrap_err(), ParserError::MapKeyMissingValue);

        // NEW: Map with invalid key type (number instead of string/symbol)
        let mut p5 = Parser::new("{ 42 \"value\" }");
        assert_eq!(p5.next().unwrap_err(), ParserError::InvalidMapKey);
    }
}
