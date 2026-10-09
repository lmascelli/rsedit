//! The shapes the primitives share, tested where they are shared.
//!
//! `lisp/base/` generates its type predicates, its one-argument number and
//! string operations, its folds and its non-local exits from six macros, and
//! every fixed-arity primitive in the editor checks its arity through one
//! function. That is a hundred-odd primitives resting on seven pieces of code,
//! and these are the tests of those seven pieces -- from Lisp, which is the
//! only side any of it is visible from.
//!
//! # Why this file exists
//!
//! The six-line arity guard at the top of a hundred primitives was replaced by
//! one call to `risp::exact_arity`, and the replacement was *ablated* to check
//! the suite would notice if it went wrong: `!=` was changed to `<`, so that
//! every one of those primitives would accept any number of surplus arguments.
//! All 1929 tests passed.
//!
//! So the guard had never been tested from the Lisp side at all. Plenty of
//! tests call a primitive with too *few* arguments -- that is the shape a
//! half-written expression has -- and none had ever passed a surplus one,
//! which is the shape a *renamed* or *reordered* primitive is called with. A
//! check nothing tests is a check that can be deleted by accident, and
//! collapsing a hundred copies of it into one place makes that one accident
//! reach a hundred primitives.
#[cfg(test)]
mod tests {
    use crate::{
        buffer::gap_buffer::GapBuffer,
        editor::create_global_env,
    };
    use risp::{EvalError, Parser, eval};

    /// Evaluate SOURCE in a fresh editor and say what went wrong, if anything.
    fn outcome(source: &str) -> Result<(), String> {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let form = Parser::new(source).next().expect("source must parse");
        match eval(&form, env.clone(), &state) {
            Ok(_) => Ok(()),
            Err(EvalError::WrongNumberOfArguments { expected, got }) => {
                Err(format!("expected {expected}, got {got}"))
            }
            Err(other) => Err(format!("{other:?}")),
        }
    }

    /// One surplus argument is an error, for every arity this covers.
    ///
    /// Spread across the arities and the kinds of primitive deliberately:
    /// one-argument and two-argument, a predicate, an accessor, an arithmetic
    /// operation, a string operation and a non-local exit, so that a guard
    /// lost from any one of the shapes the macros in `lisp/base/mod.rs` generate is
    /// a failure here.
    #[test]
    fn a_surplus_argument_is_refused() {
        for source in [
            "(stringp \"a\" \"b\")",
            "(numberp 1 2)",
            "(symbolp 'a 'b)",
            "(vectorp [1] [2])",
            "(consp '(1) '(2))",
            "(atom 1 2)",
            "(car '(1 2) 'surplus)",
            "(cdr '(1 2) 'surplus)",
            "(abs -1 2)",
            "(floor 1.5 2)",
            "(upcase \"a\" \"b\")",
            "(downcase \"A\" \"B\")",
            "(not nil 'surplus)",
            "(cons 1 2 3)",
            "(throw 'tag 'value 'surplus)",
            "(signal 'my-error '(1) 'surplus)",
        ] {
            let result = outcome(source);
            assert!(
                result.is_err(),
                "{source} was accepted with a surplus argument"
            );
        }
    }

    /// And a missing one still is, which is the half that was already covered
    /// -- kept here so the two live together and neither can be deleted
    /// without the other being noticed.
    #[test]
    fn a_missing_argument_is_refused() {
        for source in [
            "(stringp)",
            "(numberp)",
            "(car)",
            "(abs)",
            "(upcase)",
            "(cons 1)",
            "(throw 'tag)",
            "(signal 'my-error)",
        ] {
            assert!(
                outcome(source).is_err(),
                "{source} was accepted with an argument missing"
            );
        }
    }

    /// The variadic ones are the other half of the rule: they take any number
    /// from one upwards, so a surplus argument is not surplus and only *none*
    /// is an error.
    #[test]
    fn a_variadic_primitive_wants_at_least_one_argument() {
        for source in ["(max)", "(min)", "(/)"] {
            assert!(
                outcome(source).is_err(),
                "{source} was accepted with no arguments at all"
            );
        }
        for source in ["(max 1)", "(max 1 2 3 4 5)", "(min 1 2 3)"] {
            assert_eq!(
                outcome(source),
                Ok(()),
                "{source} should be fine -- these take any number of arguments"
            );
        }
    }
}

/// That a number read out of Lisp cannot crash the editor or stop a timer.
///
/// # Why these live beside the arity tests
///
/// Same story, same ablation. The seven hand-written readers of a numeric
/// setting became one call to `Env::number_at_least`, and removing its
/// `is_finite` guard broke no test -- although `(setq auto-save-interval (*
/// 1e308 10))` is then a one-line program that panics the editor, because
/// `Duration::from_secs_f64` is documented to panic on a non-finite number.
///
/// Infinity is a number, and `>= 1.0` is true of it. Only `is_finite` is not,
/// which is exactly why it is the guard that gets left out. Division by zero
/// is refused by this Lisp, so the way to reach an infinity is to overflow
/// one, which nothing refuses -- `f64` arithmetic is not checked and should
/// not be.
#[cfg(test)]
mod setting_tests {
    use crate::{
        buffer::gap_buffer::GapBuffer,
        editor::{EditorState, create_global_env},
        modes::autosave::{AUTO_SAVE_INTERVAL, auto_save_interval},
        modes::watcher::{WATCH_FILE_INTERVAL, watch_interval},
    };
    use risp::{Env, Parser, eval};
    use std::sync::Arc;

    /// A fresh editor with NAME bound to whatever SOURCE evaluates to.
    ///
    /// Bound at the root, as a `setq` at top level would, because a setting
    /// read on a later turn must outlive the form that set it.
    fn with_setting(name: &str, source: &str) -> Arc<Env<EditorState<GapBuffer>>> {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let form = Parser::new(source).next().expect("source must parse");
        let value = eval(&form, env.clone(), &state).expect("source must evaluate");
        env.set_root_variable(name.to_string(), value);
        env
    }

    /// `1` followed by three hundred and nine zeros: larger than an `f64` can
    /// hold, so the reader's own `parse` hands back infinity.
    ///
    /// Spelled out rather than written `1e309` because this reader has no
    /// exponent notation -- `1e309` is read as a *symbol*, and a test using it
    /// fails with `UnboundVariable` while appearing to be about durations.
    fn too_large_for_a_float() -> String {
        format!("1{}", "0".repeat(309))
    }

    /// Infinity, negative infinity and NaN are all numbers and none of them is
    /// a duration. Each has to fall back rather than reach
    /// `Duration::from_secs_f64`, which panics on every one of them -- so a
    /// regression here is not a wrong interval, it is the editor exiting.
    #[test]
    fn a_non_finite_interval_falls_back_to_the_default() {
        let huge = too_large_for_a_float();
        for source in [
            huge.clone(),
            format!("(- 0 {huge})"),
            format!("(- {huge} {huge})"),
        ] {
            let source = source.as_str();
            let interval = auto_save_interval(&with_setting(AUTO_SAVE_INTERVAL, source));
            assert!(
                interval.as_secs() >= 1,
                "auto-save-interval of {source} gave {interval:?}"
            );

            let interval = watch_interval(&with_setting(WATCH_FILE_INTERVAL, source));
            assert!(
                interval.as_secs_f64() >= 0.1,
                "watch-file-interval of {source} gave {interval:?}"
            );
        }
    }

    /// Below the floor is the other half. Each setting has a floor of its own
    /// and a value under it means the *default* rather than the floor: the two
    /// floors differ because the costs differ, and silently rounding a
    /// hundredth of a second up to one second would make the setting look as
    /// though it had been honoured.
    #[test]
    fn a_number_under_the_floor_falls_back_to_the_default() {
        let interval = auto_save_interval(&with_setting(AUTO_SAVE_INTERVAL, "0.01"));
        assert!(
            interval.as_secs() >= 1,
            "an auto-save every hundredth of a second is the crash, not the insurance"
        );

        let interval = watch_interval(&with_setting(WATCH_FILE_INTERVAL, "-5"));
        assert!(
            interval.as_secs_f64() >= 0.1,
            "a negative interval is not an interval, it is {interval:?}"
        );
    }

    /// A value of the wrong *kind* is the same question asked another way.
    #[test]
    fn a_setting_of_the_wrong_type_falls_back_to_the_default() {
        let interval = auto_save_interval(&with_setting(AUTO_SAVE_INTERVAL, "\"often\""));
        assert!(
            interval.as_secs() >= 1,
            "a string where a count belongs must leave the default in place"
        );
    }
}

/// That each generated primitive is about what its name says.
///
/// # Why a table
///
/// Changing `predicate!(primitive_symbolp, LispExp::Symbol(_))` to
/// `LispExp::String(_)` -- a one-token edit, and the sort a macro invites,
/// since the pattern is now the only thing written -- broke none of the
/// suite's 1935 tests. The predicates had never been called from Lisp at all.
///
/// A macro makes the distinguishing token small, and a small token is easy to
/// get wrong and impossible to notice. So the table below says, for each
/// generated primitive, one thing it must answer yes to and one thing it must
/// answer no to. Anything swapped between two of them fails twice.
#[cfg(test)]
mod shape_tests {
    use crate::{
        buffer::gap_buffer::GapBuffer,
        editor::create_global_env,
    };
    use risp::{LispExp, Parser, eval};

    type Value = LispExp<crate::editor::EditorState<GapBuffer>>;

    /// What SOURCE evaluates to.
    fn value_of(source: &str) -> Value {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let form = Parser::new(source).next().expect("source must parse");
        match eval(&form, env.clone(), &state) {
            Ok(value) => value,
            Err(why) => panic!("{source} failed: {why:?}"),
        }
    }

    /// Both sides of an expectation are written as Lisp and compared as values.
    ///
    /// Writing the expected side as Lisp too -- `"t"`, `"\"AB C\""`, `"5"` --
    /// keeps a table of twenty rows readable as a table, which the constructor
    /// calls it would otherwise hold (`LispExp::string("AB C".into())`) would
    /// not. Nothing is compared as text, so a change to how this Lisp *prints*
    /// cannot make these pass or fail.
    fn assert_evaluates_to(source: &str, expected: &str) {
        assert_eq!(
            value_of(source),
            value_of(expected),
            "{source} should give {expected}"
        );
    }

    #[test]
    fn each_predicate_answers_about_its_own_type() {
        for (source, expected) in [
            ("(stringp \"a\")", "t"),
            ("(stringp 'a)", "nil"),
            ("(stringp 1)", "nil"),
            ("(numberp 1)", "t"),
            ("(numberp \"1\")", "nil"),
            ("(numberp 'one)", "nil"),
            ("(symbolp 'a)", "t"),
            ("(symbolp \"a\")", "nil"),
            ("(symbolp 1)", "nil"),
            ("(vectorp [1 2])", "t"),
            ("(vectorp '(1 2))", "nil"),
            ("(vectorp \"12\")", "nil"),
            ("(consp '(1 2))", "t"),
            ("(consp 1)", "nil"),
            ("(consp nil)", "nil"),
            // `atom' is the one generated by inversion, so it is the one whose
            // answers must disagree with `consp' everywhere.
            ("(atom 1)", "t"),
            ("(atom nil)", "t"),
            ("(atom '(1 2))", "nil"),
        ] {
            assert_evaluates_to(source, expected);
        }
    }

    #[test]
    fn each_one_argument_operation_does_its_own_operation() {
        for (source, expected) in [
            ("(abs -3)", "3"),
            ("(abs 3)", "3"),
            ("(floor 3.7)", "3"),
            ("(floor -3.2)", "-4"),
            ("(upcase \"aB c\")", "\"AB C\""),
            ("(downcase \"aB C\")", "\"ab c\""),
        ] {
            assert_evaluates_to(source, expected);
        }
    }

    /// `max` and `min` share one generated fold, so they are the pair that
    /// would both still pass if the fold picked the wrong end -- unless both
    /// directions are asked for, which is what this does.
    #[test]
    fn each_fold_folds_its_own_way() {
        for (source, expected) in [
            ("(max 1)", "1"),
            ("(max 1 5 3)", "5"),
            ("(max -5 -1)", "-1"),
            ("(min 1)", "1"),
            ("(min 1 5 3)", "1"),
            ("(min -5 -1)", "-5"),
        ] {
            assert_evaluates_to(source, expected);
        }
    }

    /// `throw` and `signal` are generated from one shape and caught by
    /// different machinery, which is the whole reason they must not be swapped:
    /// a `condition-case' does not catch a throw, and a `catch' does not catch
    /// a signal.
    #[test]
    fn each_non_local_exit_is_caught_by_its_own_catcher() {
        assert_evaluates_to("(catch 'tag (throw 'tag 42))", "42");
        assert_evaluates_to(
            "(condition-case e (signal 'my-error '(1 2)) (my-error (cdr e)))",
            "'(1 2)",
        );
    }

    /// The three name listings read three separate namespaces, and a macro
    /// invocation naming the wrong one would give a plausible-looking list.
    ///
    /// Each name is *defined here* rather than looked for among the shipped
    /// ones: a bare global environment has no macros at all, so a test
    /// expecting to find `when` among them passes only because the editor
    /// happened to have loaded its Lisp, and fails for a reason that has
    /// nothing to do with namespaces.
    #[test]
    fn each_name_listing_reads_its_own_namespace() {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let mut parser = Parser::new(
            "(defun shape-test-fn () 1) \
             (defmacro shape-test-macro () 1) \
             (setq shape-test-var 1)",
        );
        while let Ok(form) = parser.next::<crate::editor::EditorState<GapBuffer>>() {
            if form.is_nil() {
                break;
            }
            eval(&form, env.clone(), &state).expect("definitions must evaluate");
        }

        let names = |source: &str| -> Vec<String> {
            let form = Parser::new(source).next().expect("source must parse");
            let value: Value = eval(&form, env.clone(), &state).expect("listing must evaluate");
            value
                .iter()
                .filter_map(|item| match item {
                    LispExp::String(name) => Some(name.to_string()),
                    _ => None,
                })
                .collect()
        };

        let functions = names("(all-functions)");
        let macros = names("(all-macros)");
        let variables = names("(all-variables)");

        for (namespace, listed, name) in [
            ("all-functions", &functions, "shape-test-fn"),
            ("all-macros", &macros, "shape-test-macro"),
            ("all-variables", &variables, "shape-test-var"),
        ] {
            assert!(
                listed.iter().any(|found| found == name),
                "{namespace} should list {name}: {listed:?}"
            );
            for (other_name, other) in [
                ("all-functions", &functions),
                ("all-macros", &macros),
                ("all-variables", &variables),
            ] {
                if other_name != namespace {
                    assert!(
                        !other.iter().any(|found| found == name),
                        "{other_name} should not list {name}, which is a {namespace} name"
                    );
                }
            }
        }
    }
}

/// That every primitives module is actually installed.
///
/// # Why a list of names and not a count
///
/// Each of the twenty-six modules under `primitives/` now registers its own
/// primitives, and `install_primitives` is the list of modules and nothing
/// else. That list is the one place a module can be *forgotten*: the code
/// compiles, every test of that module's own behaviour fails, and if the module
/// happens to be one whose behaviour is thinly tested, nothing fails at all.
///
/// Dropping each of four modules from the list was tried, and the suite caught
/// all four -- so the cover is real today. What it is not is *structural*: it
/// holds because those modules happen to be tested, which is a property of the
/// rest of the suite rather than of this arrangement. One name per module makes
/// it structural, and reads as the list of what the editor provides.
#[cfg(test)]
mod registration_tests {
    use crate::{buffer::gap_buffer::GapBuffer, editor::create_global_env};

    #[test]
    fn every_primitives_module_is_installed() {
        let (_state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let installed = env.function_names();

        for (module, primitive) in [
            ("ask", "y-or-n"),
            ("buffers", "buffer-put"),
            ("commands", "register-command"),
            ("comments", "comment-dwim"),
            ("completion", "completion-at-point"),
            ("edits", "delete-region"),
            ("general", "eval-file"),
            ("help", "key-binding"),
            ("io", "quit"),
            ("isearch", "isearch-forward"),
            ("macros", "kmacro-start-macro"),
            ("modes", "make-mode"),
            ("mouse", "mouse-set-point"),
            ("overlays", "make-overlay"),
            ("rectangle", "rectangle-mark-mode"),
            ("region", "set-mark"),
            ("replace", "query-replace"),
            ("results", "occur--scan"),
            ("scan", "scan-buffer"),
            ("shell", "shell-command-start"),
            ("theme", "set-face"),
            ("ui", "recenter"),
            ("verbs", "upcase-word"),
            ("virtual_text", "make-virtual-text"),
            ("windows", "split-window-below"),
            ("workers", "define-worker"),
        ] {
            assert!(
                installed.iter().any(|name| name == primitive),
                "`{primitive}` is missing, so primitives::{module}::install is \
                 not being called from install_primitives"
            );
        }
    }

    /// The same check for the interpreter's own ten modules.
    ///
    /// Separate from the one above because the two halves install separately --
    /// `setup_base_env` runs first and knows nothing of an editor,
    /// `install_primitives` second -- and a reader looking for `car` should be
    /// told it is `lisp/base/lists.rs` rather than hunting through
    /// `primitives/`.
    #[test]
    fn every_base_env_module_is_installed() {
        let (_state, env) = create_global_env::<GapBuffer>().expect("global env must build");
        let installed = env.function_names();

        for (module, primitive) in [
            ("atoms", "make-atom"),
            ("comparisons", "<"),
            ("fibers", "fiber-done-p"),
            ("functions", "funcall"),
            ("inspect", "boundp"),
            ("lists", "car"),
            ("math", "+"),
            ("predicates", "eq"),
            ("strings", "concat"),
            ("symbols", "put"),
        ] {
            assert!(
                installed.iter().any(|name| name == primitive),
                "`{primitive}` is missing, so risp::base::{module}::install is not being \
                 called from setup_base_env"
            );
        }
    }

    /// A command is a primitive *and* a registry entry, and the two halves are
    /// made by one call -- so a module registered as functions only, by a
    /// `Registry::function` where `Registry::command` was meant, would leave
    /// `M-x` unable to reach it while every direct Lisp call still worked.
    #[test]
    fn a_built_in_command_reaches_the_registry_as_well_as_the_namespace() {
        let (state, env) = create_global_env::<GapBuffer>().expect("global env must build");

        for command in [
            "quit",
            "recenter",
            "set-mark",
            "upcase-word",
            "comment-dwim",
            "isearch-forward",
            "query-replace",
            "split-window-below",
            "rectangle-mark-mode",
            "kmacro-start-macro",
        ] {
            assert!(
                env.function_names().iter().any(|name| name == command),
                "{command} is not bound as a function"
            );
            assert!(
                state.is_command(command),
                "{command} is bound but M-x cannot reach it -- registered with \
                 Registry::function where Registry::command was meant"
            );
        }
    }

    /// And the argument specs survive the move: a command that prompts for
    /// something must still say what.
    #[test]
    fn a_commands_argument_spec_is_registered_with_it() {
        let (state, _env) = create_global_env::<GapBuffer>().expect("global env must build");

        for (command, arguments) in [
            ("query-replace", 2),
            ("set-face", 3),
            ("string-rectangle", 1),
            ("kmacro-set-counter", 1),
        ] {
            let specs = state.command_specs(command).unwrap_or_default();
            assert_eq!(
                specs.len(),
                arguments,
                "{command} should collect {arguments} argument(s), not {}",
                specs.len()
            );
        }
    }
}
