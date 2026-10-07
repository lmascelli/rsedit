//! The reader's behaviour on input that is broken, deep or unusual.
//!
//! Each section is one entry of the problem report (`doc/rapporto-problemi.typ`,
//! LET-1 to LET-6). The broken inputs come from that report and from the
//! reader fuzzer that found most of them; every one of them used to hang,
//! panic, overflow the stack or read as something it is not.
#[cfg(test)]
mod tests {
    use crate::lisp::parser::{MAX_READ_DEPTH, ParserError};
    use crate::lisp::{Env, LispExp, Parser, Token, eval, setup_base_env};
    use std::sync::mpsc;
    use std::time::Duration;

    fn read(source: &str) -> Result<LispExp<()>, ParserError> {
        Parser::new(source).next()
    }

    fn tokens(source: &str) -> Result<Vec<Token>, ParserError> {
        let mut parser = Parser::new(source);
        let mut out = Vec::new();
        while let Some(token) = parser.next_token()? {
            out.push(token);
        }
        Ok(out)
    }

    fn sym(name: &str) -> LispExp<()> {
        LispExp::symbol(name.into())
    }

    /// Run BODY on its own thread and fail, rather than hang the whole test
    /// run, if it has not finished within a few seconds.
    fn within_seconds(body: impl FnOnce() + Send + 'static) {
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            body();
            let _ = done.send(());
        });
        finished
            .recv_timeout(Duration::from_secs(10))
            .expect("the reader did not finish");
    }

    // =====================================================================
    // LET-1: input ending inside a token
    // =====================================================================

    #[test]
    fn input_ending_inside_a_symbol_in_a_list_is_an_unclosed_list() {
        within_seconds(|| {
            assert_eq!(read("(foo"), Err(ParserError::UnclosedList));
            assert_eq!(read("(a (b c"), Err(ParserError::UnclosedList));
            assert_eq!(read("(-"), Err(ParserError::UnclosedList));
            assert_eq!(read("'(x"), Err(ParserError::UnclosedList));
            assert_eq!(read("(message"), Err(ParserError::UnclosedList));
            assert_eq!(read("[x"), Err(ParserError::UnclosedVector));
            assert_eq!(read("{a"), Err(ParserError::UnclosedMap));
        });
    }

    #[test]
    fn the_end_of_the_input_stays_the_end() {
        let mut parser = Parser::new("(a");
        assert_eq!(parser.next_token(), Ok(Some(Token::LParen)));
        assert_eq!(parser.next_token(), Ok(Some(Token::Symbol("a".into()))));
        for _ in 0..3 {
            assert_eq!(parser.next_token(), Ok(None));
        }
    }

    /// The test that would have found LET-1: every prefix of a valid program
    /// is read to a value or an error, and never forever.
    #[test]
    fn every_prefix_of_a_program_is_read_to_an_end() {
        within_seconds(|| {
            let program = r#"(defun f (x &optional y) "doc \"q\"" ; comment
  (let ((v [1 -2.5 .9 1e3]) (m {a 1 "b" 2}))
    `(,x ,@(list 'a #'car) . -) (cons 'a 'b) (- x)))"#;
            let chars: Vec<char> = program.chars().collect();
            for end in 0..=chars.len() {
                let prefix: String = chars[..end].iter().collect();
                let mut parser = Parser::new(&prefix);
                // Each successful read consumes at least one token, so a
                // prefix can yield no more expressions than it has characters.
                for _ in 0..=prefix.len() {
                    if parser.next::<()>().is_err() {
                        break;
                    }
                }
            }
        });
    }

    #[test]
    fn eval_string_safe_reports_an_unclosed_list() {
        within_seconds(|| {
            let env = Env::new_root();
            setup_base_env(env.clone());
            let ast: LispExp<()> = read(r#"(eval-string-safe "(message")"#).unwrap();
            let answer = eval(&ast, env, &()).expect("eval-string-safe never fails");
            let (items, _) = answer.split_list();
            assert!(items[0].is_nil(), "{answer:?}");
        });
    }

    // =====================================================================
    // LET-2: tokens that cannot begin an expression
    // =====================================================================

    #[test]
    fn a_misplaced_dot_is_an_error_not_a_panic() {
        assert_eq!(read("[1 . 2]"), Err(ParserError::UnexpectedDot));
        assert_eq!(read("{a .}"), Err(ParserError::UnexpectedDot));
        assert_eq!(read(". a"), Err(ParserError::UnexpectedDot));
        assert_eq!(read("'."), Err(ParserError::UnexpectedDot));
    }

    #[test]
    fn a_closing_delimiter_after_a_prefix_is_an_error_not_a_panic() {
        let unexpected = |spelling: &str| Err(ParserError::UnexpectedToken(spelling.into()));
        assert_eq!(read("(list 'a ')"), unexpected(")"));
        assert_eq!(read("(a ,)"), unexpected(")"));
        assert_eq!(read("[1 `]"), unexpected("]"));
        assert_eq!(read("{:k ,@}"), unexpected("}"));
    }

    // =====================================================================
    // LET-3: nesting
    // =====================================================================

    fn nested(depth: usize) -> String {
        format!("{}{}", "(".repeat(depth), ")".repeat(depth))
    }

    #[test]
    fn nesting_up_to_the_limit_is_read() {
        assert!(read(&nested(MAX_READ_DEPTH)).is_ok());
        let quotes = format!("{}x", "'".repeat(MAX_READ_DEPTH - 1));
        assert!(read(&quotes).is_ok());
    }

    #[test]
    fn nesting_past_the_limit_is_an_error_not_a_stack_overflow() {
        assert_eq!(read(&nested(MAX_READ_DEPTH + 1)), Err(ParserError::TooDeep));
        // Far past it, where the recursion used to abort the process.
        assert_eq!(read(&nested(100_000)), Err(ParserError::TooDeep));
        assert_eq!(read(&"[".repeat(100_000)), Err(ParserError::TooDeep));
        assert_eq!(read(&"'".repeat(100_000)), Err(ParserError::TooDeep));
    }

    #[test]
    fn the_depth_is_counted_per_expression_not_per_input() {
        // Many shallow expressions in a row must not add up to a deep one.
        let source = "(a) ".repeat(MAX_READ_DEPTH * 2);
        let mut parser = Parser::new(&source);
        for _ in 0..MAX_READ_DEPTH * 2 {
            assert!(parser.next::<()>().is_ok());
        }
        // And neither must the levels an error abandoned.
        let mut parser = Parser::new("((((x] (y)");
        assert!(parser.next::<()>().is_err());
    }

    // =====================================================================
    // LET-4: numbers
    // =====================================================================

    #[test]
    fn numbers_are_read_in_every_usual_spelling() {
        for (source, value) in [
            (".9", 0.9),
            (".5", 0.5),
            ("-.5", -0.5),
            ("+5", 5.0),
            ("1.", 1.0),
            ("1e5", 100_000.0),
            ("2.5e-3", 0.0025),
            ("1E+2", 100.0),
            ("-7", -7.0),
        ] {
            assert_eq!(read(source), Ok(LispExp::number(value)), "{source}");
        }
    }

    #[test]
    fn things_that_merely_contain_digits_stay_symbols() {
        for source in [
            "1+", "1-", "inf", "NaN", "-", "+", "1e", "e5", "1.5a", "1-2", "...",
        ] {
            assert_eq!(read(source), Ok(sym(source)), "{source}");
        }
    }

    #[test]
    fn digits_and_dots_that_are_not_a_number_are_an_error() {
        for source in ["1.2.3", "1..2", "..5"] {
            assert!(
                matches!(read(source), Err(ParserError::NumberParseError(_))),
                "{source}"
            );
        }
    }

    // =====================================================================
    // LET-5: what ends a symbol
    // =====================================================================

    #[test]
    fn a_carriage_return_ends_a_symbol() {
        assert_eq!(
            tokens("foo\rbar\r\n"),
            Ok(vec![
                Token::Symbol("foo".into()),
                Token::Symbol("bar".into())
            ])
        );
    }

    #[test]
    fn quotes_and_commas_end_a_symbol() {
        assert_eq!(
            tokens(r#"foo"bar" a,b c'd e`f"#),
            Ok(vec![
                Token::Symbol("foo".into()),
                Token::String("bar".into()),
                Token::Symbol("a".into()),
                Token::Comma,
                Token::Symbol("b".into()),
                Token::Symbol("c".into()),
                Token::Quote,
                Token::Symbol("d".into()),
                Token::Symbol("e".into()),
                Token::BackQuote,
                Token::Symbol("f".into()),
            ])
        );
    }

    #[test]
    fn sharp_quote_reads_as_quote() {
        assert_eq!(
            read("#'car"),
            Ok(LispExp::form(vec![sym("quote"), sym("car")]))
        );
        // A `#` before anything else is an ordinary symbol character.
        assert_eq!(read("#foo"), Ok(sym("#foo")));
        // `?` has no reading of its own: there is no character type.
        assert_eq!(read("?a"), Ok(sym("?a")));
    }

    // =====================================================================
    // LET-6: no state survives an error
    // =====================================================================

    #[test]
    fn reading_carries_on_cleanly_after_a_bad_number() {
        let mut parser = Parser::new("1.2.3 foo");
        assert!(matches!(
            parser.next::<()>(),
            Err(ParserError::NumberParseError(_))
        ));
        assert_eq!(parser.next::<()>(), Ok(sym("foo")));
    }

    #[test]
    fn a_closing_delimiter_alone_is_an_error() {
        // The input that reached the `todo!()` the old lexer kept for a
        // pending token it could not name.
        assert_eq!(read(")"), Err(ParserError::UnbalancedRParen));
        assert_eq!(read("]"), Err(ParserError::UnbalancedRSquared));
        assert_eq!(read("}"), Err(ParserError::UnbalancedRBracket));
    }
}
