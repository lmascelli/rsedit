//! That the editor holds no lock across a call into the interpreter: Lisp
//! re-enters the editor through any primitive, and a lock it needs again hangs
//! the thread, every time. Each case runs on a thread of its own with a
//! deadline, since a hang fails nothing and `cargo test` has no timeout.
//!
//! Design: `doc/architettura-editor.typ, <regole-compartimenti>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::create_global_env;
    use risp::{Parser, eval};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// Generous next to the milliseconds these take when they work, and short
    /// enough that a regression reports quickly.
    const DEADLINE: Duration = Duration::from_secs(10);

    /// Runs BODY on a thread of its own, and returns whether it finished in
    /// time. A thread that deadlocked is left parked: the process is about to
    /// end.
    fn completes(body: impl FnOnce() + Send + 'static) -> bool {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            body();
            // A send failure means the receiver already timed out and gave up,
            // which the deadline has by then reported.
            let _ = tx.send(());
        });
        rx.recv_timeout(DEADLINE).is_ok()
    }

    /// A `post-command-hook` that registers another hook: `add-hook` takes the
    /// modes' lock to write, which `run_hook` must have given back before
    /// evaluating.
    #[test]
    fn a_hook_may_register_another_hook() {
        assert!(
            completes(|| {
                let (ctx, env) = create_global_env::<GapBuffer>().expect("global env must build");
                let setup = r#"(progn
                    (make-mode 'probe-mode)
                    (defun probe-fn () (add-hook 'probe-mode "later-hook" 'probe-fn))
                    (add-hook 'probe-mode "post-command-hook" 'probe-fn))"#;
                let ast = Parser::new(setup).next().expect("setup must parse");
                eval(&ast, env.clone(), &ctx).expect("setup must evaluate");

                ctx.run_hook("probe-mode", "post-command-hook", &env);
            }),
            "running a hook that calls `add-hook` deadlocked -- a lock on the mode registry is \
             being held across `eval`"
        );
    }

    /// The same, through the other writers of the modes' lock: `define-key`
    /// with a mode argument, and `make-mode`. A mode that binds its keys on
    /// its first command is ordinary Lisp.
    #[test]
    fn a_hook_may_define_keys_and_modes() {
        assert!(
            completes(|| {
                let (ctx, env) = create_global_env::<GapBuffer>().expect("global env must build");
                let setup = r#"(progn
                    (make-mode 'probe-mode)
                    (defun probe-fn ()
                      (make-mode 'lazily-made-mode)
                      (define-key 'probe-mode "C-q" 'ignore))
                    (add-hook 'probe-mode "post-command-hook" 'probe-fn))"#;
                let ast = Parser::new(setup).next().expect("setup must parse");
                eval(&ast, env.clone(), &ctx).expect("setup must evaluate");

                ctx.run_hook("probe-mode", "post-command-hook", &env);
            }),
            "running a hook that calls `define-key`/`make-mode` deadlocked -- a lock on the \
             mode registry is being held across `eval`"
        );
    }
}
