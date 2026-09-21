//! Running a shell command, and reading its output back.
//!
//! These actually spawn processes. That is the point -- this is the first
//! place the editor starts one, and a test that mocked the spawning would
//! check only that the mock was called.
//!
//! Everything here has to wait for the worker thread. `settle` polls rather
//! than sleeping a fixed time, so the tests are as fast as the machine allows
//! and do not fail on a slow one.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::lisp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

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
        for source in [
            include_str!("../../lisp/commands.lisp"),
            include_str!("../../lisp/debug.lisp"),
            include_str!("../../lisp/shell.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        (ctx, env)
    }

    /// Wait until every running command has finished, or give up.
    ///
    /// Polling the editor's own count rather than sleeping a guessed interval:
    /// the count is the thing the renderer consults too, so a test that waits
    /// on it waits for exactly what the editor considers "still working".
    fn settle(ctx: &Ctx) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while ctx.shell_commands_running() > 0 {
            assert!(
                Instant::now() < deadline,
                "a shell command did not finish within ten seconds"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn text_of(ctx: &Ctx, name: &str) -> String {
        ctx.get_buffer(name)
            .unwrap_or_else(|| panic!("no buffer called {name}"))
            .read()
            .expect("read lock")
            .text
            .to_string()
    }

    fn started(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match run(src, env, ctx) {
            LispExp::String(name) => name.to_string(),
            other => panic!("expected a buffer name, got {other:?}"),
        }
    }

    #[test]
    fn a_commands_output_reaches_its_buffer() {
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "echo hello")"#, &env, &ctx);
        settle(&ctx);
        assert!(
            text_of(&ctx, &name).contains("hello"),
            "got {:?}",
            text_of(&ctx, &name)
        );
    }

    #[test]
    fn the_command_is_echoed_at_the_top() {
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "echo hi")"#, &env, &ctx);
        settle(&ctx);
        assert!(text_of(&ctx, &name).starts_with("$ echo hi\n"));
    }

    #[test]
    fn a_successful_command_says_it_exited_zero() {
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "true")"#, &env, &ctx);
        settle(&ctx);
        assert!(
            text_of(&ctx, &name).contains("--- exited 0 ---"),
            "got {:?}",
            text_of(&ctx, &name)
        );
    }

    #[test]
    fn a_failing_command_reports_its_status() {
        // Without this you cannot tell a command that produced no output from
        // one that failed before producing any.
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "exit 3")"#, &env, &ctx);
        settle(&ctx);
        assert!(
            text_of(&ctx, &name).contains("--- exited 3 ---"),
            "got {:?}",
            text_of(&ctx, &name)
        );
    }

    #[test]
    fn standard_error_is_kept_and_shown() {
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "echo problem >&2")"#, &env, &ctx);
        settle(&ctx);
        assert!(
            text_of(&ctx, &name).contains("problem"),
            "a failure message must not be dropped, got {:?}",
            text_of(&ctx, &name)
        );
    }

    #[test]
    fn the_command_goes_to_a_shell_so_pipes_work() {
        // The whole value of `M-!` is that what you type is what you would
        // have typed at a prompt. Splitting on spaces here would make this
        // three arguments to `echo`.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "echo one two three | tr ' ' '-'")"#,
            &env,
            &ctx,
        );
        settle(&ctx);
        assert!(
            text_of(&ctx, &name).contains("one-two-three"),
            "got {:?}",
            text_of(&ctx, &name)
        );
    }

    #[test]
    fn quoting_survives_the_way_it_would_at_a_prompt() {
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "echo 'two words'")"#, &env, &ctx);
        settle(&ctx);
        assert!(text_of(&ctx, &name).contains("two words"));
    }

    #[test]
    fn the_output_buffer_is_read_only() {
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "echo x")"#, &env, &ctx);
        settle(&ctx);
        let before = text_of(&ctx, &name);
        run(
            &format!(r#"(switch-to-buffer "{name}") (goto-char 0) (insert "typed")"#),
            &env,
            &ctx,
        );
        assert_eq!(
            text_of(&ctx, &name),
            before,
            "nobody types into a transcript"
        );
    }

    #[test]
    fn a_second_command_gets_a_buffer_of_its_own() {
        // One buffer between two commands would interleave their lines into
        // something neither of them said.
        let (ctx, env) = editor();
        let first = started(r#"(shell-command-start "echo first")"#, &env, &ctx);
        let second = started(r#"(shell-command-start "echo second")"#, &env, &ctx);
        assert_ne!(first, second);
        settle(&ctx);
        assert!(text_of(&ctx, &first).contains("first"));
        assert!(text_of(&ctx, &second).contains("second"));
        assert!(!text_of(&ctx, &first).contains("second"));
    }

    #[test]
    fn the_editor_knows_while_a_command_is_running() {
        // What keeps the renderer waking up. Without it the output would sit
        // in the buffer unseen until some key was pressed.
        let (ctx, env) = editor();
        assert!(run("(shell-command-running-p)", &env, &ctx).is_nil());
        started(r#"(shell-command-start "echo x")"#, &env, &ctx);
        // Counted before the task is sent, so this is true the instant the
        // primitive returns rather than once the worker gets round to it.
        assert!(!run("(shell-command-running-p)", &env, &ctx).is_nil());
        settle(&ctx);
        assert!(run("(shell-command-running-p)", &env, &ctx).is_nil());
    }

    #[test]
    fn a_running_command_keeps_the_renderer_awake() {
        let (ctx, env) = editor();
        started(r#"(shell-command-start "sleep 0.2")"#, &env, &ctx);
        let frame = ctx.snapshot(&env, 80, 24);
        assert!(
            ctx.next_redraw_in(&env, &frame).is_some(),
            "the renderer must come back to look, since nothing will press a key"
        );
        settle(&ctx);
    }

    #[test]
    fn killing_the_output_buffer_mid_command_is_allowed() {
        // A perfectly reasonable thing to have done, and the worker must not
        // panic when it finds the buffer gone.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "sleep 0.1; echo late")"#,
            &env,
            &ctx,
        );
        run(&format!(r#"(close-buffer "{name}")"#), &env, &ctx);
        settle(&ctx);
        assert!(ctx.get_buffer(&name).is_none());
    }

    #[test]
    fn a_command_that_writes_a_lot_arrives_whole() {
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "seq 1 500")"#, &env, &ctx);
        settle(&ctx);
        let text = text_of(&ctx, &name);
        assert!(text.contains("\n1\n"), "the first line");
        assert!(text.contains("\n500\n"), "and the last");
    }

    #[test]
    fn the_lisp_command_shows_the_buffer_in_a_window() {
        let (ctx, env) = editor();
        let before = ctx.window_count();
        run(r#"(shell-command "echo shown")"#, &env, &ctx);
        assert!(
            ctx.window_count() > before,
            "the output should be on screen without being switched to"
        );
        settle(&ctx);
    }
}
