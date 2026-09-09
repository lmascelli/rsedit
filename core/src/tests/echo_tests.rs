//! The echo area's timeout: how long a message stays, and what controls it.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::thread::sleep;
    use std::time::Duration;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        create_global_env::<GapBuffer>().expect("global env must build")
    }

    /// What the renderer would draw in the echo area.
    fn shown(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
        ctx.snapshot(env, W, H).echo_message
    }

    /// Long enough that no amount of scheduling noise can expire a message
    /// mid-test, so "still shown" means the timeout is being honoured rather
    /// than the machine being fast.
    const NEVER_IN_PRACTICE: &str = "3600";
    /// Short enough that a modest sleep is certain to pass it.
    const ALREADY_GONE: &str = "0.01";
    const LONGER_THAN_THAT: Duration = Duration::from_millis(150);

    #[test]
    fn a_message_is_shown_while_its_timeout_has_not_run_out() {
        let (ctx, env) = editor();
        eval_str(
            &format!("(setq echo-message-timeout {NEVER_IN_PRACTICE})"),
            &env,
            &ctx,
        )
        .expect("setq");
        ctx.set_echo_message("still here");

        assert_eq!(shown(&ctx, &env), "still here");
        assert!(
            ctx.echo_expiry_in(&env).is_some(),
            "a message with a timeout in force is waiting to expire"
        );
    }

    #[test]
    fn a_message_disappears_once_its_timeout_runs_out() {
        let (ctx, env) = editor();
        eval_str(
            &format!("(setq echo-message-timeout {ALREADY_GONE})"),
            &env,
            &ctx,
        )
        .expect("setq");
        ctx.set_echo_message("fleeting");
        assert_eq!(shown(&ctx, &env), "fleeting", "shown to begin with");

        sleep(LONGER_THAN_THAT);
        assert_eq!(
            shown(&ctx, &env),
            "",
            "the message should be gone once its time is up"
        );
        assert_eq!(
            ctx.echo_expiry_in(&env),
            None,
            "nothing is left to wait for once it has expired"
        );
    }

    /// The view forgets; the state does not. Keeping the message means nothing
    /// has to run on a timer to tidy it away, and a caller that wants to know
    /// what was last said can still ask.
    #[test]
    fn an_expired_message_is_still_in_the_state_that_holds_it() {
        let (ctx, env) = editor();
        eval_str(
            &format!("(setq echo-message-timeout {ALREADY_GONE})"),
            &env,
            &ctx,
        )
        .expect("setq");
        ctx.set_echo_message("said once");
        sleep(LONGER_THAN_THAT);

        assert_eq!(shown(&ctx, &env), "", "the view drops it");
        assert_eq!(
            ctx.get_echo_message(),
            "said once",
            "but the editor still knows what it said"
        );
    }

    #[test]
    fn a_nil_timeout_keeps_the_message_on_screen() {
        let (ctx, env) = editor();
        eval_str("(setq echo-message-timeout nil)", &env, &ctx).expect("setq");
        ctx.set_echo_message("permanent");

        sleep(LONGER_THAN_THAT);
        assert_eq!(
            shown(&ctx, &env),
            "permanent",
            "nil means the message stays until something replaces it"
        );
        assert_eq!(
            ctx.echo_expiry_in(&env),
            None,
            "with no timeout there is nothing to wake up for"
        );
    }

    /// Anything that is not a number is treated as nil rather than guessed at.
    /// The failure mode that direction is a message that outstays its welcome;
    /// the other direction would be one that vanishes before it is read.
    #[test]
    fn a_timeout_that_is_not_a_number_is_treated_as_nil() {
        for value in ["\"soon\"", "'(1 2)", "t"] {
            let (ctx, env) = editor();
            eval_str(&format!("(setq echo-message-timeout {value})"), &env, &ctx).expect("setq");
            ctx.set_echo_message("kept");

            sleep(Duration::from_millis(20));
            assert_eq!(
                shown(&ctx, &env),
                "kept",
                "{value} should be treated as no timeout at all"
            );
        }
    }

    /// Each message is shown for its own full timeout, however soon it follows
    /// the one before -- the clock belongs to the message, not to the editor.
    #[test]
    fn a_new_message_restarts_the_clock() {
        let (ctx, env) = editor();
        eval_str(
            &format!("(setq echo-message-timeout {ALREADY_GONE})"),
            &env,
            &ctx,
        )
        .expect("setq");
        ctx.set_echo_message("first");
        sleep(LONGER_THAN_THAT);
        assert_eq!(shown(&ctx, &env), "", "the first one expired");

        eval_str(
            &format!("(setq echo-message-timeout {NEVER_IN_PRACTICE})"),
            &env,
            &ctx,
        )
        .expect("setq");
        ctx.set_echo_message("second");
        assert_eq!(
            shown(&ctx, &env),
            "second",
            "the new message starts its own timeout rather than inheriting the \
             expired one"
        );
    }

    /// The value matters, not just its presence: this is the deadline a UI
    /// event loop waits on, so a number that is too large leaves the message
    /// up past its time and one that is too small wakes the loop for nothing.
    #[test]
    fn the_time_left_is_what_is_actually_left() {
        let (ctx, env) = editor();
        eval_str("(setq echo-message-timeout 30)", &env, &ctx).expect("setq");
        ctx.set_echo_message("counting down");

        let left = ctx.echo_expiry_in(&env).expect("a message is pending");
        assert!(
            left <= Duration::from_secs(30),
            "cannot have longer left than the whole timeout, got {left:?}"
        );
        assert!(
            left > Duration::from_secs(29),
            "a message set a moment ago has nearly all of its time left, got {left:?}"
        );

        sleep(Duration::from_millis(100));
        let later = ctx.echo_expiry_in(&env).expect("still pending");
        assert!(
            later < left,
            "the time left must shrink as the message ages: {later:?} then {left:?}"
        );
    }

    #[test]
    fn an_empty_echo_area_is_never_waiting_to_expire() {
        let (ctx, env) = editor();
        eval_str(
            &format!("(setq echo-message-timeout {NEVER_IN_PRACTICE})"),
            &env,
            &ctx,
        )
        .expect("setq");
        ctx.set_echo_message("");

        assert_eq!(
            ctx.echo_expiry_in(&env),
            None,
            "an empty echo area must not keep the event loop waking up"
        );
    }

    #[test]
    fn a_negative_timeout_is_clamped_rather_than_panicking() {
        let (ctx, env) = editor();
        eval_str("(setq echo-message-timeout -5)", &env, &ctx).expect("setq");
        ctx.set_echo_message("gone at once");

        sleep(Duration::from_millis(10));
        assert_eq!(
            shown(&ctx, &env),
            "",
            "a negative timeout means no time at all, not a panic in Duration"
        );
    }

    /// `message` is the Lisp-facing way in, and it must obey the same timeout
    /// as a message set from Rust.
    #[test]
    fn a_message_set_from_lisp_expires_the_same_way() {
        let (ctx, env) = editor();
        eval_str(
            &format!("(setq echo-message-timeout {ALREADY_GONE}) (set-echo-message \"from lisp\")"),
            &env,
            &ctx,
        )
        .expect("set the message");
        assert_eq!(shown(&ctx, &env), "from lisp");

        sleep(LONGER_THAN_THAT);
        assert_eq!(shown(&ctx, &env), "");
    }

    /// The default is a number, so the timeout is on without any configuration
    /// -- that is the behaviour the variable exists to *adjust*, not to enable.
    #[test]
    fn the_timeout_is_armed_by_default() {
        let (ctx, env) = editor();
        ctx.set_echo_message("default policy");

        assert_eq!(
            shown(&ctx, &env),
            "default policy",
            "a fresh message is shown"
        );
        assert!(
            ctx.echo_expiry_in(&env).is_some(),
            "and it is on a clock without anyone having set the variable"
        );
    }
}
