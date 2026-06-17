#[cfg(test)]
mod test {
    // ==========================================
    // PARSER TESTS
    // ==========================================

    use crate::lisp::{Env, LispExp, Parser, ParserError};

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
        assert_eq!(
            p2.next::<()>().unwrap_err(),
            ParserError::UnbalancedRSquared
        );

        // Multiple decimal points
        let mut p3 = Parser::new("1.2.3");
        assert!(matches!(
            p3.next::<()>().unwrap_err(),
            ParserError::NumberParseError(_)
        ));

        // NEW: Map with missing value
        let mut p4 = Parser::new("{ \"key\" }");
        assert_eq!(
            p4.next::<()>().unwrap_err(),
            ParserError::MapKeyMissingValue
        );

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
            Parser::new("{ \"key1\" \"val1\" \"key2\" }")
                .next::<()>()
                .unwrap_err(),
            ParserError::MapKeyMissingValue
        );

        // Keys MUST be Strings or Symbols. Passing Lists or Vectors should fail.
        assert_eq!(
            Parser::new("{ [1 2] \"value\" }").next::<()>().unwrap_err(),
            ParserError::InvalidMapKey
        );
        assert_eq!(
            Parser::new("{ (func) \"value\" }")
                .next::<()>()
                .unwrap_err(),
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
}
