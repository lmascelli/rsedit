//! Talking with a process: its output cut into pieces, lines or framed
//! messages, delivered between commands through a bounded queue; writing to
//! it, closing its input, stopping it. The cutting is checked on bytes alone;
//! the rest with real processes, as the shell's tests do, waiting by polling.
//!
//! Design: `doc/architettura-editor.typ, <processi-dialoganti>`.
#[cfg(test)]
mod tests {
    use crate::primitives::processes::{Frames, Lines, MAX_HEADER, MAX_LINE, framed};

    // ----------------------------------------------------------------
    // Lines and messages, from bytes
    // ----------------------------------------------------------------

    #[test]
    fn a_line_comes_without_its_end_whichever_end_it_has() {
        let mut lines = Lines::default();
        assert_eq!(lines.push(b"one\ntwo\r\nthr"), ["one", "two"]);
        assert_eq!(lines.push(b"ee\n"), ["three"]);
        assert_eq!(lines.finish(), None);
    }

    #[test]
    fn a_last_line_without_its_end_comes_at_the_end() {
        let mut lines = Lines::default();
        assert!(lines.push(b"no end").is_empty());
        assert_eq!(lines.finish().as_deref(), Some("no end"));
    }

    #[test]
    fn a_character_cut_between_two_reads_is_read_whole() {
        let mut lines = Lines::default();
        let bytes = "è\n".as_bytes();
        assert!(lines.push(&bytes[..1]).is_empty());
        assert_eq!(lines.push(&bytes[1..]), ["è"]);
    }

    #[test]
    fn a_line_longer_than_the_longest_comes_in_pieces() {
        let mut lines = Lines::default();
        let long = vec![b'x'; MAX_LINE + 10];
        let pieces = lines.push(&long);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].len(), MAX_LINE);
        assert_eq!(lines.finish().map(|rest| rest.len()), Some(10));
    }

    #[test]
    fn a_message_is_its_body_however_its_bytes_arrive() {
        let message = framed(r#"{"è":"ü"}"#);
        let mut frames = Frames::default();
        let mut bodies = Vec::new();
        for byte in &message {
            bodies.extend(
                frames
                    .push(std::slice::from_ref(byte))
                    .expect("well framed"),
            );
        }
        assert_eq!(bodies, [r#"{"è":"ü"}"#]);
        assert!(frames.is_empty());
    }

    #[test]
    fn the_length_of_a_framed_message_counts_bytes() {
        assert!(framed("è").starts_with(b"Content-Length: 2\r\n\r\n"));
    }

    #[test]
    fn two_messages_read_together_are_two() {
        let mut bytes = framed("first");
        bytes.extend(framed("second"));
        assert_eq!(
            Frames::default().push(&bytes).expect("well framed"),
            ["first", "second"]
        );
    }

    #[test]
    fn other_fields_are_let_by_and_names_are_read_in_any_case() {
        let bytes = b"content-type: application/json\r\nCONTENT-LENGTH:  2 \r\n\r\n{}";
        assert_eq!(Frames::default().push(bytes).expect("well framed"), ["{}"]);
    }

    #[test]
    fn a_header_whose_lines_end_in_newline_alone_is_read() {
        assert_eq!(
            Frames::default()
                .push(b"Content-Length: 2\n\n{}")
                .expect("well framed"),
            ["{}"]
        );
    }

    #[test]
    fn a_message_half_read_is_kept_for_the_next_read() {
        let mut frames = Frames::default();
        assert!(
            frames
                .push(b"Content-Length: 4\r\n\r\nab")
                .unwrap()
                .is_empty()
        );
        assert!(!frames.is_empty());
        assert_eq!(frames.push(b"cd").unwrap(), ["abcd"]);
    }

    #[test]
    fn a_header_without_a_length_is_refused() {
        let why = Frames::default()
            .push(b"Content-Type: x\r\n\r\n{}")
            .unwrap_err();
        assert!(why.contains("without Content-Length"), "{why}");
    }

    #[test]
    fn a_length_that_is_not_a_number_is_refused() {
        let why = Frames::default()
            .push(b"Content-Length: two\r\n\r\n")
            .unwrap_err();
        assert!(why.contains("not a length"), "{why}");
    }

    #[test]
    fn a_line_that_is_not_a_field_is_refused() {
        assert!(Frames::default().push(b"hello\r\n\r\n").is_err());
    }

    #[test]
    fn a_header_that_never_ends_is_refused_rather_than_kept() {
        let endless = vec![b'x'; MAX_HEADER + 1];
        assert!(Frames::default().push(&endless).is_err());
    }

    #[test]
    fn a_message_longer_than_the_longest_is_refused_before_it_is_read() {
        let why = Frames::default()
            .push(b"Content-Length: 99999999999\r\n\r\n")
            .unwrap_err();
        assert!(why.contains("more than"), "{why}");
    }

    // ----------------------------------------------------------------
    // Real processes
    // ----------------------------------------------------------------

    #[cfg(unix)]
    mod running {
        use crate::buffer::gap_buffer::GapBuffer;
        use crate::editor::processes::{PASS_DELIVERIES, QUEUE_ENTRIES};
        use crate::editor::{EditorState, create_global_env};
        use risp::{Env, EvalError, LispExp, Parser, eval};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        type Ctx = EditorState<GapBuffer>;

        fn eval_str(
            src: &str,
            env: &Arc<Env<Ctx>>,
            ctx: &Ctx,
        ) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
            let ast = Parser::new(&format!("(progn {src})"))
                .next()
                .expect("test source must parse");
            // A budget of its own, as the host gives each command: a function
            // stopped for running out of its own leaves the meter empty.
            let _command = ctx.begin_command();
            eval(&ast, env.clone(), ctx)
        }

        fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
            eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
        }

        /// An editor with `record`, a function that keeps what it is given:
        /// the outputs, the errors and the kinds in the order they came, and
        /// the exit in `ended`.
        fn editor() -> (Ctx, Arc<Env<Ctx>>) {
            let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
            run(
                "(defvar outputs nil) (defvar errors nil) (defvar kinds nil) (defvar ended nil)
                 (defun record (name kind value)
                   (setq kinds (cons kind kinds))
                   (if (eq kind 'output)
                       (setq outputs (cons value outputs))
                     (if (eq kind 'error)
                         (setq errors (cons value errors))
                       (setq ended value))))",
                &env,
                &ctx,
            );
            (ctx, env)
        }

        fn strings(items: &[&str]) -> LispExp<Ctx> {
            LispExp::proper_list(
                items
                    .iter()
                    .map(|item| LispExp::string(item.to_string()))
                    .collect(),
            )
        }

        fn exited(code: i32) -> LispExp<Ctx> {
            LispExp::proper_list(vec![
                LispExp::symbol("exit".to_string()),
                LispExp::number(code as f64),
            ])
        }

        fn killed() -> LispExp<Ctx> {
            LispExp::symbol("killed".to_string())
        }

        /// Runs the host's ticks until CONDITION, a Lisp form, is true, or
        /// gives up.
        fn tick_until(condition: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !run(condition, env, ctx).is_truthy() {
                assert!(
                    Instant::now() < deadline,
                    "{condition} did not come true within ten seconds"
                );
                if !ctx.tick(env) {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }

        /// Runs the host's ticks until the exit has been delivered.
        fn settle(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
            tick_until("ended", env, ctx);
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

        /// Waits until no process is running COMMAND, or gives up.
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
        fn lines_arrive_without_their_ends_and_the_exit_comes_last() {
            let (ctx, env) = editor();
            let name = run(
                r#"(process-start 'p "printf" '("a\nb\r\nc") 'record)"#,
                &env,
                &ctx,
            );
            assert_eq!(name, LispExp::string("p".to_string()));
            settle(&env, &ctx);
            assert_eq!(
                run("(reverse outputs)", &env, &ctx),
                strings(&["a", "b", "c"])
            );
            assert_eq!(run("ended", &env, &ctx), exited(0));
            assert_eq!(
                run("(car kinds)", &env, &ctx),
                LispExp::symbol("exit".to_string())
            );
        }

        #[test]
        fn the_exit_code_is_delivered_and_the_status_keeps_it() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "/bin/sh" '("-c" "exit 3") 'record)"#,
                &env,
                &ctx,
            );
            settle(&env, &ctx);
            assert_eq!(run("ended", &env, &ctx), exited(3));
            assert_eq!(run("(process-status 'p)", &env, &ctx), exited(3));
        }

        #[test]
        fn standard_error_arrives_apart_from_the_output() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "/bin/sh" '("-c" "echo out; echo err >&2") 'record)"#,
                &env,
                &ctx,
            );
            settle(&env, &ctx);
            assert_eq!(run("outputs", &env, &ctx), strings(&["out"]));
            assert_eq!(run("errors", &env, &ctx), strings(&["err"]));
        }

        #[test]
        fn arguments_reach_the_program_as_they_are_with_no_shell() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "printf" '("%s" "a b; $HOME '*'") 'record 'chunks)"#,
                &env,
                &ctx,
            );
            settle(&env, &ctx);
            assert_eq!(run("outputs", &env, &ctx), strings(&["a b; $HOME '*'"]));
        }

        #[test]
        fn chunks_arrive_before_their_line_ends_and_a_stop_says_killed() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "/bin/sh" '("-c" "printf 'ask? '; sleep 30.75") 'record 'chunks)"#,
                &env,
                &ctx,
            );
            tick_until("outputs", &env, &ctx);
            assert_eq!(run("outputs", &env, &ctx), strings(&["ask? "]));
            assert_eq!(
                run("(process-status 'p)", &env, &ctx),
                LispExp::symbol("run".to_string())
            );
            assert!(run("(process-stop 'p)", &env, &ctx).is_truthy());
            settle(&env, &ctx);
            assert_eq!(run("ended", &env, &ctx), killed());
            assert_eq!(run("(process-status 'p)", &env, &ctx), killed());
            gone("sleep 30.75");
        }

        #[test]
        fn what_is_sent_comes_back_and_closing_the_input_ends_it() {
            let (ctx, env) = editor();
            run(r#"(process-start 'p "cat" nil 'record)"#, &env, &ctx);
            assert!(run(r#"(process-send 'p "one\ntwo\n")"#, &env, &ctx).is_truthy());
            tick_until("(cdr outputs)", &env, &ctx);
            assert!(run("(process-close-input 'p)", &env, &ctx).is_truthy());
            assert!(run("(process-close-input 'p)", &env, &ctx).is_nil());
            assert!(run(r#"(process-send 'p "three\n")"#, &env, &ctx).is_nil());
            settle(&env, &ctx);
            assert_eq!(
                run("(reverse outputs)", &env, &ctx),
                strings(&["one", "two"])
            );
            assert_eq!(run("ended", &env, &ctx), exited(0));
        }

        #[test]
        fn messages_are_framed_going_and_coming() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "cat" nil 'record 'messages)"#,
                &env,
                &ctx,
            );
            run(r#"(process-send 'p "{\"è\":1}")"#, &env, &ctx);
            run(r#"(process-send 'p "second")"#, &env, &ctx);
            tick_until("(cdr outputs)", &env, &ctx);
            run("(process-close-input 'p)", &env, &ctx);
            settle(&env, &ctx);
            assert_eq!(
                run("(reverse outputs)", &env, &ctx),
                strings(&["{\"è\":1}", "second"])
            );
            assert!(run("errors", &env, &ctx).is_nil());
        }

        #[test]
        fn output_that_is_not_a_message_is_an_error_and_stops_the_process() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "/bin/sh" '("-c" "printf 'Content-Type: x\r\n\r\n{}'; sleep 31.5") 'record 'messages)"#,
                &env,
                &ctx,
            );
            settle(&env, &ctx);
            let errors = format!("{:?}", run("errors", &env, &ctx));
            assert!(errors.contains("without Content-Length"), "{errors}");
            assert!(run("outputs", &env, &ctx).is_nil());
            assert_eq!(run("ended", &env, &ctx), killed());
            gone("sleep 31.5");
        }

        #[test]
        fn the_queue_is_bounded_a_pass_is_bounded_and_nothing_is_lost() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "seq" '("1" "20000") 'record)"#,
                &env,
                &ctx,
            );
            let queued = || ctx.dialogue("p").expect("started").exchange.queued();
            // Nobody ticks: the reader fills the queue, and waits.
            let deadline = Instant::now() + Duration::from_secs(10);
            while queued() < QUEUE_ENTRIES {
                assert!(Instant::now() < deadline, "the queue never filled");
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(queued(), QUEUE_ENTRIES);
            assert!(ctx.callbacks_owed(), "the host is told there is work");

            assert!(ctx.tick(&env));
            let first_pass = run("(length outputs)", &env, &ctx);
            assert!(
                matches!(first_pass, LispExp::Number(n) if n >= 1.0 && n <= PASS_DELIVERIES as f64),
                "{first_pass:?}"
            );

            settle(&env, &ctx);
            let all: Vec<String> = (1..=20000).map(|n| n.to_string()).collect();
            let all: Vec<&str> = all.iter().map(String::as_str).collect();
            assert_eq!(run("(reverse outputs)", &env, &ctx), strings(&all));
            assert_eq!(run("ended", &env, &ctx), exited(0));
            assert!(!ctx.callbacks_owed());
        }

        #[test]
        fn a_function_that_never_returns_is_stopped_and_delivery_goes_on() {
            let (ctx, env) = editor();
            run(
                "(defun stuck (name kind value)
                   (if (eq kind 'exit) (setq ended value) (while t nil)))",
                &env,
                &ctx,
            );
            run(
                r#"(process-start 'p "printf" '("a\nb\n") 'stuck)"#,
                &env,
                &ctx,
            );
            // Each output must exhaust its budget. Keep that work small so it
            // finishes within settle's deadline even when the suite is busy.
            ctx.set_fuel_budget(1_000);
            settle(&env, &ctx);
            assert_eq!(run("ended", &env, &ctx), exited(0));
            let logged = ctx.get_logs().join("\n");
            assert_eq!(
                logged
                    .matches("the function of process p failed: OutOfFuel")
                    .count(),
                2,
                "{logged}"
            );
        }

        #[test]
        fn a_name_in_use_is_refused_until_its_exit_is_delivered() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "sleep" '("32.25") 'record)"#,
                &env,
                &ctx,
            );
            assert!(eval_str(r#"(process-start 'p "true" nil 'record)"#, &env, &ctx).is_err());
            run("(process-stop 'p)", &env, &ctx);
            settle(&env, &ctx);
            run("(setq ended nil)", &env, &ctx);
            assert_eq!(
                run(r#"(process-start 'p "true" nil 'record)"#, &env, &ctx),
                LispExp::string("p".to_string())
            );
            settle(&env, &ctx);
            assert_eq!(run("ended", &env, &ctx), exited(0));
            gone("sleep 32.25");
        }

        #[test]
        fn a_name_another_work_has_is_refused() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "sleep" '("32.75") 'record)"#,
                &env,
                &ctx,
            );
            // The reading thread has the name among the editor's work.
            assert!(ctx.work().is_named("p"));
            run(r#"(stop-worker 'p)"#, &env, &ctx);
            settle(&env, &ctx);
            assert_eq!(run("ended", &env, &ctx), killed());
            gone("sleep 32.75");
        }

        #[test]
        fn a_program_that_cannot_be_started_is_nil_and_said() {
            let (ctx, env) = editor();
            let started = run(
                r#"(process-start 'p "rsedit-no-such-program" nil 'record)"#,
                &env,
                &ctx,
            );
            assert!(started.is_nil());
            assert!(
                ctx.get_echo_message()
                    .contains("Cannot run rsedit-no-such-program")
            );
            assert!(run("(process-status 'p)", &env, &ctx).is_nil());
        }

        #[test]
        fn a_process_that_is_not_there_answers_nil() {
            let (ctx, env) = editor();
            for form in [
                r#"(process-send 'nobody "x")"#,
                "(process-close-input 'nobody)",
                "(process-stop 'nobody)",
                "(process-status 'nobody)",
            ] {
                assert!(run(form, &env, &ctx).is_nil(), "{form}");
            }
        }

        #[test]
        fn a_form_that_is_none_of_the_three_is_refused() {
            let (ctx, env) = editor();
            assert!(
                eval_str(
                    r#"(process-start 'p "true" nil 'record 'words)"#,
                    &env,
                    &ctx
                )
                .is_err()
            );
            assert!(run("(process-status 'p)", &env, &ctx).is_nil());
        }

        #[test]
        fn writing_to_a_process_that_does_not_read_is_refused_rather_than_waited_for() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "sleep" '("36.5") 'record)"#,
                &env,
                &ctx,
            );
            run(r#"(setq block (make-string 65536 "x"))"#, &env, &ctx);
            let began = Instant::now();
            let mut refused = false;
            for _ in 0..crate::editor::processes::INPUT_QUEUE + 64 {
                if run("(process-send 'p block)", &env, &ctx).is_nil() {
                    refused = true;
                    break;
                }
            }
            assert!(refused, "the input queue has no end");
            assert!(
                began.elapsed() < Duration::from_secs(5),
                "{:?}",
                began.elapsed()
            );
            run("(process-stop 'p)", &env, &ctx);
            settle(&env, &ctx);
            gone("sleep 36.5");
        }

        #[test]
        fn a_process_starts_in_the_working_directory() {
            let root = std::env::temp_dir().join(format!("rsedit-process-{}", std::process::id()));
            std::fs::create_dir_all(&root).expect("directory");
            let physical = root.canonicalize().expect("exists").display().to_string();
            let (ctx, env) = editor();
            run(&format!(r#"(cd "{}")"#, root.display()), &env, &ctx);
            run(r#"(process-start 'p "pwd" '("-P") 'record)"#, &env, &ctx);
            settle(&env, &ctx);
            let _ = std::fs::remove_dir_all(&root);
            assert_eq!(run("outputs", &env, &ctx), strings(&[physical.as_str()]));
        }

        #[test]
        fn a_process_does_not_outlive_the_editor() {
            let (ctx, env) = editor();
            run(
                r#"(process-start 'p "/bin/sh" '("-c" "trap '' TERM; echo started; sleep 34.75") 'record)"#,
                &env,
                &ctx,
            );
            tick_until("outputs", &env, &ctx);
            ctx.shutdown(Duration::from_millis(500));
            gone("sleep 34.75");
        }
    }
}
