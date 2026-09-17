//! `risp-mode.lisp`: the mode for the editor's own Lisp.
//!
//! The claim worth testing is that its vocabulary comes from the interpreter
//! rather than from a list in the file. A mode that merely *had* the right
//! words on the day it was written would pass a test that checked `defun` is a
//! keyword; what distinguishes this one is that a function defined afterwards
//! can be picked up, and that the words it knows are the words this editor
//! actually has.
#[cfg(test)]
mod tests {
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

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/indent.lisp"),
            include_str!("../../lisp/risp-mode.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// A buffer in risp-mode holding TEXT, with point at OFFSET.
    fn risp_buffer(text: &str, at: usize, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            r#"(buffer-create "code.lisp" 'risp-mode) (switch-to-buffer "code.lisp")"#,
            env,
            ctx,
        );
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        run(&format!(r#"(insert "{escaped}") (goto-char {at})"#), env, ctx);
    }

    fn contents(ctx: &Ctx) -> String {
        ctx.get_current_buffer()
            .read()
            .expect("read lock")
            .text
            .to_string()
    }

    // ----------------------------------------------------------------
    // The vocabulary
    // ----------------------------------------------------------------

    #[test]
    fn the_special_forms_the_mode_lists_are_the_ones_the_evaluator_has() {
        // The list in the file is the only hand-written part of the
        // vocabulary, and it exists because these have no binding to
        // enumerate. If one of them could be looked up, it should not be here.
        let (ctx, env) = editor();
        for name in ["if", "let", "lambda", "cond", "while"] {
            let looked_up = run(&format!("(functionp '{name})"), &env, &ctx);
            assert!(
                looked_up.is_nil(),
                "{name} resolves as a function, so it is not a special form"
            );
        }
    }

    #[test]
    fn the_vocabulary_offered_for_completion_is_not_empty_and_has_the_forms() {
        let (ctx, env) = editor();
        let words: Vec<String> = run("(get 'risp-mode 'keywords)", &env, &ctx)
            .iter()
            .map(|item| match item {
                LispExp::String(s) => s.to_string(),
                other => panic!("expected a word, got {other:?}"),
            })
            .collect();
        for expected in ["defun", "let", "lambda", "nil", "t"] {
            assert!(words.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn refreshing_the_vocabulary_picks_up_a_macro_defined_since() {
        // The point of asking the interpreter rather than listing: a macro
        // added after the mode loaded can be known to it.
        let (ctx, env) = editor();
        let before: Vec<String> = run("(get 'risp-mode 'keywords)", &env, &ctx)
            .iter()
            .map(|item| match item {
                LispExp::String(s) => s.to_string(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert!(!before.contains(&"with-thing".to_string()));

        run(
            "(defmacro with-thing (body) body) (risp-refresh-vocabulary)",
            &env,
            &ctx,
        );

        let after: Vec<String> = run("(get 'risp-mode 'keywords)", &env, &ctx)
            .iter()
            .map(|item| match item {
                LispExp::String(s) => s.to_string(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert!(after.contains(&"with-thing".to_string()), "got {after:?}");
    }

    #[test]
    fn a_name_ending_in_punctuation_is_matched_whole() {
        // Why the pattern spells out its own bounds instead of using `\b`: a
        // Lisp name may end in a character the engine does not call a word
        // character, and `\b` after one of those matches in the wrong place.
        let (ctx, env) = editor();
        let pattern = run(r#"(risp--word-pattern '("1+" "string<"))"#, &env, &ctx);
        let LispExp::String(pattern) = pattern else {
            panic!("expected a pattern");
        };
        let compiled = regex::Regex::new(&pattern).expect("the pattern must compile");
        assert!(compiled.is_match("(1+ x)"), "should match `1+` in a call");
        assert!(
            compiled.is_match("(string< a b)"),
            "should match `string<`"
        );
    }

    // ----------------------------------------------------------------
    // Structure
    // ----------------------------------------------------------------

    #[test]
    fn the_scanner_sees_a_list_in_a_risp_buffer() {
        let (ctx, env) = editor();
        risp_buffer("(foo (bar baz))", 6, &env, &ctx);
        let state = run("(syntax-ppss)", &env, &ctx);
        let depth = state.iter().next().expect("a depth");
        assert_eq!(depth, LispExp::number(2.0), "two lists deep at offset 6");
    }

    #[test]
    fn a_semicolon_starts_a_comment() {
        let (ctx, env) = editor();
        risp_buffer("(foo) ; a note\n(bar)", 10, &env, &ctx);
        let state = run("(syntax-ppss)", &env, &ctx);
        let comment = state.iter().nth(3).expect("the comment element");
        assert!(!comment.is_nil(), "point should be inside a comment");
    }

    #[test]
    fn square_brackets_pair_too() {
        let (ctx, env) = editor();
        risp_buffer("[a b]", 2, &env, &ctx);
        assert_eq!(
            run("(syntax-ppss)", &env, &ctx)
                .iter()
                .next()
                .expect("a depth"),
            LispExp::number(1.0)
        );
    }

    // ----------------------------------------------------------------
    // Indentation -- the rule that shipped with no mode to attach to
    // ----------------------------------------------------------------

    #[test]
    fn the_indenter_is_attached_to_the_mode() {
        let (ctx, env) = editor();
        assert_eq!(
            run("(get 'risp-mode 'indent-function)", &env, &ctx),
            LispExp::symbol("lisp-indent-line".to_string())
        );
    }

    #[test]
    fn a_body_form_indents_by_two() {
        let (ctx, env) = editor();
        risp_buffer("(defun f ()\nbody)", 12, &env, &ctx);
        run("(indent-line)", &env, &ctx);
        assert_eq!(contents(&ctx), "(defun f ()\n  body)");
    }

    #[test]
    fn a_call_lines_up_under_its_first_argument() {
        let (ctx, env) = editor();
        risp_buffer("(foo bar\nbaz)", 9, &env, &ctx);
        run("(indent-line)", &env, &ctx);
        assert_eq!(contents(&ctx), "(foo bar\n     baz)");
    }

    #[test]
    fn a_top_level_form_starts_at_the_left_margin() {
        let (ctx, env) = editor();
        risp_buffer("(one)\n    (two)", 6, &env, &ctx);
        run("(indent-line)", &env, &ctx);
        assert_eq!(contents(&ctx), "(one)\n(two)");
    }
}
