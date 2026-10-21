//! Running a shell command, and reading its output back.
//!
//! These actually spawn processes. That is the point -- this is the first
//! place the editor starts one, and a test that mocked the spawning would
//! check only that the mock was called.
//!
//! Everything here has to wait for a command's own thread. `settle` polls
//! rather than sleeping a fixed time, so the tests are as fast as the machine
//! allows and do not fail on a slow one.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
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
        ctx.with_buffer(name, |b| b.text.to_string())
            .unwrap_or_else(|| panic!("no buffer called {name}"))
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
        // A command that lasts. `echo` could be over before the question
        // below was asked, and on a loaded machine about once in a hundred
        // runs it was: the count had gone up and come down again.
        started(r#"(shell-command-start "sleep 0.5")"#, &env, &ctx);
        // Counted before the thread starts, so this is true the instant the
        // primitive returns rather than once the thread gets going.
        assert!(!run("(shell-command-running-p)", &env, &ctx).is_nil());
        settle(&ctx);
        assert!(run("(shell-command-running-p)", &env, &ctx).is_nil());
    }

    /// Wait until the buffer called NAME contains NEEDLE, or give up.
    fn shows(ctx: &Ctx, name: &str, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !text_of(ctx, name).contains(needle) {
            assert!(
                Instant::now() < deadline,
                "{needle:?} never arrived: {:?}",
                text_of(ctx, name)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn output_wakes_the_renderer_and_silence_does_not() {
        // What used to keep the renderer waking every turn for as long as any
        // command ran, output or not. Now each piece of output says so, and a
        // command that is quiet costs a frame nothing.
        let (ctx, env) = editor();
        let told = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        ctx.on_background_change({
            let told = told.clone();
            move || {
                told.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        // Still running once it has spoken, so that only the line itself can
        // have told the host -- the end of the thread tells it too.
        let name = started(
            r#"(shell-command-start "sleep 0.3; echo late; sleep 30")"#,
            &env,
            &ctx,
        );
        // The output buffer is coloured, as its mode says, before the frame:
        // colour still to come is a reason of its own to come back.
        ctx.work().run_until_idle(&ctx);
        let frame = ctx.snapshot(&env, 80, 24);
        let before = told.load(std::sync::atomic::Ordering::SeqCst);
        let soon = crate::modes::highlighter::TURN_INTERVAL;
        assert!(
            ctx.next_redraw_in(&env, &frame)
                .is_none_or(|wait| wait > soon),
            "nothing has been said yet, so nothing is due"
        );
        // The whole line: the command at the top says `late' too.
        shows(&ctx, &name, "\nlate\n");
        assert!(
            told.load(std::sync::atomic::Ordering::SeqCst) > before,
            "the host was told"
        );
        assert!(
            ctx.next_redraw_in(&env, &frame)
                .is_some_and(|wait| wait <= soon),
            "and the line is drawn within a turn"
        );
        run(&format!(r#"(stop-worker "{name}")"#), &env, &ctx);
        settle(&ctx);
    }

    // ----------------------------------------------------------------
    // What the output may be
    // ----------------------------------------------------------------

    #[test]
    fn a_command_that_fills_standard_error_is_not_stuck() {
        // SHL-1. Standard output was read to the end before standard error was
        // read at all, so a command that wrote more than a pipe holds to
        // standard error -- a build full of warnings -- before closing its
        // standard output waited on a reader that was waiting on it.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "yes warning | head -c 200000 >&2; echo done")"#,
            &env,
            &ctx,
        );
        settle(&ctx);
        let text = text_of(&ctx, &name);
        assert!(text.contains("\ndone\n"), "the output after it arrived");
        assert!(text.contains("--- exited 0 ---"), "and the command ended");
        assert!(
            text.matches("warning").count() > 20_000,
            "and none of standard error was lost"
        );
    }

    #[test]
    fn the_two_streams_arrive_in_the_order_they_were_written() {
        // One pipe for both, as at a prompt: a failure message lands next to
        // the output it is about, not after all of it.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "echo one; echo two >&2; echo three")"#,
            &env,
            &ctx,
        );
        settle(&ctx);
        let text = text_of(&ctx, &name);
        let at = |word: &str| text.find(&format!("\n{word}\n")).unwrap_or(usize::MAX);
        assert!(at("one") < at("two") && at("two") < at("three"), "{text:?}");
    }

    #[test]
    fn output_that_is_not_utf8_is_shown_and_the_command_goes_on() {
        // SHL-2. The first line that was not UTF-8 ended the reading, the pipe
        // closed under the command, and it died of SIGPIPE at its next write:
        // a build printing one Latin-1 file name was killed half-way.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "printf 'one\n\377\ntwo\n'; sleep 0.2; echo after")"#,
            &env,
            &ctx,
        );
        settle(&ctx);
        let text = text_of(&ctx, &name);
        assert!(text.contains("\none\n\u{fffd}\ntwo\n"), "{text:?}");
        assert!(text.contains("\nafter\n"), "it went on: {text:?}");
        assert!(
            text.contains("--- exited 0 ---"),
            "and was not killed: {text:?}"
        );
    }

    #[test]
    fn a_last_line_with_no_newline_still_ends_before_the_trailer() {
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "printf unfinished")"#, &env, &ctx);
        settle(&ctx);
        assert!(
            text_of(&ctx, &name).ends_with("\nunfinished\n--- exited 0 ---\n"),
            "{:?}",
            text_of(&ctx, &name)
        );
    }

    #[test]
    fn a_line_longer_than_a_batch_arrives_whole() {
        // Read in pieces, appended one after the other: nothing lost at a
        // cut, and nothing added.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "printf a; yes é | head -n 40000 | tr -d '\n'; echo")"#,
            &env,
            &ctx,
        );
        settle(&ctx);
        let text = text_of(&ctx, &name);
        let line = format!("\na{}\n", "é".repeat(40_000));
        assert!(text.contains(&line), "{} bytes", text.len());
    }

    #[test]
    fn a_character_cut_in_two_by_the_reading_is_shown_whole() {
        // Its first byte is read on its own, and kept for the next read
        // rather than shown as a broken character, and then another.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "printf 'caf\303'; sleep 0.2; printf '\251\n'")"#,
            &env,
            &ctx,
        );
        settle(&ctx);
        let text = text_of(&ctx, &name);
        assert!(text.contains("\ncafé\n"), "{text:?}");
        assert!(!text.contains('\u{fffd}'), "{text:?}");
    }

    #[test]
    fn what_does_not_end_a_line_is_shown_before_the_line_ends() {
        // A progress bar, or a question: read a line at a time, it waited for
        // a newline that may never come. Composed by the shell, so that the
        // command at the top does not say it already.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "printf 'wait%s' ing; sleep 30")"#,
            &env,
            &ctx,
        );
        shows(&ctx, &name, "\nwaiting");
        run(&format!(r#"(stop-worker "{name}")"#), &env, &ctx);
        settle(&ctx);
        let text = text_of(&ctx, &name);
        assert!(text.ends_with("\nwaiting\n--- killed ---\n"), "{text:?}");
    }

    // ----------------------------------------------------------------
    // Stopping one
    // ----------------------------------------------------------------

    #[test]
    fn a_command_can_be_stopped_by_the_name_of_its_buffer() {
        // SHL-3. Nothing could reach the process: it belonged to the thread
        // reading it, and ran to its end whatever happened.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "echo started; sleep 30; echo never")"#,
            &env,
            &ctx,
        );
        shows(&ctx, &name, "\nstarted\n");
        let stopped = Instant::now();
        assert_eq!(
            run(&format!(r#"(stop-worker "{name}")"#), &env, &ctx),
            LispExp::t()
        );
        settle(&ctx);
        assert!(
            stopped.elapsed() < Duration::from_secs(5),
            "it stopped when asked, not when it was done"
        );
        let text = text_of(&ctx, &name);
        assert!(text.ends_with("--- killed ---\n"), "{text:?}");
        assert!(!text.contains("\nnever\n"), "{text:?}");
    }

    /// How many processes are running COMMAND, word for word.
    fn processes_running(command: &str) -> usize {
        let listing = std::process::Command::new("ps")
            .args(["-eo", "args="])
            .output()
            .expect("ps runs");
        String::from_utf8_lossy(&listing.stdout)
            .lines()
            .filter(|line| line.trim() == command)
            .count()
    }

    /// Wait until no process is running COMMAND, or give up.
    fn gone(command: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while processes_running(command) > 0 {
            assert!(
                Instant::now() < deadline,
                "{command} is still running: the stop missed it"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn stopping_a_command_stops_what_it_started_too() {
        // Killing the shell alone leaves its children running, holding the
        // pipe open: the reading would go on until they were done, and they
        // would outlive the editor. A duration nothing else here sleeps for,
        // so the processes can be found by it.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "echo started; sleep 31.25 | cat; echo never")"#,
            &env,
            &ctx,
        );
        shows(&ctx, &name, "\nstarted\n");
        assert_eq!(processes_running("sleep 31.25"), 1);
        let stopped = Instant::now();
        run(&format!(r#"(stop-worker "{name}")"#), &env, &ctx);
        settle(&ctx);
        assert!(stopped.elapsed() < Duration::from_secs(5));
        assert!(text_of(&ctx, &name).ends_with("--- killed ---\n"));
        gone("sleep 31.25");
    }

    #[test]
    fn a_command_stopped_before_it_has_said_anything_leaves_nothing_behind() {
        // Stopped at once: the request arrives before the thread is reading,
        // and must still reach the whole group rather than the shell alone.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "sleep 32.5; echo never")"#,
            &env,
            &ctx,
        );
        run(&format!(r#"(stop-worker "{name}")"#), &env, &ctx);
        settle(&ctx);
        gone("sleep 32.5");
        assert!(text_of(&ctx, &name).ends_with("--- killed ---\n"));
    }

    #[test]
    fn a_command_that_ignores_term_is_killed_after_its_grace() {
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "trap '' TERM; echo started; sleep 33.75")"#,
            &env,
            &ctx,
        );
        shows(&ctx, &name, "\nstarted\n");
        let stopped = Instant::now();
        run(&format!(r#"(stop-worker "{name}")"#), &env, &ctx);
        settle(&ctx);
        let took = stopped.elapsed();
        assert!(
            took >= Duration::from_secs(1) && took < Duration::from_secs(8),
            "TERM did nothing, KILL came after the grace: {took:?}"
        );
        assert!(text_of(&ctx, &name).ends_with("--- killed ---\n"));
        gone("sleep 33.75");
    }

    #[test]
    fn a_command_the_system_has_stopped_is_stopped_at_once() {
        // As one that read the terminal is: in a group of its own, it is
        // stopped there. Stopped, it does not hear TERM until it is
        // continued, and without that it went only with the KILL.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "echo started; kill -s STOP $$; echo never")"#,
            &env,
            &ctx,
        );
        shows(&ctx, &name, "\nstarted\n");
        // Long enough for the shell to have stopped itself.
        std::thread::sleep(Duration::from_millis(200));
        let stopped = Instant::now();
        run(&format!(r#"(stop-worker "{name}")"#), &env, &ctx);
        settle(&ctx);
        let took = stopped.elapsed();
        assert!(
            took < Duration::from_secs(1),
            "not after the grace: {took:?}"
        );
        let text = text_of(&ctx, &name);
        assert!(text.ends_with("--- killed ---\n"), "{text:?}");
        assert!(!text.contains("\nnever\n"), "{text:?}");
    }

    #[test]
    fn a_stopped_command_says_killed_however_it_exits() {
        // One that tidies up on TERM and leaves with a status of its own was
        // still stopped: the trailer says so, rather than report the status
        // as though the command had finished.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "trap 'exit 3' TERM; echo started; sleep 30 & wait")"#,
            &env,
            &ctx,
        );
        shows(&ctx, &name, "\nstarted\n");
        run(&format!(r#"(stop-worker "{name}")"#), &env, &ctx);
        settle(&ctx);
        let text = text_of(&ctx, &name);
        assert!(text.ends_with("\nstarted\n--- killed ---\n"), "{text:?}");
    }

    #[test]
    fn a_command_does_not_outlive_the_editor() {
        // Shut down, the editor stops its commands as it stops all its work.
        // Nothing reached them before: a command went on running after the
        // editor had gone, with nobody left to read what it said.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "echo started; sleep 41.25")"#,
            &env,
            &ctx,
        );
        shows(&ctx, &name, "\nstarted\n");
        ctx.shutdown(Duration::from_secs(5));
        gone("sleep 41.25");
    }

    #[test]
    fn closing_the_output_stops_the_command() {
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "echo started; sleep 34.5")"#,
            &env,
            &ctx,
        );
        shows(&ctx, &name, "\nstarted\n");
        let closed = Instant::now();
        run(&format!(r#"(close-buffer "{name}")"#), &env, &ctx);
        settle(&ctx);
        assert!(closed.elapsed() < Duration::from_secs(5));
        gone("sleep 34.5");
    }

    #[test]
    fn a_running_command_is_listed_by_the_name_of_its_buffer() {
        let (ctx, env) = editor();
        let name = started(r#"(shell-command-start "sleep 30")"#, &env, &ctx);
        let listed = run(
            &format!(
                r#"(let ((found nil))
                     (mapc (lambda (entry)
                             (if (equal (nth 1 (memq 'name entry)) "{name}")
                                 (setq found (nth 1 (memq 'kind entry)))))
                           (work-list))
                     found)"#
            ),
            &env,
            &ctx,
        );
        assert_eq!(listed, LispExp::symbol("thread".into()));
        run(&format!(r#"(stop-worker "{name}")"#), &env, &ctx);
        settle(&ctx);
    }

    #[test]
    fn the_stop_command_stops_the_command_of_the_buffer_it_is_given_in() {
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "echo started; sleep 30")"#,
            &env,
            &ctx,
        );
        shows(&ctx, &name, "\nstarted\n");
        run(
            &format!(r#"(with-current-buffer "{name}" (lambda () (shell-command-kill)))"#),
            &env,
            &ctx,
        );
        settle(&ctx);
        assert!(text_of(&ctx, &name).ends_with("--- killed ---\n"));
        run(
            &format!(r#"(with-current-buffer "{name}" (lambda () (shell-command-kill)))"#),
            &env,
            &ctx,
        );
        assert_eq!(ctx.get_echo_message(), "Nothing is running in this buffer");
    }

    #[test]
    fn a_slow_command_does_not_hold_back_another_commands_output() {
        // What was actually serialised, and what was not.
        //
        // The *processes* were always concurrent: `spawn' returns as soon as
        // the child exists, so two commands have always run at the same time.
        // What queued was the *reading* -- one task read one pipe to the end
        // on the shared thread, and every other command's output sat in its
        // pipe until that finished. So a `git status' started during a build
        // really did run, and its output really did not appear for as long as
        // the build lasted; and a command writing more than a pipe will hold
        // would have blocked part-way through, waiting for a reader that was
        // busy with somebody else.
        let (ctx, env) = editor();
        let slow = started(r#"(shell-command-start "sleep 1; echo slow")"#, &env, &ctx);
        let quick = started(r#"(shell-command-start "echo quick")"#, &env, &ctx);
        // The exit trailer in each buffer, rather than the output. The first
        // line of a buffer is the command itself -- `$ echo quick' -- so
        // looking for what a command will print finds what it was asked to
        // print, and reports it as finished before it has started. The
        // trailer is written by the reader, and only when the pipe is closed,
        // so it means what this needs it to mean.
        let deadline = Instant::now() + Duration::from_millis(500);
        while !text_of(&ctx, &quick).contains("--- exited") {
            assert!(
                Instant::now() < deadline,
                "the quick command was not read until the slow one had been: {:?}",
                text_of(&ctx, &quick)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !text_of(&ctx, &slow).contains("--- exited"),
            "and it got there while the slow one was still going: {:?}",
            text_of(&ctx, &slow)
        );
        settle(&ctx);
    }

    #[test]
    fn a_command_does_not_hold_up_the_editors_other_background_work() {
        // The failure this prevents is the one that was actually there: a
        // `cargo build' read on the shared thread stopped syntax colouring,
        // the file watcher, the auto-saver and every Lisp worker until the
        // build finished.
        //
        // A background job stands in for all of them, because it is the one
        // that can be *watched* -- it reports when it is done, and the report
        // arriving while the command is still running is the whole assertion.
        // The job's steps are taken by this test, which is how the editor's
        // work runs in tests; the command runs on a thread of its own.
        let (ctx, env) = editor();
        run(
            "(setq job-finished nil)
             (defun quick-job () t)
             (defun quick-done (name ok) (setq job-finished t))",
            &env,
            &ctx,
        );
        started(r#"(shell-command-start "sleep 1")"#, &env, &ctx);
        run(
            "(background-call 'quick 'quick-job nil 'quick-done)",
            &env,
            &ctx,
        );
        let deadline = Instant::now() + Duration::from_millis(700);
        while run("job-finished", &env, &ctx).is_nil() {
            assert!(
                Instant::now() < deadline,
                "the job waited for the shell command, which is the bug"
            );
            ctx.work().run_until_idle(&ctx);
            ctx.run_owed_callbacks(&env);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !run("(shell-command-running-p)", &env, &ctx).is_nil(),
            "and it got there while the command was still going"
        );
        settle(&ctx);
    }

    #[test]
    fn killing_the_output_buffer_mid_command_is_allowed() {
        // A perfectly reasonable thing to have done, and the command's thread
        // must not panic when it finds the buffer gone.
        let (ctx, env) = editor();
        let name = started(
            r#"(shell-command-start "sleep 0.1; echo late")"#,
            &env,
            &ctx,
        );
        run(&format!(r#"(close-buffer "{name}")"#), &env, &ctx);
        settle(&ctx);
        assert!(!ctx.has_buffer(&name));
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

    #[test]
    fn the_next_command_is_shown_in_the_strip_of_the_one_before() {
        // Each used to open a strip of its own under the others, until the
        // window being worked in was a single row.
        let (ctx, env) = editor();
        run(r#"(shell-command "echo one")"#, &env, &ctx);
        let strips = ctx.window_count();
        run(r#"(shell-command "echo two")"#, &env, &ctx);
        run(r#"(shell-command "echo three")"#, &env, &ctx);
        settle(&ctx);
        assert_eq!(ctx.window_count(), strips);
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        run("(other-window 1)", &env, &ctx);
        assert_eq!(ctx.get_current_buffer_name(), "*Shell Output*<3>");
    }

    #[test]
    fn a_strip_closed_or_given_away_is_not_taken_back() {
        let (ctx, env) = editor();
        run(r#"(shell-command "echo one")"#, &env, &ctx);
        run("(other-window 1) (delete-window)", &env, &ctx);
        let alone = ctx.window_count();
        run(r#"(shell-command "echo two")"#, &env, &ctx);
        assert_eq!(ctx.window_count(), alone + 1, "a strip again");

        // The strip given to a buffer of the user's: the next output gets a
        // strip of its own, and the buffer keeps its window.
        run(
            r#"(other-window 1) (switch-to-buffer "*scratch*") (other-window 1)"#,
            &env,
            &ctx,
        );
        run(r#"(shell-command "echo three")"#, &env, &ctx);
        settle(&ctx);
        assert_eq!(ctx.window_count(), alone + 2);
    }

    #[test]
    fn starting_a_command_without_showing_it_is_no_command() {
        // From M-x it ran the command and showed nothing; `shell-command` is
        // the command, and shows it.
        let (ctx, env) = editor();
        assert!(run("(commandp 'shell-command-start)", &env, &ctx).is_nil());
        assert!(run("(commandp 'shell-command)", &env, &ctx).is_truthy());
    }
}
