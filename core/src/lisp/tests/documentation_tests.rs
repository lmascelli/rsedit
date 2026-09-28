//! What a symbol says about itself: the docstring `defvar` keeps.
//!
//! # What is worth testing here
//!
//! A docstring is storage with no reader in the interpreter -- nothing
//! evaluates differently for having one -- so every way of losing it is
//! silent. That is the whole risk, and it has already happened once: until
//! this landed, `defvar`'s third form was parsed, ignored, and thrown away,
//! and twenty-eight docstrings in the shipped modules had been written into
//! nothing without a single failure to show for it.
//!
//! So the tests below pin the three places it can go quietly wrong: the
//! *rules* (stored literally, stored on re-evaluation even where the value is
//! not, refused rather than dropped when it is not a string), the *scope* (on
//! the symbol, not the binding -- a `let` cannot shadow it), and the *reader*
//! (`variable-doc` distinguishing documented, bound-but-undocumented, and
//! neither).
#[cfg(test)]
mod tests {
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval, setup_base_env};
    use std::sync::Arc;

    fn env_with_primitives() -> Arc<Env<()>> {
        let env = Env::new_root();
        setup_base_env(env.clone());
        env
    }

    fn eval_str(source: &str, env: Arc<Env<()>>) -> Result<LispExp<()>, EvalError<()>> {
        let wrapped = format!("(progn {source})");
        let ast = Parser::new(&wrapped)
            .next()
            .expect("failed to parse test script");
        eval(&ast, env, &())
    }

    fn eval_in(source: &str, env: Arc<Env<()>>) -> LispExp<()> {
        eval_str(source, env).unwrap_or_else(|why| panic!("eval of `{source}` failed: {why:?}"))
    }

    fn eval_ok(source: &str) -> LispExp<()> {
        eval_in(source, env_with_primitives())
    }

    fn text(exp: &LispExp<()>) -> String {
        match exp {
            LispExp::String(text) => text.to_string(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    // ----------------------------------------------------------------
    // The rules
    // ----------------------------------------------------------------

    #[test]
    fn a_defvar_docstring_can_be_read_back() {
        let doc = eval_ok(
            r#"(defvar fill-column 70 "Where lines are wrapped.")
               (variable-doc 'fill-column)"#,
        );
        assert_eq!(text(&doc), "Where lines are wrapped.");
    }

    #[test]
    fn defconst_documents_the_same_way() {
        let doc = eval_ok(
            r#"(defconst pi-ish 3.14 "Close enough for a test.")
               (variable-doc 'pi-ish)"#,
        );
        assert_eq!(text(&doc), "Close enough for a test.");
    }

    #[test]
    fn a_docstring_is_taken_literally_and_never_evaluated() {
        // The same rule `defun` follows. A form here is a mistake, not a
        // computation -- and if it were evaluated, a docstring could have
        // side effects, which is not a thing documentation should have.
        let env = env_with_primitives();
        let failed = eval_str(r#"(defvar x 1 (concat "a" "b"))"#, env.clone());
        assert!(
            matches!(failed, Err(EvalError::DefvarDocMustBeAString)),
            "a computed docstring should be refused, got {failed:?}"
        );
        // And nothing was stored on the way to refusing it: no property,
        // and no value either -- the variable is still unbound, so the form
        // did not take effect half-way.
        assert_eq!(
            eval_in("(get 'x 'variable-documentation)", env.clone()),
            LispExp::nil()
        );
        assert!(
            matches!(eval_str("x", env), Err(EvalError::UnboundVariable(_))),
            "the refused form should have bound nothing"
        );
    }

    #[test]
    fn a_fourth_form_is_refused_rather_than_ignored() {
        let failed = eval_str(r#"(defvar x 1 "doc" surprise)"#, env_with_primitives());
        assert!(
            matches!(failed, Err(EvalError::WrongNumberOfArguments { .. })),
            "a fourth form should be refused, got {failed:?}"
        );
    }

    #[test]
    fn a_variable_can_be_documented_before_it_has_a_value() {
        // `(defvar x)` binds nothing at all, which is its documented
        // behaviour. Documentation is about the name, so it is still
        // readable -- answering "Undocumented" here would be answering
        // "is it bound", which is a different question.
        let env = env_with_primitives();
        eval_in(
            r#"(defvar later nil "Set by whatever runs first.")"#,
            env.clone(),
        );
        assert_eq!(
            text(&eval_in("(variable-doc 'later)", env.clone())),
            "Set by whatever runs first."
        );
    }

    // ----------------------------------------------------------------
    // Re-evaluation: the value rule and the docstring rule differ
    // ----------------------------------------------------------------

    #[test]
    fn re_evaluating_a_defvar_updates_the_docstring_but_not_the_value() {
        // This is the one asymmetry worth having, and the reason for it is
        // in the arm itself: the value rule protects a setting the user has
        // changed since, and a docstring is not a setting. Editing one and
        // reloading the file has to show the new text or the feature is
        // worse than useless -- it would be confidently wrong.
        let env = env_with_primitives();
        eval_in(
            r#"(defvar tab-width 8 "The old description.")"#,
            env.clone(),
        );
        eval_in("(setq tab-width 4)", env.clone());
        eval_in(
            r#"(defvar tab-width 8 "The new description.")"#,
            env.clone(),
        );
        assert_eq!(eval_in("tab-width", env.clone()), LispExp::number(4.0));
        assert_eq!(
            text(&eval_in("(variable-doc 'tab-width)", env)),
            "The new description."
        );
    }

    #[test]
    fn a_docstring_can_be_taken_away_by_leaving_it_out() {
        // Not a rule anybody will lean on, but the answer has to be *some*
        // definite thing: a re-evaluation with no docstring leaves the old
        // one standing, because it says nothing about the documentation
        // rather than saying the documentation is gone.
        let env = env_with_primitives();
        eval_in(r#"(defvar kept 1 "Still true.")"#, env.clone());
        eval_in("(defvar kept 1)", env.clone());
        assert_eq!(text(&eval_in("(variable-doc 'kept)", env)), "Still true.");
    }

    // ----------------------------------------------------------------
    // Scope: on the symbol, not on the binding
    // ----------------------------------------------------------------

    #[test]
    fn a_let_shadows_the_value_and_not_the_documentation() {
        let env = env_with_primitives();
        eval_in(r#"(defvar depth 1 "How deep.")"#, env.clone());
        let inside = eval_in(
            "(let ((depth 99)) (list depth (variable-doc 'depth)))",
            env.clone(),
        );
        let LispExp::Cons(cell) = &inside else {
            panic!("expected a list, got {inside:?}");
        };
        assert_eq!(cell.car, LispExp::number(99.0));
        assert_eq!(text(&eval_in("(variable-doc 'depth)", env)), "How deep.");
    }

    #[test]
    fn a_docstring_written_inside_a_let_outlives_it() {
        // Falls out of the property table being unscoped, and is worth
        // pinning because it is the visible half of "a property belongs to
        // the symbol and a symbol is not a binding".
        let env = env_with_primitives();
        eval_in(
            r#"(let ((ignored 1)) (defvar escaped 2 "Made inside."))"#,
            env.clone(),
        );
        assert_eq!(
            text(&eval_in("(variable-doc 'escaped)", env)),
            "Made inside."
        );
    }

    // ----------------------------------------------------------------
    // The reader
    // ----------------------------------------------------------------

    #[test]
    fn a_bound_variable_with_no_docstring_says_so() {
        let env = env_with_primitives();
        eval_in("(setq plain 1)", env.clone());
        assert_eq!(
            text(&eval_in("(variable-doc 'plain)", env)),
            "Undocumented variable"
        );
    }

    #[test]
    fn a_name_that_is_neither_bound_nor_documented_reads_as_nil() {
        assert_eq!(eval_ok("(variable-doc 'nothing-at-all)"), LispExp::nil());
    }

    #[test]
    fn the_docstring_is_an_ordinary_property() {
        // Not an alias anybody needs, but the storage is deliberately the
        // property table rather than a new one, and a reader that could not
        // see it through `get` would mean it had quietly become something
        // else.
        let doc = eval_ok(
            r#"(defvar shared 1 "Under a property.")
               (get 'shared 'variable-documentation)"#,
        );
        assert_eq!(text(&doc), "Under a property.");
    }

    #[test]
    fn documenting_a_variable_does_not_document_the_function_of_the_same_name() {
        // Two name spaces, and the help commands built on these will ask
        // about a symbol that is often both.
        let env = env_with_primitives();
        eval_in(r#"(defvar both 1 "The variable.")"#, env.clone());
        eval_in(r#"(defun both () "The function." 1)"#, env.clone());
        assert_eq!(
            text(&eval_in("(variable-doc 'both)", env.clone())),
            "The variable."
        );
        assert_eq!(text(&eval_in("(function-doc 'both)", env)), "The function.");
    }

    #[test]
    fn variable_doc_wants_a_symbol() {
        let failed = eval_str(r#"(variable-doc "fill-column")"#, env_with_primitives());
        assert!(
            matches!(failed, Err(EvalError::WrongArgumentType { .. })),
            "a string should be refused, got {failed:?}"
        );
    }
}
