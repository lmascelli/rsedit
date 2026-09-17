//! Killing a buffer that more than one window is showing.
//!
//! # The crash these pin
//!
//! `close_buffer` used to repoint the *focused* window and no other. Open one
//! buffer in two windows, kill it, and the second window was left naming a
//! buffer that no longer existed. Nothing complained until focus moved there,
//! at which point the current buffer name became that dead name and the next
//! `get_current_buffer` hit its "Corruption in the hashmap of buffers" panic --
//! taking the editor down, and whatever was unsaved in the other windows with
//! it.
//!
//! A panic is not something a test can catch politely, so these check the
//! invariant instead: after a kill, no window names a buffer that is not there.
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
        (ctx, env)
    }

    /// The buffer each tiled window is showing.
    fn shown(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<String> {
        ctx.snapshot(env, 80, 24)
            .views
            .iter()
            .map(|view| view.buffer_name.clone())
            .collect()
    }

    /// The invariant: no window names a buffer that does not exist.
    fn assert_no_dangling(ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        for name in shown(ctx, env) {
            assert!(
                ctx.get_buffer(&name).is_some(),
                "window is showing {name:?}, which is not a live buffer"
            );
        }
    }

    #[test]
    fn killing_a_buffer_shown_twice_leaves_no_window_dangling() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "doomed" 'fundamental-mode)
               (switch-to-buffer "doomed")
               (split-window-right)"#,
            &env,
            &ctx,
        );
        assert_eq!(shown(&ctx, &env), vec!["doomed", "doomed"]);

        run(r#"(close-buffer "doomed")"#, &env, &ctx);

        assert_no_dangling(&ctx, &env);
    }

    #[test]
    fn focusing_the_other_window_afterwards_does_not_crash() {
        // The step that actually took the editor down: the dangling name only
        // became fatal once it was made current.
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "doomed" 'fundamental-mode)
               (switch-to-buffer "doomed")
               (split-window-right)
               (close-buffer "doomed")
               (other-window 1)"#,
            &env,
            &ctx,
        );
        // Reaching here at all is most of the test -- the old code panicked
        // inside `get_current_buffer` on the way.
        assert!(ctx.get_buffer(&ctx.get_current_buffer_name()).is_some());
        assert_no_dangling(&ctx, &env);
    }

    #[test]
    fn an_unfocused_window_is_repointed_as_well_as_the_focused_one() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "doomed" 'fundamental-mode)
               (switch-to-buffer "doomed")
               (split-window-right)
               (close-buffer "doomed")"#,
            &env,
            &ctx,
        );
        for name in shown(&ctx, &env) {
            assert_ne!(name, "doomed", "no window may still name the killed buffer");
        }
    }

    #[test]
    fn three_windows_on_one_buffer_are_all_repointed() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "doomed" 'fundamental-mode)
               (switch-to-buffer "doomed")
               (split-window-right)
               (split-window-below)
               (close-buffer "doomed")"#,
            &env,
            &ctx,
        );
        assert_eq!(shown(&ctx, &env).len(), 3, "the frame is unchanged");
        assert_no_dangling(&ctx, &env);
    }

    #[test]
    fn a_window_showing_something_else_is_left_alone() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "keep" 'fundamental-mode)
               (buffer-create "doomed" 'fundamental-mode)
               (switch-to-buffer "keep")
               (split-window-right)
               (other-window 1)
               (switch-to-buffer "doomed")
               (close-buffer "doomed")"#,
            &env,
            &ctx,
        );
        assert!(
            shown(&ctx, &env).contains(&"keep".to_string()),
            "killing one buffer must not disturb a window showing another"
        );
        assert_no_dangling(&ctx, &env);
    }

    // ----------------------------------------------------------------
    // What replaces it
    // ----------------------------------------------------------------

    #[test]
    fn the_replacement_is_the_most_recently_used_buffer() {
        // `*scratch*` is a poor answer when the person had files open.
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "older" 'fundamental-mode)
               (buffer-create "newer" 'fundamental-mode)
               (switch-to-buffer "older")
               (switch-to-buffer "newer")
               (switch-to-buffer "doomed-not-really")"#,
            &env,
            &ctx,
        );
        run(
            r#"(buffer-create "doomed" 'fundamental-mode)
               (switch-to-buffer "doomed")
               (close-buffer "doomed")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            ctx.get_current_buffer_name(),
            "newer",
            "the buffer used most recently before the one that was killed"
        );
    }

    #[test]
    fn what_is_current_and_what_is_on_screen_agree_afterwards() {
        // Them disagreeing is how the panic was reached at all.
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "other" 'fundamental-mode)
               (switch-to-buffer "other")
               (buffer-create "doomed" 'fundamental-mode)
               (switch-to-buffer "doomed")
               (split-window-right)
               (close-buffer "doomed")"#,
            &env,
            &ctx,
        );
        let current = ctx.get_current_buffer_name();
        assert!(
            shown(&ctx, &env).contains(&current),
            "the current buffer {current:?} should be one of the windows' {:?}",
            shown(&ctx, &env)
        );
    }

    #[test]
    fn with_nothing_else_to_show_it_falls_back_to_scratch() {
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "only" 'fundamental-mode)
               (switch-to-buffer "only")
               (split-window-right)
               (close-buffer "only")"#,
            &env,
            &ctx,
        );
        assert_no_dangling(&ctx, &env);
        assert!(
            shown(&ctx, &env).iter().all(|n| n == "*scratch*"),
            "got {:?}",
            shown(&ctx, &env)
        );
    }

    #[test]
    fn a_killed_buffer_is_never_offered_as_the_replacement() {
        // Names are left in the recency list rather than pruned on every kill,
        // so readers have to skip the ones that are gone.
        let (ctx, env) = editor();
        run(
            r#"(buffer-create "first" 'fundamental-mode)
               (switch-to-buffer "first")
               (buffer-create "second" 'fundamental-mode)
               (switch-to-buffer "second")
               (close-buffer "first")
               (buffer-create "third" 'fundamental-mode)
               (switch-to-buffer "third")
               (close-buffer "third")"#,
            &env,
            &ctx,
        );
        assert_ne!(ctx.get_current_buffer_name(), "first");
        assert!(ctx.get_buffer(&ctx.get_current_buffer_name()).is_some());
    }
}
