//! Every shipped Lisp module, loaded in the order the default init.lisp loads
//! them, with the errors that `require' would only log.
//!
//! Design: `doc/architettura-editor.typ, <test-documentazione>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, MODULE_KINDS, create_global_env};
    use crate::tests::shipped::{default_modules, load_default_modules};
    use risp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn loaded() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        load_default_modules(&env, &ctx);
        (ctx, env)
    }

    #[test]
    fn every_shipped_module_loads_in_init_order() {
        loaded();
    }

    #[test]
    fn every_module_the_default_init_loads_is_beside_the_binary() {
        // The editor finds its modules where build.rs copies them, beside the
        // binary, each kind in its own directory.
        let (ctx, env) = loaded();
        let LispExp::String(dir) = eval(
            &Parser::new("(data-directory \"lisp\")")
                .next()
                .expect("must parse"),
            env.clone(),
            &ctx,
        )
        .expect("data-directory must not fail") else {
            panic!("the data directory beside the test binary is missing");
        };
        let modules = default_modules();
        assert!(modules.contains(&"preview".to_string()), "{modules:?}");
        let missing: Vec<&String> = modules
            .iter()
            .filter(|name| {
                !MODULE_KINDS.iter().any(|kind| {
                    std::path::Path::new(&*dir)
                        .join(kind)
                        .join(format!("{name}.lisp"))
                        .is_file()
                })
            })
            .collect();
        assert!(
            missing.is_empty(),
            "loaded by init.lisp, not in {dir}: {missing:?}"
        );
    }

    #[test]
    fn the_commands_the_new_modules_define_are_registered() {
        let (ctx, env) = loaded();
        for name in [
            "electric-pair-delete-backward",
            "insert-pasted-text",
            "clipboard-sync-toggle",
        ] {
            let ast = Parser::new(&format!("(commandp '{name})"))
                .next()
                .expect("source must parse");
            let answer = eval(&ast, env.clone(), &ctx).expect("commandp must not fail");
            assert!(!answer.is_nil(), "{name} should be a command");
        }
    }

    #[test]
    fn loading_everything_leaves_the_clipboard_on() {
        let (_ctx, env) = loaded();
        let sync = env
            .get_variable("clipboard-sync")
            .expect("clipboard-sync should be set by clipboard.lisp");
        assert!(sync.is_truthy());
    }

    #[test]
    fn the_docstrings_the_modules_already_carry_are_readable() {
        // Module variables from three files, read back with their docstrings.
        let (ctx, env) = loaded();
        for (name, expected_start) in [
            ("manpage-buffer-name", "The one buffer pages are shown in"),
            (
                "buffer-list-buffer-name",
                "The buffer the listing is shown in",
            ),
            ("compilation-window-height", "How tall the window showing"),
        ] {
            let ast = Parser::new(&format!("(variable-doc '{name})"))
                .next()
                .expect("source must parse");
            let answer = eval(&ast, env.clone(), &ctx).expect("variable-doc must not fail");
            let LispExp::String(doc) = &answer else {
                panic!("{name} should have a docstring, got {answer:?}");
            };
            assert!(
                doc.starts_with(expected_start),
                "{name}'s documentation should be its own: {doc:?}"
            );
        }
    }

    #[test]
    fn rust_mode_asks_for_its_quotes_to_be_left_alone() {
        // Set in electric-pair.lisp, read through the symbol property table.
        // Rust's lifetimes -- `'a` -- are why: pairing that quote turns every
        // generic parameter into `''a`.
        let (ctx, env) = loaded();
        let ast = Parser::new("(get 'rust-mode 'electric-pair-inhibit-quotes)")
            .next()
            .expect("source must parse");
        let answer = eval(&ast, env.clone(), &ctx).expect("get must not fail");
        assert!(!answer.is_nil());
    }
}
