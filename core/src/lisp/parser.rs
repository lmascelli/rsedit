//! ========================================================================== //
//!                    +----------------------------------------+
//!                    |  Parsing system for the lisp language  |
//!                    +----------------------------------------+
//! ========================================================================== //

use super::{LispContext, LispExp};
use std::collections::HashMap;

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

/// Convert a form produced by the reader into the data it denotes.
/// Runs once, at read time, on quoted structure only.
pub fn form_to_data<T: LispContext>(exp: &LispExp<T>) -> LispExp<T> {
    match exp {
        LispExp::Form(items) => LispExp::proper_list(items.iter().map(form_to_data).collect()),
        LispExp::Vector(items) => LispExp::vec(items.iter().map(form_to_data).collect()),
        LispExp::Map(m) => LispExp::map(
            m.iter()
                .map(|(k, v)| (k.clone(), form_to_data(v)))
                .collect(),
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
                    return Ok(LispExp::form(list));
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
                return Ok(LispExp::form(vec![
                    LispExp::symbol("quote".into()),
                    form_to_data(&quoted),
                ]));
            }
            Token::BackQuote => {
                self.advance_token()?;
                return Ok(LispExp::form(vec![
                    LispExp::symbol("backquote".into()),
                    self.next()?,
                ]));
            }
            Token::Comma => {
                self.advance_token()?;
                return Ok(LispExp::form(vec![
                    LispExp::symbol("unquote".into()),
                    self.next()?,
                ]));
            }
            Token::CommaAt => {
                self.advance_token()?;
                return Ok(LispExp::form(vec![
                    LispExp::symbol("unquote-splicing".into()),
                    self.next()?,
                ]));
            }
            Token::Void => Err(ParserError::VoidExp),
            _ => unreachable!("token parse not implemented for {:?}", self.current_token),
        }
    }
}
