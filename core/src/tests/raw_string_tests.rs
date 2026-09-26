//! Multi-character string delimiters, and the scanner learning to read them.
//!
//! # The bug these come from
//!
//! The `string` syntax class says "this character opens a string and the same
//! one closes it", which is true of the double quote and of nothing else. A
//! Rust raw string opens with `r#"` and closes with `"#`, and the scanner --
//! knowing only the class -- read one as a *run* of ordinary strings with code
//! between them. Every bracket in the content then counted as a bracket.
//!
//! This editor's own source is the example. `create_global_env` holds the
//! default init.lisp in a raw string, and one of its comments reads
//! `; typing "(" gives you "()"`. That `(` opened a list that never closed, so
//! `core/src/editor/mod.rs` reported whole-file depth 1 for a file that balances --
//! and electric-pair, which refuses to pair when the buffer already balances,
//! paired everywhere in it.
//!
//! The fix is in the *table*, not in electric-pair: `forward-sexp`, the
//! indenter and `syntax-ppss` all read the table and are all fixed by it. This
//! is the third bug of that shape, after block comments and character
//! literals; the pattern is always the grammar (what the text looks like)
//! knowing about a construct that the table (what the text means) does not.
#[cfg(test)]
mod tests {
    use crate::buffer::BufferTrait;
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
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

    /// An editor with rust-mode and electric-pair, as the real one has them.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/indent.lisp"),
            include_str!("../../lisp/rust-mode.lisp"),
            include_str!("../../lisp/electric-pair.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// A fresh buffer in `mode` holding `text`, point at `at`.
    fn buffer_in(mode: &str, name: &str, text: &str, at: usize, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        run(
            &format!(
                r#"(buffer-create "{name}" '{mode}) (switch-to-buffer "{name}")
                   (clear-buffer) (insert "{escaped}") (goto-char {at})"#
            ),
            env,
            ctx,
        );
    }

    fn rust_buffer(name: &str, text: &str, at: usize, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        buffer_in("rust-mode", name, text, at, env, ctx);
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.with_current_buffer(|b| b.text.to_string())
    }

    /// The list depth at the end of the buffer: 0 when every list is closed.
    fn depth_at_end(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> f64 {
        match run("(nth 0 (syntax-ppss (point-max)))", env, ctx) {
            LispExp::Number(n) => n,
            other => panic!("expected a depth, got {other:?}"),
        }
    }

    // ----------------------------------------------------------------
    // What the scanner now sees
    // ----------------------------------------------------------------

    #[test]
    fn a_bracket_inside_a_raw_string_opens_nothing() {
        // The bug at its smallest: one unmatched bracket, inside quotes that
        // are themselves inside the raw string.
        let (ctx, env) = editor();
        rust_buffer(
            "a",
            "fn a() {\n  let s = r#\"typing \"(\" here\"#;\n}",
            0,
            &env,
            &ctx,
        );
        assert_eq!(
            depth_at_end(&env, &ctx),
            0.0,
            "the braces balance: nothing in the raw string was read as code"
        );
    }

    #[test]
    fn a_quote_inside_a_raw_string_does_not_end_it() {
        let (ctx, env) = editor();
        // Read as plain strings the quotes pair up as `"a "` and `" c"`, and
        // the brace between them counts -- which is one brace too many.
        rust_buffer("b", "let s = r#\"a \" { c\"#;\nfn f() {\n}", 0, &env, &ctx);
        assert_eq!(
            depth_at_end(&env, &ctx),
            0.0,
            "only the function's brace is a brace"
        );
    }

    #[test]
    fn a_backslash_inside_a_raw_string_escapes_nothing() {
        // Why `StyledStr` honours no escapes: a raw string exists so that a
        // backslash is a backslash. Were `\` an escape here, the `\"` would
        // not close the string and the `{` after it would go uncounted.
        let (ctx, env) = editor();
        rust_buffer("c", "let p = r\"c:\\\";\nfn f() {\n}", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn a_byte_raw_string_is_a_raw_string_too() {
        let (ctx, env) = editor();
        rust_buffer("d", "fn a() {\n  let b = br#\"(\"#;\n}", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn a_hashless_raw_string_is_a_raw_string_too() {
        let (ctx, env) = editor();
        rust_buffer("e", "fn a() {\n  let s = r\"(\";\n}", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn two_raw_strings_in_a_row_do_not_run_together() {
        // The closer must end the string rather than merely being noticed: if
        // it did not, the code between the two would be inside one long
        // string and the brace would be lost.
        let (ctx, env) = editor();
        rust_buffer(
            "f",
            "let a = r#\"(\"#; let b = r#\")\"#;\nfn f() {",
            0,
            &env,
            &ctx,
        );
        assert_eq!(depth_at_end(&env, &ctx), 1.0);
    }

    #[test]
    fn an_ordinary_string_is_still_an_ordinary_string() {
        // The style is tried before the class dispatch, so the plain quote had
        // to keep working -- including its escapes, which the raw form drops.
        let (ctx, env) = editor();
        rust_buffer("g", "fn a() {\n  let s = \"a \\\" ( b\";\n}", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn an_r_that_is_not_an_opener_is_ordinary_code() {
        // `r` is a perfectly good identifier, and `let r = ...` must not begin
        // anything.
        let (ctx, env) = editor();
        rust_buffer(
            "h",
            "fn a() {\n  let r = 1;\n  let rs = \"x\";\n}",
            0,
            &env,
            &ctx,
        );
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn a_raw_string_opener_inside_a_comment_opens_nothing() {
        // Comments are decided before strings, in that order, so prose about
        // raw strings is still prose.
        let (ctx, env) = editor();
        rust_buffer("i", "fn a() {\n  // like r#\"(\n}", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn a_mode_without_string_styles_reads_the_same_text_as_plain_strings() {
        // The other half of the previous tests: this is what rust-mode used to
        // do, and what a mode that has not declared raw strings still does.
        // `r#"a " ( b"#` scans as the string `"a "`, then ` ( ` as *code*, and
        // the bracket counts.
        let (ctx, env) = editor();
        run("(make-mode 'plain-mode)", &env, &ctx);
        buffer_in("plain-mode", "j", "let s = r#\"a \" ( b\"#;", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 1.0);
    }

    #[test]
    fn the_same_text_in_rust_mode_balances() {
        let (ctx, env) = editor();
        rust_buffer("k", "let s = r#\"a \" ( b\"#;", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn the_longest_opener_wins() {
        // Two styles where one opener is a prefix of the other. By position
        // alone the short one would always win and the long form would be
        // unreachable, so `<<a > ( b>>` would end at the first `>` and the
        // bracket would escape into the code.
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'angle-mode)
               (set-string-syntax 'angle-mode '(("<" ">") ("<<" ">>")))"#,
            &env,
            &ctx,
        );
        buffer_in("angle-mode", "l", "x = <<a > ( b>>;", 0, &env, &ctx);
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
    }

    #[test]
    fn clearing_the_styles_puts_the_mode_back() {
        let (ctx, env) = editor();
        run(
            r##"(make-mode 'clearable-mode)
               (set-string-syntax 'clearable-mode '(("r#\"" "\"#")))"##,
            &env,
            &ctx,
        );
        buffer_in(
            "clearable-mode",
            "m",
            "let s = r#\"a \" ( b\"#;",
            0,
            &env,
            &ctx,
        );
        assert_eq!(depth_at_end(&env, &ctx), 0.0);
        run("(set-string-syntax 'clearable-mode nil)", &env, &ctx);
        assert_eq!(
            depth_at_end(&env, &ctx),
            1.0,
            "with the styles gone the bracket is code again"
        );
    }

    // ----------------------------------------------------------------
    // The primitive's edges
    // ----------------------------------------------------------------

    #[test]
    fn set_string_syntax_on_an_unknown_mode_answers_nil() {
        let (ctx, env) = editor();
        assert!(
            run(
                r#"(set-string-syntax 'no-such-mode '(("<" ">")))"#,
                &env,
                &ctx
            )
            .is_nil(),
            "an unknown mode is reported, not invented"
        );
    }

    #[test]
    fn set_string_syntax_refuses_an_empty_delimiter() {
        // An empty opener matches at every position, which would make the
        // whole buffer a string the first time the scanner looked at it.
        let (ctx, env) = editor();
        run("(make-mode 'empty-mode)", &env, &ctx);
        assert!(
            eval_str(r#"(set-string-syntax 'empty-mode '(("" ">")))"#, &env, &ctx).is_err(),
            "an empty opener is refused"
        );
        assert!(
            eval_str(r#"(set-string-syntax 'empty-mode '(("<" "")))"#, &env, &ctx).is_err(),
            "an empty closer is refused"
        );
    }

    #[test]
    fn set_string_syntax_refuses_a_style_with_no_closer() {
        let (ctx, env) = editor();
        run("(make-mode 'half-mode)", &env, &ctx);
        assert!(eval_str(r#"(set-string-syntax 'half-mode '(("<")))"#, &env, &ctx).is_err());
    }

    // ----------------------------------------------------------------
    // What it fixes, as a user meets it
    // ----------------------------------------------------------------

    /// A file shaped like the reported one: a raw string holding text with an
    /// unbalanced bracket in it, and a perfectly ordinary function below.
    const REPORTED: &str = r####"const INIT: &str = r#"
(eval-file "electric-pair") ; typing "(" gives you "()"
"#;

fn push_call_frame(&self, frame: &str) %BODY%
    self.call_stack
        .write()
        .expect("lock")
        .push(frame.to_string());
}
"####;

    #[test]
    fn retyping_the_brace_of_a_function_below_a_raw_string_adds_no_second_one() {
        // The report: delete the `{` after the arguments, type it back, and a
        // pair appeared -- whose closer was the one already closing the body.
        let (ctx, env) = editor();
        let text = REPORTED.replace("%BODY%", "");
        let at = text
            .find("\n    self.call_stack")
            .expect("the body follows");
        rust_buffer("n", &text, at, &env, &ctx);
        run(
            r#"(self-insert "{") (electric-pair-post-self-insert)"#,
            &env,
            &ctx,
        );
        assert_eq!(contents(&ctx), REPORTED.replace("%BODY%", "{"));
    }

    #[test]
    fn deleting_a_brace_in_the_same_file_deletes_only_that_brace() {
        // The other half of the report: having been given a pair it should
        // not have had, deleting the stray closer took the opener with it.
        // Nothing here pairs, so nothing here unpairs.
        let (ctx, env) = editor();
        let text = REPORTED.replace("%BODY%", "{");
        let at = text.find('{').expect("the brace is there") + 1;
        rust_buffer("o", &text, at, &env, &ctx);
        run("(delete-backward-char)", &env, &ctx);
        assert_eq!(contents(&ctx), REPORTED.replace("%BODY%", ""));
    }

    #[test]
    fn pairing_still_works_below_a_raw_string_where_it_should() {
        // The check must not have become "never pair in a file with a raw
        // string in it": with the body's closing brace gone, typing the
        // opener should give a pair.
        let (ctx, env) = editor();
        let text = REPORTED.replace("%BODY%", "").replacen("\n}\n", "\n", 1);
        let at = text
            .find("\n    self.call_stack")
            .expect("the body follows");
        rust_buffer("p", &text, at, &env, &ctx);
        run(
            r#"(self-insert "{") (electric-pair-post-self-insert)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            contents(&ctx),
            text.replacen(" \n    self", " {}\n    self", 1)
        );
    }

    #[test]
    fn forward_sexp_steps_over_a_raw_string_as_one_thing() {
        // The table is read by more than electric-pair, and all of it is
        // fixed by the same change.
        let (ctx, env) = editor();
        rust_buffer("q", "r#\"a \" ( b\"# next", 0, &env, &ctx);
        run("(forward-sexp)", &env, &ctx);
        assert_eq!(
            run("(point)", &env, &ctx),
            LispExp::number(12.0),
            "point lands just after the closing `\"#`"
        );
    }

    // ----------------------------------------------------------------
    // The regression that would have caught it
    // ----------------------------------------------------------------

    /// Put `source` in a rust-mode buffer without going through `insert`.
    ///
    /// These files run to hundreds of kilobytes; round-tripping them through
    /// an escaped Lisp string literal costs more than the rest of the suite.
    /// The version bump is what `edits` would have done, and is what makes the
    /// syntax cache refuse the stale answer.
    fn rust_buffer_from_source(name: &str, source: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!(r#"(buffer-create "{name}" 'rust-mode) (switch-to-buffer "{name}")"#),
            env,
            ctx,
        );
        ctx.with_current_buffer_mut(|buffer| {
            buffer.text = GapBuffer::from(source);
            buffer.text.cursor_move(0, 0);
            buffer.version += 1;
        })
    }

    #[test]
    fn the_editors_own_sources_balance() {
        // The check electric-pair makes is over the whole buffer, so the test
        // that would have found this bug is one that reads a whole real file.
        // These four are the ones with raw strings in them; each reported a
        // depth of 1 or more before the fix, and each balances in fact.
        //
        // If this fails after an honest edit, look at the file first: a
        // genuinely unbalanced bracket in a *string* is fine, one in code is
        // not, and it is nearly always the table missing a construct again.
        let (ctx, env) = editor();
        for (name, source) in [
            ("editor/mod.rs", include_str!("../editor/mod.rs")),
            ("modes.rs", include_str!("../primitives/modes.rs")),
            ("sexp.rs", include_str!("../modes/sexp.rs")),
            ("shell.rs", include_str!("../primitives/shell.rs")),
        ] {
            rust_buffer_from_source(name, source, &env, &ctx);
            assert_eq!(
                depth_at_end(&env, &ctx),
                0.0,
                "core/src/{name} balances, and the scanner should say so"
            );
        }
    }
}
