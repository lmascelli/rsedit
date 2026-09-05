//! Guards against the editor deadlocking itself.
//!
//! # The rule these enforce
//!
//! **Never hold a lock across a callback into the interpreter.**
//!
//! Lisp can re-enter the editor through any primitive, so a lock held across
//! `eval` is a lock handed to arbitrary user code. `std::sync::RwLock` is not
//! reentrant, so the moment that code needs the same lock the thread waits on
//! itself. This is not a race that shows up under load on a busy machine -- it
//! hangs every time, on one thread, from Lisp a user could plausibly write.
//!
//! # Why these tests need a watchdog
//!
//! A deadlock does not fail a test, it hangs it, and `cargo test` has no
//! per-test timeout -- so a regression here would stall CI rather than report
//! anything useful. Each case therefore runs on its own thread and is awaited
//! with a deadline; the assertion is on whether it finished. A thread that has
//! genuinely deadlocked is left parked, which is fine: the process is about to
//! end, and a leaked thread is a far better outcome than a hung suite.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::create_global_env;
    use crate::lisp::{Parser, eval};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// Generous next to the milliseconds these take when they work, and short
    /// enough that a regression reports quickly.
    const DEADLINE: Duration = Duration::from_secs(10);

    /// Run `body` on its own thread and report whether it finished in time.
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

    /// A `post-command-hook` that registers another hook.
    ///
    /// `run_hook` used to hold `mode_registry.read()` for the whole loop,
    /// including the `eval` of each hook. `add-hook` needs
    /// `mode_registry.write()`, so this hung the editor permanently. The fix is
    /// to copy the hook list out and drop the guard before evaluating any of
    /// it.
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

    /// The same hazard reached through the other writers of the mode registry:
    /// `define-key` with a mode argument, and `make-mode`. A mode that binds
    /// its keys lazily on first command is ordinary Lisp, not a stress test.
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
