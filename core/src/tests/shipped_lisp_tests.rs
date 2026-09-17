//! Every shipped Lisp module, loaded in the order the default init.lisp loads
//! them.
//!
//! # Why this is worth a test of its own
//!
//! In the running editor these arrive through `eval-file`, which *logs* a
//! failure rather than raising one -- deliberately, so that one broken module
//! cannot stop the editor coming up. The cost of that is silence: a module with
//! a parse error, or one that calls something the module above it was supposed
//! to define, simply does not exist, and the only sign is a line in the log
//! nobody reads. The feature is just missing.
//!
//! So this loads them the way init.lisp does and insists on the errors.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// The modules `create_global_env` writes into a fresh init.lisp, in order.
    /// Order is the whole point: each is written against what the ones above it
    /// define.
    const SHIPPED: [(&str, &str); 11] = [
        ("commands", include_str!("../../lisp/commands.lisp")),
        ("debug", include_str!("../../lisp/debug.lisp")),
        ("common-keymaps", include_str!("../../lisp/common-keymaps.lisp")),
        ("indent", include_str!("../../lisp/indent.lisp")),
        ("minibuffer", include_str!("../../lisp/minibuffer.lisp")),
        ("rust-mode", include_str!("../../lisp/rust-mode.lisp")),
        ("dired", include_str!("../../lisp/dired.lisp")),
        ("completion", include_str!("../../lisp/completion.lisp")),
        ("clipboard", include_str!("../../lisp/clipboard.lisp")),
        ("electric-pair", include_str!("../../lisp/electric-pair.lisp")),
        (
            "find-file-recursive",
            include_str!("../../lisp/find-file-recursive.lisp"),
        ),
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

    #[test]
    fn every_shipped_module_loads_in_init_order() {
        loaded();
    }

    #[test]
    fn the_commands_the_new_modules_define_are_registered() {
        let (ctx, env) = loaded();
        for name in [
            "find-file-recursive",
            "electric-pair-delete-backward",
            "insert-pasted-text",
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
