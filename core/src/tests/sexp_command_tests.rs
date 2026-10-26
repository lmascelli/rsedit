//! The expression commands as a user reaches them, and `syntax-ppss`: that a
//! command reaches the scanner, that a kill goes through the two doors, and
//! that the same text moves differently in two modes. The scanner itself is
//! tested in `sexp_tests`, against tables built in Rust.
//!
//! Design: `doc/architettura-editor.typ, <espressioni-indentazione>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

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

    /// The editor with `indent.lisp` loaded, and the files it is written
    /// against.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/indent.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// A mode whose syntax table is a Lisp's.
    fn with_lisp_mode(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        run(
            "(make-mode 'toy-lisp)
             (set-syntax-pairs 'toy-lisp \"()[]\")
             (set-syntax-entry 'toy-lisp \"'`,\" 'prefix)
             (set-syntax-entry 'toy-lisp \"-*+<>=?!\" 'symbol)
             (set-comment-syntax 'toy-lisp '((\";\")))",
            env,
            ctx,
        );
    }

    fn in_mode(ctx: &Ctx, mode: &str) {
        ctx.with_buffer_mut("*scratch*", |b| b.current_mode = mode.into());
    }

    fn typing(ctx: &Ctx, text: &str, at: usize) {
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text);
            let (line, col) = b.text.cursor_1d_to_2d(at);
            b.text.cursor_move(line, col);
            b.is_modified = false;
        });
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.with_buffer("*scratch*", |b| b.text.to_string())
            .expect("*scratch*")
    }

    fn point(ctx: &Ctx) -> usize {
        ctx.with_buffer("*scratch*", |b| b.text.cursor_pos_1d())
            .expect("*scratch*")
    }

    fn number(exp: &LispExp<Ctx>) -> f64 {
        match exp {
            LispExp::Number(n) => *n,
            other => panic!("expected a number, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // A kill is an edit like any other
    // -----------------------------------------------------------------------

    #[test]
    fn killing_an_expression_can_be_undone_in_one_step() {
        // Through the two doors, so undo takes the kill back in one step.
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a (b c)) rest", 0);

        run("(kill-sexp)", &env, &ctx);
        assert_eq!(contents(&ctx), " rest");
        run("(undo)", &env, &ctx);
        assert_eq!(contents(&ctx), "(a (b c)) rest");
    }

    #[test]
    fn a_killed_expression_goes_to_the_kill_ring() {
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a b) rest", 0);

        run("(kill-sexp)", &env, &ctx);
        run("(yank)", &env, &ctx);
        assert_eq!(contents(&ctx), "(a b) rest");
    }

    #[test]
    fn a_read_only_buffer_refuses_the_kill() {
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a b) rest", 0);
        run("(set-buffer-read-only t)", &env, &ctx);

        run("(kill-sexp)", &env, &ctx);
        assert_eq!(contents(&ctx), "(a b) rest");
    }

    #[test]
    fn kill_sexp_removes_exactly_what_forward_sexp_traverses() {
        // The kill asks the scanner where the motion ends, so the two take the
        // same text.
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");

        for text in ["(a (b)) rest", "(message \"a )\") rest", "'foo rest"] {
            typing(&ctx, text, 0);
            run("(forward-sexp)", &env, &ctx);
            let traversed = point(&ctx);

            typing(&ctx, text, 0);
            run("(kill-sexp)", &env, &ctx);
            let mut expected = String::new();
            expected.push_str(&text[traversed..]);
            assert_eq!(contents(&ctx), expected, "killing {text:?}");
        }
    }

    // -----------------------------------------------------------------------
    // Reachable from the keyboard
    // -----------------------------------------------------------------------

    #[test]
    fn the_sexp_keys_reach_the_commands() {
        let (ctx, env) = editor();
        eval_str(include_str!("../../lisp/common-keymaps.lisp"), &env, &ctx)
            .expect("loading common-keymaps.lisp");
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a b) (c)", 0);

        let ctrl_alt = || KeyModifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: ctrl_alt(),
            },
            &env,
        );
        assert_eq!(point(&ctx), 5, "C-M-f steps over the first list");

        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: ctrl_alt(),
            },
            &env,
        );
        assert_eq!(point(&ctx), 0, "C-M-b comes back");

        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: ctrl_alt(),
            },
            &env,
        );
        assert_eq!(contents(&ctx), " (c)", "C-M-k kills it");
    }

    #[test]
    fn the_sexp_commands_are_available_from_m_x() {
        let (ctx, _env) = editor();
        let names = ctx.command_names();
        for name in [
            "forward-sexp",
            "backward-sexp",
            "kill-sexp",
            "backward-kill-sexp",
            "up-list",
            "backward-up-list",
            "down-list",
            "recenter",
            "indent-for-tab-command",
        ] {
            assert!(names.iter().any(|known| known == name), "{name} is missing");
        }
    }

    // -----------------------------------------------------------------------
    // The same text, two modes
    // -----------------------------------------------------------------------

    #[test]
    fn the_same_text_moves_differently_in_two_modes() {
        // In a Lisp `;` opens a comment, so the `)` inside it is text and the
        // list runs on; in a mode that never said so it is punctuation, and
        // the list ends at the first `)` it meets.
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        run(
            "(make-mode 'toy-plain) (set-syntax-pairs 'toy-plain \"()\")",
            &env,
            &ctx,
        );

        let text = "(a ; ) not the end\n b) after";

        in_mode(&ctx, "toy-lisp");
        typing(&ctx, text, 0);
        run("(forward-sexp)", &env, &ctx);
        assert_eq!(point(&ctx), 22, "the `)` in the comment is text");

        in_mode(&ctx, "toy-plain");
        typing(&ctx, text, 0);
        run("(forward-sexp)", &env, &ctx);
        assert_eq!(
            point(&ctx),
            6,
            "with no comment syntax it ends at the first `)`"
        );
    }

    // -----------------------------------------------------------------------
    // syntax-ppss
    // -----------------------------------------------------------------------

    #[test]
    fn syntax_ppss_in_a_string_reports_the_string_and_not_a_comment() {
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a \"bc\" d)", 5);

        let state = run("(syntax-ppss)", &env, &ctx);
        let parts: Vec<LispExp<Ctx>> = state.iter().collect();
        assert_eq!(number(&parts[0]), 1.0, "a string does not change the depth");
        assert_eq!(
            number(&parts[2]),
            3.0,
            "element 2 is where the string began"
        );
        assert_eq!(
            parts[3],
            LispExp::nil(),
            "element 3 says it is not a comment"
        );
    }

    #[test]
    fn syntax_ppss_reports_the_delimiter_as_well_as_the_expression() {
        // Element 4. They differ by the prefix, and indentation lines up
        // against the delimiter while motion wants the expression.
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "'(a b)", 3);

        let parts: Vec<LispExp<Ctx>> = run("(syntax-ppss)", &env, &ctx).iter().collect();
        assert_eq!(number(&parts[1]), 0.0, "the expression starts at the quote");
        assert_eq!(
            number(&parts[4]),
            1.0,
            "the delimiter is the paren after it"
        );
    }

    #[test]
    fn bounds_of_enclosing_list_is_the_whole_list() {
        let (ctx, env) = editor();
        with_lisp_mode(&ctx, &env);
        in_mode(&ctx, "toy-lisp");
        typing(&ctx, "(a (b c) d)", 5);

        let parts: Vec<LispExp<Ctx>> = run("(bounds-of-enclosing-list)", &env, &ctx)
            .iter()
            .collect();
        assert_eq!((number(&parts[0]), number(&parts[1])), (3.0, 8.0));

        typing(&ctx, "(a b)", 0);
        assert_eq!(
            run("(bounds-of-enclosing-list)", &env, &ctx),
            LispExp::nil(),
            "nothing encloses the top level"
        );
    }
}
