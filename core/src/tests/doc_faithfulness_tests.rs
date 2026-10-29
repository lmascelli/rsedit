//! The editor's account of itself, checked against the running editor with
//! the shipped modules loaded: the docstrings of every primitive and module
//! function, the variables' documentation, the commands' specs and every
//! binding. Examples and wording are not checked.
//!
//! Design: `doc/architettura-editor.typ, <test-documentazione>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, SETTINGS, create_global_env};
    use crate::tests::shipped::{
        editor_functions, exceptions, known_names, load_default_modules, man_pages,
        module_variables, quoted_names,
    };
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// The editor with every module the default init.lisp loads.
    fn loaded() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        load_default_modules(&env, &ctx);
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn text(exp: &LispExp<Ctx>) -> String {
        match exp {
            LispExp::String(text) => text.to_string(),
            other => format!("{other:?}"),
        }
    }

    fn strings(exp: &LispExp<Ctx>) -> Vec<String> {
        exp.iter().map(|item| text(&item)).collect()
    }

    /// What a function's documentation says, or `None` when it has none.
    ///
    /// The two "no documentation" answers are what the readers hand back for
    /// a function that has none, and they are not documentation.
    fn documentation(name: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Option<String> {
        let doc = text(&run(&format!("(function-doc '{name})"), env, ctx));
        let missing = doc.trim().is_empty()
            || doc == "No documentation provided."
            || doc == "Undocumented function"
            || doc == "nil";
        (!missing).then_some(doc)
    }

    /// Whether the first line of DOC opens with `(NAME`, as a call.
    fn first_line_names(doc: &str, name: &str) -> bool {
        let first = doc.lines().next().unwrap_or("").trim();
        let opens = format!("({name}");
        first.starts_with(&opens)
            && first[opens.len()..]
                .chars()
                .next()
                .is_none_or(|next| next == ' ' || next == ')')
    }

    /// The functions bound after everything is loaded, split by what they are.
    ///
    /// Told apart here: `function-arglist` answers nil both for a primitive
    /// and for a Lisp function of no arguments.
    fn primitives_and_lambdas(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> (Vec<String>, Vec<String>) {
        let mut primitives = Vec::new();
        let mut lambdas = Vec::new();
        for name in strings(&run("(all-functions)", env, ctx)) {
            match env.get_function(&name) {
                Some(LispExp::Primitive { .. }) => primitives.push(name),
                Some(LispExp::Lambda(_)) => lambdas.push(name),
                _ => {}
            }
        }
        assert!(
            primitives.len() > 200 && lambdas.len() > 100,
            "the editor should come up with both kinds: {} primitives, {} functions",
            primitives.len(),
            lambdas.len()
        );
        (primitives, lambdas)
    }

    // ----------------------------------------------------------------
    // Functions
    // ----------------------------------------------------------------

    #[test]
    fn every_primitive_says_what_it_is_called() {
        // A primitive has no parameter list: the first line of its
        // documentation is the call a help page shows.
        let (ctx, env) = loaded();
        let (primitives, _) = primitives_and_lambdas(&env, &ctx);
        let mut wrong = Vec::new();
        for name in &primitives {
            let Some(doc) = documentation(name, &env, &ctx) else {
                wrong.push(format!("{name}: no documentation at all"));
                continue;
            };
            let first = doc.lines().next().unwrap_or("").trim().to_string();
            if !first_line_names(&doc, name) {
                wrong.push(format!("{name}: first line is {first:?}"));
            } else if !first.contains("): ") && !first.trim_end().ends_with("):") {
                // The colon is what separates the call from the sentence, and
                // what the signature is cut at when a page shows it.
                wrong.push(format!("{name}: no `):' after the call in {first:?}"));
            }
        }
        assert!(
            wrong.is_empty(),
            "a primitive's first line must be its own call, `(name ARGS): ...':\n  {}",
            wrong.join("\n  ")
        );
    }

    #[test]
    fn every_function_the_modules_define_is_documented() {
        // The internal ones too: `apropos` lists them.
        let (ctx, env) = loaded();
        let (_, lambdas) = primitives_and_lambdas(&env, &ctx);
        let undocumented: Vec<&String> = lambdas
            .iter()
            .filter(|name| documentation(name, &env, &ctx).is_none())
            .collect();
        assert!(
            undocumented.is_empty(),
            "these module functions have no docstring: {undocumented:?}"
        );
    }

    #[test]
    fn a_module_function_that_writes_a_signature_agrees_with_its_parameters() {
        // The modules write a sentence instead, so this passes over an empty
        // set: it is there for a `(foo A B)` that falls behind its function.
        let (ctx, env) = loaded();
        let (_, lambdas) = primitives_and_lambdas(&env, &ctx);
        let mut wrong = Vec::new();
        for name in &lambdas {
            let Some(doc) = documentation(name, &env, &ctx) else {
                continue;
            };
            if !first_line_names(&doc, name) {
                continue;
            }
            let first = doc.lines().next().unwrap_or("").trim().to_string();
            let declared = strings(&run(&format!("(function-arglist '{name})"), &env, &ctx));
            for parameter in &declared {
                if !first.contains(parameter.trim_start_matches('&')) {
                    wrong.push(format!(
                        "{name}: {first:?} does not mention {parameter:?} of {declared:?}"
                    ));
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "a written-out signature must match the parameter list:\n  {}",
            wrong.join("\n  ")
        );
    }

    // ----------------------------------------------------------------
    // Commands
    // ----------------------------------------------------------------

    #[test]
    fn every_command_is_documented() {
        // An undocumented command is a name in `M-x` with nothing behind it.
        let (ctx, env) = loaded();
        let undocumented: Vec<String> = strings(&run("(all-commands)", &env, &ctx))
            .into_iter()
            .filter(|name| documentation(name, &env, &ctx).is_none())
            .collect();
        assert!(
            undocumented.is_empty(),
            "these commands have no docstring: {undocumented:?}"
        );
    }

    #[test]
    fn every_command_names_a_function_that_exists() {
        // `register-command` records specs against a name, in either order with
        // the function; a name that never gets one fails when run.
        let (ctx, env) = loaded();
        let missing: Vec<String> = strings(&run("(all-commands)", &env, &ctx))
            .into_iter()
            .filter(|name| env.get_function(name).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "these commands are registered but define nothing: {missing:?}"
        );
    }

    // ----------------------------------------------------------------
    // Keys
    // ----------------------------------------------------------------

    #[test]
    fn every_binding_names_something_that_exists() {
        // A key bound to a misspelt command does nothing, and nothing says so.
        let (ctx, env) = loaded();
        let mut dead = Vec::new();
        for binding in run("(keymap-bindings)", &env, &ctx).iter() {
            let parts: Vec<LispExp<Ctx>> = binding.iter().collect();
            let keys = text(&parts[0]);
            let named = match &parts[1] {
                LispExp::Symbol(name) => Some(name.to_string()),
                other => match other.iter().next() {
                    Some(LispExp::Symbol(name)) => Some(name.to_string()),
                    _ => None,
                },
            };
            if let Some(name) = named
                && env.get_function(&name).is_none()
                && env.get_macro(&name).is_none()
            {
                dead.push(format!("{keys} is bound to {name}, which does not exist"));
            }
        }
        assert!(dead.is_empty(), "dead keys:\n  {}", dead.join("\n  "));
    }

    // ----------------------------------------------------------------
    // Variables
    // ----------------------------------------------------------------

    #[test]
    fn every_variable_the_modules_define_is_documented() {
        // Read out of the sources: `all-variables` cannot tell a module's own
        // variable from one set in Rust or by a test.
        let (ctx, env) = loaded();
        let defined = module_variables();
        assert!(
            defined.len() > 20,
            "the modules should define variables to check: {defined:?}"
        );
        let undocumented: Vec<String> = defined
            .into_iter()
            .filter(|(_, name)| {
                let doc = text(&run(&format!("(variable-doc '{name})"), &env, &ctx));
                doc == "Undocumented variable" || doc == "nil"
            })
            .map(|(module, name)| format!("{name} in {module}.lisp"))
            .collect();
        assert!(
            undocumented.is_empty(),
            "these module variables have no docstring: {undocumented:?}"
        );
    }

    // ----------------------------------------------------------------
    // What the documentation says
    // ----------------------------------------------------------------

    /// Every docstring of the editor's own, as (name, text): its functions,
    /// private ones included, and the variables its modules define.
    fn docstrings(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = editor_functions(env)
            .into_iter()
            .filter_map(|name| documentation(&name, env, ctx).map(|doc| (name, doc)))
            .collect();
        let settings = SETTINGS.iter().map(|setting| setting.name.to_string());
        for name in module_variables()
            .into_iter()
            .map(|(_, name)| name)
            .chain(settings)
        {
            let doc = text(&run(&format!("(variable-doc '{name})"), env, ctx));
            if doc != "Undocumented variable" && doc != "nil" {
                out.push((name, doc));
            }
        }
        out
    }

    #[test]
    fn every_setting_the_editor_reads_is_documented_and_has_a_value() {
        // So that `C-h v` on one says what it is for and what it is now,
        // rather than that it is not set and whatever reads it will fail.
        let (ctx, env) = loaded();
        assert!(SETTINGS.len() >= 5, "the settings are not registered");
        let mut wrong = Vec::new();
        for setting in SETTINGS {
            let doc = text(&run(
                &format!("(variable-doc '{})", setting.name),
                &env,
                &ctx,
            ));
            if doc != setting.doc {
                wrong.push(format!("{}: documented as {doc:?}", setting.name));
            }
            if env.get_variable(setting.name).is_none() {
                wrong.push(format!("{}: no value", setting.name));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// How long a docstring may be, in characters. What a longer one has to
    /// say belongs in a manual page, which `C-h f` points to.
    const DOCSTRING_LIMIT: usize = 1000;

    #[test]
    fn no_docstring_is_longer_than_the_limit() {
        let (ctx, env) = loaded();
        let docstrings = docstrings(&env, &ctx);
        let too_long: Vec<&str> = docstrings
            .iter()
            .filter(|(_, doc)| doc.chars().count() > DOCSTRING_LIMIT)
            .map(|(name, _)| name.as_str())
            .collect();
        assert!(
            too_long.is_empty(),
            "longer than {DOCSTRING_LIMIT} characters; move what they explain to a manual page:\n{}",
            too_long.join("\n")
        );
    }

    #[test]
    fn every_name_quoted_in_a_page_or_a_docstring_is_known() {
        // `name' says "look this up"; it must lead somewhere.
        let (ctx, env) = loaded();
        let known = known_names(&env, &ctx);
        let elsewhere = exceptions(include_str!("doc_exceptions/quoted_elsewhere.txt"));
        let mut quoted: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for (page, body) in man_pages() {
            for name in quoted_names(&body) {
                quoted.entry(name).or_default().push(format!("page {page}"));
            }
        }
        for (owner, doc) in docstrings(&env, &ctx) {
            for name in quoted_names(&doc) {
                quoted.entry(name).or_default().push(owner.clone());
            }
        }
        assert!(quoted.len() > 150, "too few quoted names: {}", quoted.len());
        let unlisted: Vec<String> = quoted
            .iter()
            .filter(|(name, _)| !known.contains(*name))
            .filter(|(name, _)| !elsewhere.contains(name))
            .map(|(name, by)| {
                let mut by = by.clone();
                by.dedup();
                format!("{name}  # {}", by.join(", "))
            })
            .collect();
        let stale: Vec<&String> = elsewhere
            .iter()
            .filter(|name| !quoted.contains_key(*name) || known.contains(*name))
            .collect();
        assert!(
            unlisted.is_empty(),
            "quoted, and not a name the editor has:\n{}",
            unlisted.join("\n")
        );
        assert!(
            stale.is_empty(),
            "doc_exceptions/quoted_elsewhere.txt lists these, which nothing quotes now or the \
             editor has; take them out:\n{}",
            stale
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
