//! The balanced-expression scanner, and the syntax tables that drive it.
//!
//! # What these are protecting
//!
//! That the scanner never counts a delimiter it is not reading as code. Three
//! of the four parentheses in `(message "close it with )")` are text, and a
//! scanner that gets this wrong does not fail -- it silently puts the cursor
//! somewhere strange, or reports a defun boundary in the middle of a string.
//! There is nothing downstream that would catch it.
//!
//! The tests come in two halves. The first drives `crate::modes::sexp`
//! directly with tables built in Rust: the scanner is a pure function from
//! (text, table, offset) to offsets, so it needs no buffer, no mode registry
//! and no keymap to test. The second checks that a mode can actually *say* any
//! of this from Lisp.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use crate::modes::sexp::{self, Context};
    use crate::modes::{CommentStyle, SyntaxClass, SyntaxTable};
    use std::sync::Arc;

    // -----------------------------------------------------------------------
    // Two tables, as two real modes would declare them
    // -----------------------------------------------------------------------

    fn lisp_table() -> SyntaxTable {
        let mut table = SyntaxTable::default();
        table.set_pairs("()[]");
        for c in "'`,#".chars() {
            table.set_class(c, SyntaxClass::Prefix);
        }
        for c in "-*+<>=?!".chars() {
            table.set_class(c, SyntaxClass::Symbol);
        }
        table.set_comments(vec![CommentStyle::Line { opener: ";".into() }]);
        table
    }

    fn rust_table() -> SyntaxTable {
        let mut table = SyntaxTable::default();
        table.set_pairs("()[]{}");
        // Punctuation, not a quote: `'a'` is a character and `&'a str` is a
        // lifetime, and no per-character table can tell them apart. Called a
        // string quote, every lifetime in the file would open one.
        table.set_class('\'', SyntaxClass::Punctuation);
        table.set_comments(vec![
            CommentStyle::Line {
                opener: "//".into(),
            },
            CommentStyle::Block {
                opener: "/*".into(),
                closer: "*/".into(),
                nestable: true,
            },
        ]);
        table
    }

    fn text(source: &str) -> GapBuffer {
        GapBuffer::from(source)
    }

    fn forward(table: &SyntaxTable, source: &str, from: usize) -> usize {
        sexp::forward(&text(source), table, from, 1)
    }

    fn backward(table: &SyntaxTable, source: &str, from: usize) -> usize {
        sexp::backward(&text(source), table, from, 1)
    }

    fn context(table: &SyntaxTable, source: &str, at: usize) -> Context {
        sexp::context_at(&text(source), table, at)
    }

    /// The text `enclosing` picks out, which reads far better in a failure
    /// than a pair of offsets.
    fn enclosing(table: &SyntaxTable, source: &str, at: usize) -> Option<String> {
        sexp::enclosing(&text(source), table, at).map(|found| {
            source
                .chars()
                .skip(found.start)
                .take(found.end - found.start)
                .collect()
        })
    }

    // -----------------------------------------------------------------------
    // What one expression is
    // -----------------------------------------------------------------------

    #[test]
    fn an_atom_is_an_expression() {
        let table = lisp_table();
        assert_eq!(forward(&table, "foo bar", 0), 3);
        assert_eq!(forward(&table, "foo bar", 3), 7);
    }

    #[test]
    fn a_list_is_one_expression_however_deep() {
        let table = lisp_table();
        assert_eq!(forward(&table, "(a (b (c))) after", 0), 11);
    }

    #[test]
    fn a_string_is_one_expression() {
        let table = lisp_table();
        assert_eq!(forward(&table, "\"a b c\" after", 0), 7);
    }

    #[test]
    fn a_list_spanning_lines_is_still_one_expression() {
        let table = lisp_table();
        assert_eq!(forward(&table, "(defun f ()\n  (g))\nafter", 0), 18);
    }

    #[test]
    fn a_quote_belongs_to_what_it_quotes() {
        let table = lisp_table();
        assert_eq!(forward(&table, "'foo bar", 0), 4);
        assert_eq!(backward(&table, "'(a b) x", 6), 0);
        assert_eq!(backward(&table, "`,x", 3), 0, "a run of prefixes is one");
    }

    #[test]
    fn a_quote_detached_by_a_space_is_not_part_of_the_next_expression() {
        let table = lisp_table();
        assert_eq!(backward(&table, "' foo", 5), 2);
    }

    // -----------------------------------------------------------------------
    // The part a paren count gets wrong
    // -----------------------------------------------------------------------

    #[test]
    fn a_parenthesis_in_a_string_is_text() {
        let table = lisp_table();
        assert_eq!(
            forward(&table, "(message \"close it with )\") after", 0),
            27,
            "the list ends at its own closing paren, not the one in the string"
        );
    }

    #[test]
    fn a_parenthesis_in_a_line_comment_is_text() {
        let table = lisp_table();
        assert_eq!(forward(&table, "(a ; ) not the end\n b) after", 0), 22);
    }

    #[test]
    fn an_escaped_quote_does_not_end_a_string() {
        let table = lisp_table();
        assert_eq!(forward(&table, "\"say \\\" hi\" after", 0), 11);
    }

    #[test]
    fn a_comment_character_inside_a_string_starts_nothing() {
        let table = lisp_table();
        assert_eq!(forward(&table, "(a \"; not a comment\" b) after", 0), 23);
    }

    #[test]
    fn a_string_quote_inside_a_comment_opens_nothing() {
        // The other direction, and the one that hangs a scanner rather than
        // merely misplacing the cursor: an unterminated string swallows the
        // rest of the file.
        let table = lisp_table();
        assert_eq!(forward(&table, "(a ; \" unbalanced\n b) after", 0), 21);
    }

    #[test]
    fn a_backslash_in_a_comment_is_not_an_escape() {
        // Escapes are honoured in strings only. Treating this one as an escape
        // would consume the newline and run the comment into the next line.
        let table = lisp_table();
        assert_eq!(forward(&table, "(a ; ends here \\\n b) after", 0), 20);
    }

    // -----------------------------------------------------------------------
    // Rust: two comment styles, nesting, and the apostrophe
    // -----------------------------------------------------------------------

    #[test]
    fn a_line_comment_needs_both_its_characters() {
        let table = rust_table();
        // A single `/` is division, not a comment; only `//` opens one.
        assert_eq!(forward(&table, "(a / b) rest", 0), 7);
        assert_eq!(forward(&table, "(a // ) not the end\n b) rest", 0), 23);
    }

    #[test]
    fn a_block_comment_spans_lines() {
        let table = rust_table();
        assert_eq!(forward(&table, "(a /* ) \n still */ b) rest", 0), 21);
    }

    #[test]
    fn block_comments_nest() {
        // The `nestable` flag: without it this closes at the first `*/` and
        // the trailing `*/` becomes code.
        //
        // The text has to be chosen so that nesting changes the *answer*: with
        // `/* /* */ */` alone the list ends in the same place either way,
        // because the stray `*/` left over is only punctuation. Put a `)`
        // between the inner and outer close and the difference shows -- nested,
        // it is inside the comment; not nested, it ends the list early.
        let table = rust_table();
        assert_eq!(
            forward(&table, "(a /* /* */ ) */ b) rest", 0),
            19,
            "the `)` inside the nested comment is text"
        );
    }

    #[test]
    fn two_openers_sharing_a_prefix_pick_the_longer() {
        // Lua's real syntax: `--` opens a line comment and `--[[` a block one.
        // Both match wherever the second does, so the scan must prefer the
        // longer or every block comment is read as a line comment and ends at
        // the first newline.
        let mut table = SyntaxTable::default();
        table.set_comments(vec![
            CommentStyle::Line {
                opener: "--".into(),
            },
            CommentStyle::Block {
                opener: "--[[".into(),
                closer: "]]".into(),
                nestable: false,
            },
        ]);
        assert_eq!(
            forward(&table, "--[[ a\n b ]] c", 0),
            12,
            "the block comment runs past the newline to `]]`"
        );
        assert_eq!(
            forward(&table, "-- a\nb", 0),
            6,
            "and a plain `--` is still a line comment"
        );
    }

    #[test]
    fn the_longest_opener_wins() {
        // `//` and `/*` share a first character, so the scan must not settle
        // for whichever style it happens to check first.
        let table = rust_table();
        assert_eq!(
            forward(&table, "/* a */ b", 0),
            7,
            "the block comment, not the `/` of a line comment"
        );
        assert_eq!(
            forward(&table, "// a\nb", 0),
            6,
            "the line comment is skipped and `b' is the expression -- see \
             a_line_comment_is_skipped_but_a_block_comment_is_not"
        );
    }

    #[test]
    fn an_apostrophe_in_rust_opens_nothing() {
        // A lifetime and a character literal are spelled alike. As punctuation
        // neither opens a string, so neither can swallow the file.
        let table = rust_table();
        assert_eq!(forward(&table, "(f('a', &'a str)) rest", 0), 17);
    }

    #[test]
    fn braces_nest_in_rust_and_are_not_lisp_delimiters() {
        let rust = rust_table();
        assert_eq!(forward(&rust, "{ a { b } } rest", 0), 11);
        let lisp = lisp_table();
        assert_eq!(
            forward(&lisp, "{abc} rest", 0),
            5,
            "a mode that never declared braces reads them as punctuation and \
             symbol, so this is `abc' padded -- not one expression"
        );
    }

    #[test]
    fn a_line_comment_is_skipped_but_a_block_comment_is_not() {
        // Pinning an inconsistency rather than endorsing it. The `Line` arm of
        // the scan never calls `complete`, so a line comment is passed over the
        // way whitespace is; the `Block` arm calls `complete_comment`, so a
        // block comment is an expression that `forward-sexp` stops at the end
        // of. Emacs skips both.
        //
        // Whichever way it should go, it should go the same way for both. This
        // test exists so that making them agree is a visible decision and not a
        // silent change.
        let table = rust_table();
        assert_eq!(
            forward(&table, "/* a */ b", 0),
            7,
            "stops at the end of the block comment"
        );
        assert_eq!(
            forward(&table, "// a\nb", 0),
            6,
            "steps over the line comment and lands after `b'"
        );
    }

    // -----------------------------------------------------------------------
    // Files that are not balanced, which is all of them while being typed
    // -----------------------------------------------------------------------

    #[test]
    fn a_mismatched_closer_closes_anyway() {
        // Decided deliberately: a closer that does not match its opener still
        // closes it. Refusing would strand the scanner exactly when the file is
        // half-written, which is when the motion is being used.
        let table = lisp_table();
        assert_eq!(forward(&table, "(a] rest", 0), 3);
    }

    #[test]
    fn a_stray_closer_does_not_swallow_the_rest_of_the_buffer() {
        let table = lisp_table();
        assert_eq!(forward(&table, ") foo bar", 0), 5);
        assert_eq!(backward(&table, ") foo bar", 9), 6);
    }

    #[test]
    fn an_unclosed_list_reaches_the_end_of_the_buffer() {
        let table = lisp_table();
        assert_eq!(enclosing(&table, "(a (b c", 5).as_deref(), Some("(b c"));
    }

    // -----------------------------------------------------------------------
    // Repeat counts
    // -----------------------------------------------------------------------

    #[test]
    fn a_count_moves_over_that_many_expressions() {
        let table = lisp_table();
        let buffer = text("(a) (b) (c)");
        assert_eq!(sexp::forward(&buffer, &table, 0, 2), 7);
        assert_eq!(sexp::backward(&buffer, &table, 11, 2), 4);
    }

    #[test]
    fn a_count_larger_than_what_is_left_does_nothing() {
        // Nothing, rather than as far as it can: a motion that silently moved a
        // different distance than asked would be useless to build on.
        let table = lisp_table();
        let buffer = text("(a) (b)");
        assert_eq!(sexp::forward(&buffer, &table, 0, 5), 0);
        assert_eq!(sexp::backward(&buffer, &table, 7, 5), 7);
    }

    // -----------------------------------------------------------------------
    // Where point starts
    // -----------------------------------------------------------------------

    #[test]
    fn inside_a_list_the_siblings_are_what_move() {
        let table = lisp_table();
        assert_eq!(forward(&table, "(a b c)", 1), 2, "over a");
        assert_eq!(forward(&table, "(a b c)", 2), 4, "over b");
        assert_eq!(backward(&table, "(a b c)", 6), 5, "back over c");
    }

    #[test]
    fn at_the_end_of_a_list_forward_does_not_escape_it() {
        let table = lisp_table();
        assert_eq!(forward(&table, "(a b)", 4), 4);
    }

    #[test]
    fn at_the_start_of_a_list_backward_does_not_escape_it() {
        let table = lisp_table();
        assert_eq!(backward(&table, "(a b)", 1), 1);
    }

    #[test]
    fn from_inside_a_symbol_backward_reaches_its_start() {
        let table = lisp_table();
        assert_eq!(backward(&table, "foobar baz", 3), 0);
    }

    // -----------------------------------------------------------------------
    // What point is inside
    // -----------------------------------------------------------------------

    #[test]
    fn the_depth_is_how_many_lists_point_is_in() {
        let table = lisp_table();
        assert_eq!(context(&table, "(a (b c) d)", 0).depth, 0);
        assert_eq!(context(&table, "(a (b c) d)", 1).depth, 1);
        assert_eq!(context(&table, "(a (b c) d)", 5).depth, 2);
        assert_eq!(context(&table, "(a (b c) d)", 9).depth, 1);
        assert_eq!(context(&table, "(a (b c) d)", 11).depth, 0);
    }

    #[test]
    fn the_innermost_list_is_the_one_point_is_in() {
        let table = lisp_table();
        assert_eq!(context(&table, "(a (b c) d)", 5).innermost, Some(3));
        assert_eq!(context(&table, "(a (b c) d)", 1).innermost, Some(0));
        assert_eq!(context(&table, "(a (b c) d)", 0).innermost, None);
    }

    #[test]
    fn point_knows_when_it_is_in_a_string() {
        let table = lisp_table();
        let found = context(&table, "(a \"bc\" d)", 5);
        assert_eq!(found.string_start, Some(3));
        assert_eq!(found.comment_start, None);
        assert_eq!(found.depth, 1, "a string does not change the depth");
    }

    #[test]
    fn point_knows_when_it_is_in_a_comment() {
        let table = lisp_table();
        let found = context(&table, "(a ; here\n b)", 6);
        assert_eq!(found.comment_start, Some(3));
        assert_eq!(found.string_start, None);
    }

    #[test]
    fn a_comment_is_over_once_its_line_is() {
        let table = lisp_table();
        assert_eq!(context(&table, "(a ; here\n b)", 11).comment_start, None);
    }

    #[test]
    fn the_enclosing_list_is_returned_whole() {
        let table = lisp_table();
        assert_eq!(
            enclosing(&table, "(a (b c) d)", 5).as_deref(),
            Some("(b c)")
        );
        assert_eq!(
            enclosing(&table, "(a (b c) d)", 1).as_deref(),
            Some("(a (b c) d)")
        );
        assert_eq!(enclosing(&table, "(a (b c) d)", 0), None, "top level");
    }

    // -----------------------------------------------------------------------
    // A mode with no table at all
    // -----------------------------------------------------------------------

    #[test]
    fn the_default_table_still_finds_expressions() {
        // Most buffers are in a mode that never mentioned syntax. Sexp motion
        // has to work there, which is what `SyntaxTable::default` is for.
        let table = SyntaxTable::default();
        assert_eq!(forward(&table, "(a (b)) rest", 0), 7);
        assert_eq!(forward(&table, "\"a b\" rest", 0), 5);
        assert_eq!(
            forward(&table, "; not a comment here\n", 0),
            5,
            "the default table declares no comments, so `;' is punctuation and \
             the first expression is the word `not'"
        );
    }

    // -----------------------------------------------------------------------
    // Saying all this from Lisp
    // -----------------------------------------------------------------------

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        run("(make-mode 'toy)", &env, &ctx);
        (ctx, env)
    }

    fn class(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match run(src, env, ctx) {
            LispExp::Symbol(name) => name.to_string(),
            other => panic!("expected a symbol, got {other:?}"),
        }
    }

    #[test]
    fn a_mode_can_declare_its_delimiters() {
        let (ctx, env) = editor();
        run("(set-syntax-pairs 'toy \"<>\")", &env, &ctx);
        assert_eq!(class("(syntax-class \"<\" 'toy)", &env, &ctx), "open");
        assert_eq!(class("(syntax-class \">\" 'toy)", &env, &ctx), "close");
    }

    #[test]
    fn a_table_starts_from_the_default_and_states_its_differences() {
        // Declaring one thing must not throw away the brackets, the quote and
        // the escape that every language shares.
        let (ctx, env) = editor();
        run("(set-syntax-pairs 'toy \"<>\")", &env, &ctx);
        assert_eq!(class("(syntax-class \"(\" 'toy)", &env, &ctx), "open");
        assert_eq!(class("(syntax-class \"\\\"\" 'toy)", &env, &ctx), "string");
    }

    #[test]
    fn a_trailing_odd_delimiter_is_ignored_rather_than_refused() {
        let (ctx, env) = editor();
        assert_eq!(
            run("(set-syntax-pairs 'toy \"<>[\")", &env, &ctx),
            LispExp::t()
        );
        assert_eq!(class("(syntax-class \"<\" 'toy)", &env, &ctx), "open");
    }

    #[test]
    fn a_mode_can_give_many_characters_one_class() {
        let (ctx, env) = editor();
        run("(set-syntax-entry 'toy \"'`,#\" 'prefix)", &env, &ctx);
        for c in ["'", "`", ",", "#"] {
            assert_eq!(
                class(&format!("(syntax-class \"{c}\" 'toy)"), &env, &ctx),
                "prefix"
            );
        }
    }

    #[test]
    fn the_same_character_means_different_things_in_different_modes() {
        // The whole reason the table is per mode.
        let (ctx, env) = editor();
        run("(make-mode 'other)", &env, &ctx);
        run("(set-syntax-entry 'toy \"'\" 'prefix)", &env, &ctx);
        run("(set-syntax-entry 'other \"'\" 'punctuation)", &env, &ctx);
        assert_eq!(class("(syntax-class \"'\" 'toy)", &env, &ctx), "prefix");
        assert_eq!(
            class("(syntax-class \"'\" 'other)", &env, &ctx),
            "punctuation"
        );
    }

    #[test]
    fn a_delimiter_cannot_be_declared_one_side_at_a_time() {
        // `open` needs to know its partner, so it is not a class you can name.
        let (ctx, env) = editor();
        assert!(eval_str("(set-syntax-entry 'toy \"<\" 'open)", &env, &ctx).is_err());
    }

    #[test]
    fn an_unknown_class_is_refused() {
        let (ctx, env) = editor();
        assert!(eval_str("(set-syntax-entry 'toy \"x\" 'whitespace)", &env, &ctx).is_err());
    }

    #[test]
    fn a_mode_can_declare_line_and_block_comments() {
        let (ctx, env) = editor();
        assert_eq!(
            run(
                "(set-comment-syntax 'toy '((\"//\") (\"/*\" \"*/\" t)))",
                &env,
                &ctx
            ),
            LispExp::t()
        );
    }

    #[test]
    fn a_comment_opener_cannot_be_empty() {
        // It would match at every position, and the first thing the scanner
        // looked at would turn the whole buffer into a comment.
        let (ctx, env) = editor();
        assert!(eval_str("(set-comment-syntax 'toy '((\"\")))", &env, &ctx).is_err());
    }

    #[test]
    fn declaring_syntax_for_an_unknown_mode_is_reported_not_signalled() {
        let (ctx, env) = editor();
        assert_eq!(
            run("(set-syntax-pairs 'no-such-mode \"()\")", &env, &ctx),
            LispExp::nil()
        );
        assert!(
            ctx.get_logs()
                .iter()
                .any(|line| line.contains("no-such-mode"))
        );
    }

    #[test]
    fn syntax_class_falls_back_to_the_default_table() {
        let (ctx, env) = editor();
        assert_eq!(
            class("(syntax-class \"(\" 'toy)", &env, &ctx),
            "open",
            "a mode that declared nothing still answers"
        );
        assert_eq!(class("(syntax-class \"x\" 'toy)", &env, &ctx), "symbol");
        assert_eq!(
            class("(syntax-class \"+\" 'toy)", &env, &ctx),
            "punctuation"
        );
    }

    #[test]
    fn syntax_class_defaults_to_the_current_buffers_mode() {
        let (ctx, env) = editor();
        run("(set-syntax-entry 'toy \"+\" 'symbol)", &env, &ctx);
        assert_eq!(
            class("(syntax-class \"+\")", &env, &ctx),
            "punctuation",
            "*scratch* is not in toy mode"
        );
        let scratch = ctx.get_buffer("*scratch*").expect("*scratch*");
        ctx.mutate_buffer(scratch, |b| b.current_mode = "toy".into());
        assert_eq!(class("(syntax-class \"+\")", &env, &ctx), "symbol");
    }
}
