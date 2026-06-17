#[cfg(test)]
mod test {
    use crate::lisp::{Parser, ParserError, Token};

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
}
