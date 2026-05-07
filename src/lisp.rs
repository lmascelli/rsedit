#[derive(Clone, Debug, PartialEq)]
pub enum Token {
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

enum ParserState {
    Default,
    InSymbol,
    InString,
    InStringSlash,
    InNumber,
}

pub enum ParserError {
    UnbalancedRParen,
    UnbalancedRSquared,
    UnbalancedRBracket,
    NumberParseError(String),
    NumberInvadidChar(char),
}

pub struct Parser<'source> {
    source: std::str::Chars<'source>,
    token: String,
    parens_stack: Vec<Token>,
    state: ParserState,
}

impl<'source> Parser<'source> {
    fn new(source: &'source String) -> Self {
        Self {
            source: source.chars(),
            token: String::new(),
            parens_stack: Vec::new(),
            state: ParserState::Default,
        }
    }

    fn next_atom(&mut self) -> Result<Option<Token>, ParserError> {
        while let Some(c) = self.source.next() {
            match self.state {
                ParserState::Default => {
                    match c {
                        '(' => {
                            self.parens_stack.push(Token::LParen);
                            return Ok(Some(Token::LParen));
                        }
                        ')' => {
                            if Some(Token::LParen) == self.parens_stack.pop() {
                                return Ok(Some(Token::RParen));
                            } else {
                                return Err(ParserError::UnbalancedRParen);
                            }
                        }
                        '[' => {
                            self.parens_stack.push(Token::LSquared);
                            return Ok(Some(Token::LSquared));
                        }
                        ']' => {
                            if Some(Token::LSquared) == self.parens_stack.pop() {
                                return Ok(Some(Token::RSquared));
                            } else {
                                return Err(ParserError::UnbalancedRSquared);
                            }
                        }
                        '{' => {
                            self.parens_stack.push(Token::LBracket);
                            return Ok(Some(Token::LBracket));
                        }
                        '}' => {
                            if Some(Token::LBracket) == self.parens_stack.pop() {
                                return Ok(Some(Token::RBracket));
                            } else {
                                return Err(ParserError::UnbalancedRBracket);
                            }
                        }
                        ' ' => { continue; }
                        '"' => {
                            self.state = ParserState::InString;
                        }
                        '0'..='9' => {
                            self.token.push(c);
                            self.state = ParserState::InNumber;
                            continue;
                        }
                        _ => {
                            self.token.push(c);
                            self.state = ParserState::InSymbol;
                            continue;
                        }
                    }
                }
                ParserState::InSymbol => {
                    match c {
                        ' ' => {
                            let mut token_string = String::new();
                            core::mem::swap(&mut token_string, &mut self.token);
                            self.state = ParserState::Default;
                            return Ok(Some(Token::Symbol(token_string)));
                        }
                        _ => {
                            self.token.push(c);
                            continue;
                        }
                    }
                }
                ParserState::InString => {
                    match c {
                        '"' => {
                            let mut token_string = String::new();
                            core::mem::swap(&mut token_string, &mut self.token);
                            self.state = ParserState::Default;                            
                            return Ok(Some(Token::String(token_string)));
                        }
                        '\\' => {
                            self.state = ParserState::InStringSlash;
                            continue;
                        }
                        _ => {
                            self.token.push(c);
                            continue;
                        }
                    }
                }
                ParserState::InStringSlash => {
                    match c {
                        '"' => {
                            self.token.push(c);
                            continue;
                        }
                        _ => {
                            self.token.push('\\');
                            self.token.push(c);
                            self.state = ParserState::InString;
                            continue;
                        }
                    }
                }
                ParserState::InNumber => {
                    match c {
                        '0'..='9' => {
                            self.token.push(c);
                            self.state = ParserState::InNumber;
                            continue;
                        }
                        ' ' => {
                            let mut token_string = String::new();
                            core::mem::swap(&mut token_string, &mut self.token);
                            if let Ok(number) = token_string.parse() {
                                self.state = ParserState::Default;                                
                                return Ok(Some(Token::Number(number)));
                            } else {
                                return Err(ParserError::NumberParseError(token_string));
                            }
                        }
                        _ => {
                            return Err(ParserError::NumberInvadidChar(c));
                        }
                    }                    
                }
            }
        }
        // handle pending token
        match self.state {
            ParserState::InSymbol => {
                let mut token_string = String::new();
                core::mem::swap(&mut token_string, &mut self.token);
                return Ok(Some(Token::Symbol(token_string)));
            }
            _ => todo!()
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================
    // LEXER TESTS
    // ==========================================
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
        let ast_num = parse("42").unwrap();
        assert_eq!(ast_num, LispExp::Number(42.0));

        let ast_sym = parse("my-symbol").unwrap();
        assert_eq!(ast_sym, LispExp::Symbol("my-symbol".to_string()));
    }

    #[test]
    fn test_parse_simple_list() {
        let ast = parse("(print \"world\")").unwrap();

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
        let ast = parse("(define x (+ 10 20))").unwrap();

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
        let ast = parse("(((1)))").unwrap();
        
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
        let err1 = parse("(+ 1 2").unwrap_err();
        assert!(
            err1.contains("EOF") ||
            err1.contains("Unclosed") ||
            err1.contains("parenthesis"));

        // Unexpected closing parenthesis
        let err2 = parse("(+ 1 2))").unwrap_err();
        assert!(err2.contains("Unexpected") || err2.contains("parenthesis"));
    }
}
