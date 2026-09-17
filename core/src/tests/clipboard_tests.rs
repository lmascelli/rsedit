//! Killed text reaching the system clipboard.
//!
//! The editor cannot put anything in a clipboard -- it does not know it has a
//! terminal. What it does is leave the text on the frame for a renderer to
//! emit, so what is testable here is exactly that: which kills queue text,
//! which do not, and that a queued payload is handed over once and once only.
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

    /// The editor with `clipboard.lisp` loaded, which is what turns sync on.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/clipboard.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    #[test]
    fn killing_the_region_offers_it_to_the_clipboard() {
        let (ctx, env) = editor();
        run(
            r#"(insert "hello world") (goto-char 0) (set-mark) (goto-char 5)
               (kill-region)"#,
            &env,
            &ctx,
        );
        assert_eq!(ctx.take_pending_clipboard().as_deref(), Some("hello"));
    }

    #[test]
    fn copying_offers_it_too_without_deleting_anything() {
        let (ctx, env) = editor();
        run(
            r#"(insert "hello world") (goto-char 0) (set-mark) (goto-char 5)
               (kill-ring-save)"#,
            &env,
            &ctx,
        );
        assert_eq!(ctx.take_pending_clipboard().as_deref(), Some("hello"));
    }

    #[test]
    fn a_kill_that_is_not_the_region_offers_itself_as_well() {
        // The reason `kill' is the place this hangs off rather than the two
        // region commands: there are eleven commands that kill, and every one
        // of them should reach the clipboard without being listed anywhere.
        let (ctx, env) = editor();
        run(r#"(insert "one two") (goto-char 0) (kill-word)"#, &env, &ctx);
        assert_eq!(ctx.take_pending_clipboard().as_deref(), Some("one"));
    }

    #[test]
    fn nothing_is_offered_when_sync_is_off() {
        let (ctx, env) = editor();
        run(
            r#"(setq clipboard-sync nil)
               (insert "hello") (goto-char 0) (set-mark) (goto-char 5)
               (kill-region)"#,
            &env,
            &ctx,
        );
        assert_eq!(ctx.take_pending_clipboard(), None);
    }

    #[test]
    fn an_editor_that_loaded_no_lisp_offers_nothing() {
        // Unbound means off, and this is why: every test in this crate kills
        // something eventually, and none of them has a terminal to send an
        // escape to.
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        run(
            r#"(insert "hello") (goto-char 0) (set-mark) (goto-char 5)
               (kill-region)"#,
            &env,
            &ctx,
        );
        assert_eq!(ctx.take_pending_clipboard(), None);
    }

    #[test]
    fn the_payload_is_handed_over_once() {
        let (ctx, env) = editor();
        run(
            r#"(insert "hello") (goto-char 0) (set-mark) (goto-char 5)
               (kill-region)"#,
            &env,
            &ctx,
        );
        assert_eq!(ctx.take_pending_clipboard().as_deref(), Some("hello"));
        // Taken, not read. A second frame drawn without anything being killed
        // must not re-send the escape.
        assert_eq!(ctx.take_pending_clipboard(), None);
    }

    #[test]
    fn a_snapshot_carries_it_and_the_next_one_does_not() {
        let (ctx, env) = editor();
        run(
            r#"(insert "hello") (goto-char 0) (set-mark) (goto-char 5)
               (kill-region)"#,
            &env,
            &ctx,
        );
        let first = ctx.snapshot(&env, 80, 24);
        assert_eq!(first.clipboard.as_deref(), Some("hello"));
        let second = ctx.snapshot(&env, 80, 24);
        assert_eq!(second.clipboard, None);
    }

    #[test]
    fn a_frame_with_nothing_killed_carries_nothing() {
        let (ctx, env) = editor();
        run(r#"(insert "hello")"#, &env, &ctx);
        assert_eq!(ctx.snapshot(&env, 80, 24).clipboard, None);
    }

    #[test]
    fn a_refused_kill_offers_nothing() {
        // A read-only buffer refuses the deletion, so nothing leaves the
        // buffer -- and a clipboard holding text that is still there would be
        // a lie about what just happened.
        let (ctx, env) = editor();
        run(
            r#"(insert "hello") (goto-char 0) (set-mark) (goto-char 5)
               (set-buffer-read-only t)
               (kill-region)"#,
            &env,
            &ctx,
        );
        assert_eq!(ctx.take_pending_clipboard(), None);
    }
}
