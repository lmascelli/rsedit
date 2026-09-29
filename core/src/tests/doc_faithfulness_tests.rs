//! The editor's account of itself, checked against the editor.
//!
//! # Why this file exists
//!
//! Everything the help commands show is *written down* somewhere: a
//! docstring beside a primitive, a `defvar`'s third form, the specs a command
//! registered with. Written-down things drift. A primitive is renamed and its
//! docstring goes on saying the old name; a copied docstring keeps naming the
//! function it was copied from; a key is bound to a command that was never
//! defined, or was renamed, and the key silently does nothing until somebody
//! presses it.
//!
//! None of that fails a test anywhere else, and none of it looks wrong when
//! read: a help page is believed precisely because it is confident. So the
//! rules below are the ones that can be *checked against the running editor*
//! rather than against another piece of prose, and they are checked over
//! every primitive, every module function, every command and every binding
//! there is, with all the shipped modules loaded.
//!
//! # What is deliberately not checked
//!
//! An `Example:` section. Sixty primitives have none, and for most of them --
//! `isearch-exit`, `region-end` -- an example would be a line of ceremony
//! rather than an explanation. More to the point, an example cannot be
//! *wrong* the way a signature naming another function is wrong, and this
//! file is about the claims that can be false.
//!
//! The wording, likewise. A docstring that is accurate and terse and one that
//! is accurate and long are both fine, and a test that had an opinion about
//! which would be a test somebody routes around.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// Every shipped module, in the order init.lisp loads them. The point of
    /// loading all of them is that the checks below then cover the modules'
    /// own functions, variables and bindings and not just the Rust ones.
    const SHIPPED: [(&str, &str); 18] = [
        ("commands", include_str!("../../lisp/commands.lisp")),
        ("debug", include_str!("../../lisp/debug.lisp")),
        (
            "common-keymaps",
            include_str!("../../lisp/common-keymaps.lisp"),
        ),
        ("indent", include_str!("../../lisp/indent.lisp")),
        ("minibuffer", include_str!("../../lisp/minibuffer.lisp")),
        ("rust-mode", include_str!("../../lisp/rust-mode.lisp")),
        ("risp-mode", include_str!("../../lisp/risp-mode.lisp")),
        ("cc-mode", include_str!("../../lisp/cc-mode.lisp")),
        ("dired", include_str!("../../lisp/dired.lisp")),
        ("completion", include_str!("../../lisp/completion.lisp")),
        ("clipboard", include_str!("../../lisp/clipboard.lisp")),
        (
            "electric-pair",
            include_str!("../../lisp/electric-pair.lisp"),
        ),
        ("buffer-list", include_str!("../../lisp/buffer-list.lisp")),
        ("shell", include_str!("../../lisp/shell.lisp")),
        ("manpage", include_str!("../../lisp/manpage.lisp")),
        ("help", include_str!("../../lisp/help.lisp")),
        ("occur", include_str!("../../lisp/occur.lisp")),
        ("compile", include_str!("../../lisp/compile.lisp")),
    ];

    fn loaded() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for (name, source) in SHIPPED {
            let ast = Parser::new(&format!("(progn {source})"))
                .next()
                .unwrap_or_else(|why| panic!("{name}.lisp must parse: {why:?}"));
            eval(&ast, env.clone(), &ctx)
                .unwrap_or_else(|why| panic!("loading {name}.lisp: {why:?}"));
        }
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
    /// Told apart from Rust, which is the only place the difference is
    /// visible: `function-arglist` answers nil both for a primitive and for a
    /// Lisp function of no arguments, and the rules below differ between the
    /// two.
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
        // A primitive is a Rust function: it has no parameter list anything
        // can read, so the first line of its documentation *is* its
        // signature -- `help--signature` in help.lisp takes it from there
        // and shows it as the call. A first line naming another function
        // therefore shows somebody the wrong call, confidently, on the page
        // they opened to find out what the right one was.
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
        // Including the internal ones. `apropos` lists them, `describe-
        // function` will show them, and a module's own reader is the person
        // most likely to need them explained.
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
        // The modules follow the other convention -- a sentence about what
        // the function answers, with the parameters named in it -- so today
        // this passes over an empty set. It is here for the day somebody
        // writes `(foo A B)` at the top of a docstring and the function later
        // grows a third parameter: that is the drift this can catch and
        // nothing else can, since the two are never read together.
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
        // A command is what `M-x` offers and what a key runs. An undocumented
        // one is a name in a completion list with nothing behind it.
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
        // `register-command` records specs against a *name*; nothing checks
        // that the name has a function, because the two can legitimately be
        // written in either order. A name that never gets one is an `M-x`
        // entry that offers itself, completes, and then fails.
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
        // A key bound to a misspelt command is the quietest failure in the
        // editor: nothing complains at load time, `describe-bindings` lists
        // it, and it does nothing at all the day somebody presses it. The
        // keymaps are readable now, so this can be asked.
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
        // Read out of the sources rather than from `all-variables`, which
        // cannot tell a module's own variable from one the editor set in
        // Rust or one a test left lying about. What is being checked is that
        // the modules keep documenting what they define -- which they all do
        // today, and did silently and uselessly for months before `defvar`
        // learned to keep a docstring at all.
        let (ctx, env) = loaded();
        let mut defined = Vec::new();
        for (module, source) in SHIPPED {
            for line in source.lines() {
                let line = line.trim_start();
                for opener in ["(defvar ", "(defconst "] {
                    if let Some(rest) = line.strip_prefix(opener) {
                        let name: String = rest
                            .chars()
                            .take_while(|c| !c.is_whitespace() && *c != ')')
                            .collect();
                        if !name.is_empty() {
                            defined.push((module, name));
                        }
                    }
                }
            }
        }
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
}
