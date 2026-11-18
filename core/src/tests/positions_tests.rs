//! Lines and columns in characters, UTF-16 code units and UTF-8 bytes: both
//! ways, on the characters each encoding counts differently, at the edges of
//! a line and of the text, on a string alone, and from Lisp.
//!
//! Design: `doc/architettura-editor.typ, <posizioni-unita>`.
#[cfg(test)]
mod tests {
    use crate::buffer::BufferTrait;
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::text::positions::{Unit, line_column, position, string_line_column};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// Line 1 holds a character of two UTF-8 bytes, one outside the Basic
    /// Multilingual Plane (two UTF-16 units, four bytes), and an x.
    const MIXED: &str = "a\nè😀x\n";

    const UNITS: [Unit; 3] = [Unit::Char, Unit::Utf16, Unit::Utf8];

    #[test]
    fn a_column_counts_each_character_by_the_width_of_its_unit() {
        let text = GapBuffer::from(MIXED);
        // Before è, before 😀, before x, at the end of the line.
        let at = [2, 3, 4, 5];
        let columns = |unit| at.map(|pos| line_column(&text, pos, unit));
        assert_eq!(columns(Unit::Char), [(1, 0), (1, 1), (1, 2), (1, 3)]);
        assert_eq!(columns(Unit::Utf16), [(1, 0), (1, 1), (1, 3), (1, 4)]);
        assert_eq!(columns(Unit::Utf8), [(1, 0), (1, 2), (1, 6), (1, 7)]);
    }

    #[test]
    fn every_position_comes_back_from_its_line_and_column_in_each_unit() {
        let text = GapBuffer::from("ascii\n\ttab é\n𝄞 clef 😀😀\r\n\nend");
        for unit in UNITS {
            for pos in 0..=text.len() {
                let (line, column) = line_column(&text, pos, unit);
                assert_eq!(
                    position(&text, line, column, unit),
                    pos,
                    "{unit:?}: {pos} is ({line}, {column})"
                );
            }
        }
    }

    #[test]
    fn a_column_inside_a_character_is_the_position_before_it() {
        let text = GapBuffer::from(MIXED);
        // 😀 is UTF-16 columns 1..3 and UTF-8 columns 2..6 of line 1.
        assert_eq!(position(&text, 1, 2, Unit::Utf16), 3);
        for column in 3..6 {
            assert_eq!(position(&text, 1, column, Unit::Utf8), 3, "column {column}");
        }
        assert_eq!(position(&text, 1, 1, Unit::Utf8), 2, "inside è");
    }

    #[test]
    fn a_column_past_the_end_of_its_line_is_the_line_end() {
        let text = GapBuffer::from(MIXED);
        for unit in UNITS {
            assert_eq!(position(&text, 0, 99, unit), 1, "{unit:?}, line 0");
            assert_eq!(position(&text, 1, 99, unit), 5, "{unit:?}, line 1");
        }
    }

    #[test]
    fn a_line_past_the_last_is_the_end_of_the_text() {
        let text = GapBuffer::from(MIXED);
        for unit in UNITS {
            // Line 2 is the empty line after the final newline, and exists.
            assert_eq!(position(&text, 2, 0, unit), text.len(), "{unit:?}");
            assert_eq!(position(&text, 7, 3, unit), text.len(), "{unit:?}");
        }
    }

    #[test]
    fn a_position_past_the_end_is_the_end() {
        let text = GapBuffer::from(MIXED);
        assert_eq!(line_column(&text, 1_000, Unit::Utf16), (2, 0));
        let empty = GapBuffer::from("");
        for unit in UNITS {
            assert_eq!(line_column(&empty, 0, unit), (0, 0));
            assert_eq!(position(&empty, 0, 5, unit), 0);
        }
    }

    #[test]
    fn a_carriage_return_is_a_character_of_its_line() {
        let text = GapBuffer::from("ab\r\ncd");
        for unit in UNITS {
            assert_eq!(line_column(&text, 3, unit), (0, 3), "{unit:?}");
            assert_eq!(position(&text, 1, 0, unit), 4, "{unit:?}");
        }
    }

    #[test]
    fn a_string_ends_where_its_text_alone_would_end() {
        for string in [MIXED, "", "è😀", "\n", "ab\r\nè😀x", "😀\n\nè"] {
            let text = GapBuffer::from(string);
            for unit in UNITS {
                assert_eq!(
                    string_line_column(string, unit),
                    line_column(&text, text.len(), unit),
                    "{unit:?}: {string:?}"
                );
            }
        }
    }

    #[test]
    fn a_string_counts_its_newlines_and_the_column_after_the_last() {
        let measured = |unit| string_line_column("a\nb\nè😀x", unit);
        assert_eq!(measured(Unit::Char), (2, 3));
        assert_eq!(measured(Unit::Utf16), (2, 4));
        assert_eq!(measured(Unit::Utf8), (2, 7));
        // Without a newline the column counts from the start of the string.
        assert_eq!(string_line_column("è😀", Unit::Utf16), (0, 3));
        assert_eq!(string_line_column("a\n", Unit::Utf8), (1, 0));
    }

    // ---- from Lisp ------------------------------------------------------

    fn editor_with(text: &str) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        ctx.with_current_buffer_mut(|buf| buf.text = GapBuffer::from(text));
        (ctx, env)
    }

    fn try_run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn shown(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        let value =
            try_run(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"));
        risp::lisp_display(&value)
    }

    #[test]
    fn lisp_counts_lines_from_1_and_columns_from_0_in_the_unit_named() {
        let (ctx, env) = editor_with(MIXED);
        assert_eq!(shown("(position-to-line-column 4)", &env, &ctx), "(2 2)");
        assert_eq!(
            shown("(position-to-line-column 4 'char)", &env, &ctx),
            "(2 2)"
        );
        assert_eq!(
            shown("(position-to-line-column 4 'utf-16)", &env, &ctx),
            "(2 3)"
        );
        assert_eq!(
            shown("(position-to-line-column 4 'utf-8)", &env, &ctx),
            "(2 6)"
        );
        assert_eq!(
            shown("(position-to-line-column 0 nil)", &env, &ctx),
            "(1 0)"
        );
    }

    #[test]
    fn lisp_turns_a_line_and_a_column_back_into_the_same_position() {
        let (ctx, env) = editor_with(MIXED);
        assert_eq!(
            shown("(line-column-to-position 2 3 'utf-16)", &env, &ctx),
            "4"
        );
        assert_eq!(
            shown("(line-column-to-position 2 6 \"utf-8\")", &env, &ctx),
            "4"
        );
        assert_eq!(shown("(line-column-to-position 2 2)", &env, &ctx), "4");
        // Line 0 and a negative column are the start of the first line.
        assert_eq!(
            shown("(line-column-to-position 0 -4 'utf-8)", &env, &ctx),
            "0"
        );
        assert_eq!(shown("(line-column-to-position 99 0)", &env, &ctx), "6");
    }

    #[test]
    fn lisp_measures_a_string_in_newlines_and_a_column_counted_from_0() {
        let (ctx, env) = editor_with("");
        let src = "(string-line-column \"ab\\nè😀\"";
        assert_eq!(shown(&format!("{src})"), &env, &ctx), "(1 2)");
        assert_eq!(shown(&format!("{src} 'utf-16)"), &env, &ctx), "(1 3)");
        assert_eq!(shown(&format!("{src} \"utf-8\")"), &env, &ctx), "(1 6)");
        assert_eq!(shown("(string-line-column \"\" nil)", &env, &ctx), "(0 0)");
    }

    #[test]
    fn lisp_refuses_a_unit_it_does_not_know_and_a_position_that_is_not_a_number() {
        let (ctx, env) = editor_with(MIXED);
        for src in [
            "(position-to-line-column 0 'utf-32)",
            "(position-to-line-column \"0\")",
            "(line-column-to-position 1 0 'bytes)",
            "(line-column-to-position 1)",
            "(string-line-column 'ab)",
            "(string-line-column \"ab\" 'bytes)",
        ] {
            assert!(
                matches!(
                    try_run(src, &env, &ctx),
                    Err(EvalError::WrongArgumentType { .. })
                ),
                "{src}"
            );
        }
    }
}
