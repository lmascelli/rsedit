//! `eshell.lisp`: a shell in a buffer. The filters that clean what a shell
//! prints and the history are checked on their own; the rest with real
//! shells, sh and bash, polled through the host's ticks as the processes'
//! tests are.
//!
//! Design: `doc/architettura-editor.typ, <mod-eshell>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        let _command = ctx.begin_command();
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// An editor with the module, its history kept in this run only.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/features/eshell.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        run("(setq eshell-history-save nil)", &env, &ctx);
        (ctx, env)
    }

    fn string(text: &str) -> LispExp<Ctx> {
        LispExp::string(text.to_string())
    }

    fn strings(items: &[&str]) -> LispExp<Ctx> {
        LispExp::proper_list(items.iter().map(|item| string(item)).collect())
    }

    /// TEXT as a Lisp string literal.
    fn quoted(text: &str) -> String {
        format!(
            "\"{}\"",
            text.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\u{1b}', "\\e")
        )
    }

    /// A buffer in eshell-mode with no shell, its marker at the end of TEXT.
    fn shell_buffer(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        run(
            &format!(
                r#"(buffer-create "*eshell*" :mode 'eshell-mode)
                   (switch-to-buffer "*eshell*")
                   (insert {})
                   (buffer-put 'eshell--marker (make-marker (point-max)))"#,
                quoted(text)
            ),
            env,
            ctx,
        );
    }

    fn contents(ctx: &Ctx, name: &str) -> String {
        ctx.with_buffer(name, |b| b.text.to_string())
            .unwrap_or_else(|| panic!("no buffer called {name}"))
    }

    // ----------------------------------------------------------------
    // What a shell prints, cleaned
    // ----------------------------------------------------------------

    #[test]
    fn colours_and_other_control_sequences_are_taken_out() {
        let (ctx, env) = editor();
        shell_buffer("", &env, &ctx);
        run(
            &format!(
                "(eshell--output {})",
                quoted("\u{1b}[1;31mred\u{1b}[0m plain\n")
            ),
            &env,
            &ctx,
        );
        assert_eq!(contents(&ctx, "*eshell*"), "red plain\n");
    }

    #[test]
    fn a_shell_that_says_where_it_is_moves_the_buffers_directory() {
        let (ctx, env) = editor();
        shell_buffer("", &env, &ctx);
        let root = std::env::temp_dir().display().to_string();
        let osc = format!("\u{1b}]7;file://host{root}\u{1b}\\$ ");
        run(&format!("(eshell--output {})", quoted(&osc)), &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "$ ");
        assert_eq!(run("(working-directory)", &env, &ctx), string(&root));
    }

    #[test]
    fn an_escape_that_ends_a_sequence_early_takes_nothing_after_it() {
        let (ctx, env) = editor();
        shell_buffer("", &env, &ctx);
        let root = std::env::temp_dir().display().to_string();
        // A prompt whose ESC \ lost its backslash, as a shell may print it.
        let broken = format!("\u{1b}]7;file://host{root}\u{1b}$ ");
        run(&format!("(eshell--output {})", quoted(&broken)), &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "$ ");
        assert_eq!(run("(working-directory)", &env, &ctx), string(&root));
    }

    #[test]
    fn character_sets_and_two_character_escapes_are_taken_out() {
        let (ctx, env) = editor();
        shell_buffer("", &env, &ctx);
        run(
            &format!(
                "(eshell--output {})",
                quoted("a\u{1b}(B\u{1b}[mb\u{1b}=c")
            ),
            &env,
            &ctx,
        );
        assert_eq!(contents(&ctx, "*eshell*"), "abc");
    }

    #[test]
    fn a_windows_path_in_a_file_url_is_read_as_a_drive() {
        let (ctx, env) = editor();
        assert_eq!(
            run(r#"(eshell--directory "file:///C:\\Users\\me")"#, &env, &ctx),
            string("C:\\Users\\me")
        );
        assert_eq!(
            run(r#"(eshell--directory "file://host/home/me")"#, &env, &ctx),
            string("/home/me")
        );
    }

    #[test]
    fn an_escape_cut_in_two_by_a_read_is_taken_out_whole() {
        let (ctx, env) = editor();
        shell_buffer("", &env, &ctx);
        run(
            &format!("(eshell--output {})", quoted("one\u{1b}[3")),
            &env,
            &ctx,
        );
        assert_eq!(contents(&ctx, "*eshell*"), "one");
        run(&format!("(eshell--output {})", quoted("2mtwo")), &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "onetwo");
    }

    #[test]
    fn line_ends_of_windows_become_newlines() {
        let (ctx, env) = editor();
        shell_buffer("", &env, &ctx);
        run(r#"(eshell--output "a\r\nb\r\n")"#, &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "a\nb\n");
    }

    #[test]
    fn a_line_end_of_windows_cut_in_two_by_a_read_is_one_newline() {
        let (ctx, env) = editor();
        shell_buffer("", &env, &ctx);
        run(r#"(eshell--output "a\r")"#, &env, &ctx);
        run(r#"(eshell--output "\nb\r\n")"#, &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "a\nb\n");
    }

    #[test]
    fn the_copy_of_the_line_sent_that_a_shell_echoes_is_dropped() {
        let (ctx, env) = editor();
        shell_buffer("C:\\>", &env, &ctx);
        run(r#"(buffer-put 'eshell--echo "dir\n")"#, &env, &ctx);
        // In two pieces, as two reads may bring it.
        run(r#"(eshell--output "di")"#, &env, &ctx);
        run(r#"(eshell--output "r\r\n file.txt\r\n")"#, &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "C:\\> file.txt\n");
    }

    #[test]
    fn output_goes_before_what_is_being_typed_and_point_stays_on_it() {
        let (ctx, env) = editor();
        shell_buffer("$ ", &env, &ctx);
        run(
            r#"(insert "half") (goto-char (- (point-max) 2))"#,
            &env,
            &ctx,
        );
        run(r#"(eshell--output "late output\n$ ")"#, &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "$ late output\n$ half");
        assert_eq!(run("(eshell--input)", &env, &ctx), string("half"));
        assert_eq!(
            run("(buffer-substring (point) (point-max))", &env, &ctx),
            string("lf")
        );
    }

    #[test]
    fn output_leaves_point_alone_when_it_is_up_in_what_was_printed() {
        let (ctx, env) = editor();
        shell_buffer("first\n$ ", &env, &ctx);
        run("(goto-char 2)", &env, &ctx);
        run(r#"(eshell--output "more\n")"#, &env, &ctx);
        assert_eq!(run("(point)", &env, &ctx), LispExp::number(2.0));
    }

    // ----------------------------------------------------------------
    // History
    // ----------------------------------------------------------------

    #[test]
    fn a_line_like_the_one_before_is_kept_once_by_default() {
        let (ctx, env) = editor();
        run(
            r#"(eshell--remember "ls") (eshell--remember "ls") (eshell--remember "pwd")
               (eshell--remember "ls") (eshell--remember "  ")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(eshell-history)", &env, &ctx),
            strings(&["ls", "pwd", "ls"])
        );
    }

    #[test]
    fn erase_keeps_only_the_newest_copy_and_all_keeps_every_one() {
        let (ctx, env) = editor();
        run(
            r#"(setq eshell-history-duplicates 'erase)
               (eshell--remember "ls") (eshell--remember "pwd") (eshell--remember "ls")"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(eshell-history)", &env, &ctx), strings(&["ls", "pwd"]));
        run(
            r#"(setq eshell-history-duplicates 'all) (eshell--remember "ls")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(eshell-history)", &env, &ctx),
            strings(&["ls", "ls", "pwd"])
        );
    }

    #[test]
    fn deleting_the_duplicates_keeps_the_newest_copy_of_each_line() {
        let (ctx, env) = editor();
        run(
            r#"(setq eshell-history-duplicates 'all)
               (dolist (line '("a" "b" "a" "c" "b")) (eshell--remember line))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(eshell-history-delete-duplicates)", &env, &ctx),
            LispExp::number(2.0)
        );
        assert_eq!(
            run("(eshell-history)", &env, &ctx),
            strings(&["b", "c", "a"])
        );
    }

    #[test]
    fn the_history_is_bounded_and_skips_what_the_ignore_regexp_matches() {
        let (ctx, env) = editor();
        run(
            r#"(setq eshell-history-size 2) (setq eshell-history-ignore "^ ")
               (dolist (line '("one" " secret" "two" "three")) (eshell--remember line))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(eshell-history)", &env, &ctx),
            strings(&["three", "two"])
        );
    }

    #[test]
    fn the_history_is_kept_for_the_next_run_when_asked() {
        let (ctx, env) = editor();
        run(
            r#"(setq eshell-history-save t) (eshell--remember "make")"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(session-value 'eshell-history)", &env, &ctx),
            strings(&["make"])
        );
    }

    #[test]
    fn m_p_and_m_n_walk_the_history_and_give_back_what_was_typed() {
        let (ctx, env) = editor();
        shell_buffer("$ ", &env, &ctx);
        run(
            r#"(eshell--remember "first") (eshell--remember "second") (insert "draft")"#,
            &env,
            &ctx,
        );
        run("(eshell-previous-input)", &env, &ctx);
        assert_eq!(run("(eshell--input)", &env, &ctx), string("second"));
        run("(eshell-previous-input)", &env, &ctx);
        assert_eq!(run("(eshell--input)", &env, &ctx), string("first"));
        run("(eshell-next-input) (eshell-next-input)", &env, &ctx);
        assert_eq!(run("(eshell--input)", &env, &ctx), string("draft"));
        assert_eq!(contents(&ctx, "*eshell*"), "$ draft");
    }

    // ----------------------------------------------------------------
    // Lisp at the prompt
    // ----------------------------------------------------------------

    #[test]
    fn a_line_after_the_prefix_is_evaluated_and_the_prompt_comes_again() {
        let (ctx, env) = editor();
        shell_buffer("$ ", &env, &ctx);
        run(r#"(insert ",(+ 1 2)") (eshell-send-input)"#, &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "$ ,(+ 1 2)\n3\n$ ");
        assert_eq!(run("(eshell--input)", &env, &ctx), string(""));
        assert_eq!(run("(eshell-history)", &env, &ctx), strings(&[",(+ 1 2)"]));
    }

    #[test]
    fn a_lisp_error_is_written_and_a_string_shown_as_it_is() {
        let (ctx, env) = editor();
        shell_buffer("> ", &env, &ctx);
        run(
            r#"(insert ",(no-such-function)") (eshell-send-input)
               (insert ",(concat \"a\" \"b\")") (eshell-send-input)"#,
            &env,
            &ctx,
        );
        let text = contents(&ctx, "*eshell*");
        assert!(text.contains("\nerror: "), "{text}");
        assert!(text.ends_with(",(concat \"a\" \"b\")\nab\n> "), "{text}");
    }

    #[test]
    fn return_on_an_earlier_line_copies_it_after_the_prompt_without_its_prompt() {
        let (ctx, env) = editor();
        shell_buffer("$ ls -l\nfile\n$ ", &env, &ctx);
        run("(goto-char 3) (eshell-send-input)", &env, &ctx);
        assert_eq!(run("(eshell--input)", &env, &ctx), string("ls -l"));
        assert_eq!(run("(point)", &env, &ctx), run("(point-max)", &env, &ctx));
    }

    #[test]
    fn a_line_for_a_shell_that_has_ended_is_not_taken() {
        let (ctx, env) = editor();
        shell_buffer("$ ", &env, &ctx);
        run(r#"(insert "ls") (eshell-send-input)"#, &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "$ ls");
        assert!(run("(eshell-history)", &env, &ctx).is_nil());
    }

    // ----------------------------------------------------------------
    // Real shells
    // ----------------------------------------------------------------

    #[cfg(unix)]
    mod running {
        use super::*;
        use std::time::{Duration, Instant};

        /// Runs the host's ticks until CONDITION, a Lisp form, is true, or
        /// gives up saying what the buffer holds.
        fn tick_until(condition: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !run(condition, env, ctx).is_truthy() {
                assert!(
                    Instant::now() < deadline,
                    "{condition} did not come true within ten seconds; the buffer: {:?}",
                    ctx.with_buffer("*eshell*", |b| b.text.to_string())
                );
                if !ctx.tick(env) {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }

        /// Waits for the buffer to end with the prompt after TIMES prompts in
        /// all.
        fn prompts(times: usize, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
            tick_until(
                &format!(
                    r#"(let ((text (buffer-string)) (count 0) (from 0))
                         (while (string-match "\\$ " text from)
                           (setq count (+ count 1))
                           (setq from (match-end 0)))
                         (and (>= count {times}) (string-suffix-p "$ " text)))"#
                ),
                env,
                ctx,
            );
        }

        /// Types LINE after the prompt, and presses Return.
        fn enter(line: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
            run(
                &format!(
                    "(goto-char (point-max)) (insert {}) (eshell-send-input)",
                    quoted(line)
                ),
                env,
                ctx,
            );
        }

        /// Starts sh in *eshell*, shown, and waits for its first prompt.
        fn sh(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
            assert_eq!(
                run(r#"(eshell-start 'sh "*eshell*")"#, env, ctx),
                string("*eshell*")
            );
            run(r#"(switch-to-buffer "*eshell*")"#, env, ctx);
            prompts(1, env, ctx);
        }

        fn stop(env: &Arc<Env<Ctx>>, ctx: &Ctx) {
            run("(process-stop \"*eshell*\")", env, ctx);
            tick_until("(null (eshell-process))", env, ctx);
        }

        #[test]
        fn what_is_typed_goes_to_the_shell_and_what_it_prints_comes_back() {
            let (ctx, env) = editor();
            sh(&env, &ctx);
            assert_eq!(run("(eshell-process)", &env, &ctx), string("*eshell*"));
            enter("echo hello", &env, &ctx);
            prompts(2, &env, &ctx);
            let text = contents(&ctx, "*eshell*");
            assert!(text.ends_with("$ echo hello\nhello\n$ "), "{text:?}");
            assert!(!text.contains('\u{1b}'), "{text:?}");
            stop(&env, &ctx);
        }

        #[test]
        fn errors_come_in_the_order_they_were_printed() {
            let (ctx, env) = editor();
            sh(&env, &ctx);
            enter("echo one; echo two >&2; echo three", &env, &ctx);
            prompts(2, &env, &ctx);
            assert!(contents(&ctx, "*eshell*").ends_with("one\ntwo\nthree\n$ "));
            stop(&env, &ctx);
        }

        #[test]
        fn the_buffer_follows_the_shell_from_directory_to_directory() {
            let (ctx, env) = editor();
            let root = std::env::temp_dir().canonicalize().expect("a temp dir");
            sh(&env, &ctx);
            enter(&format!("cd {}", root.display()), &env, &ctx);
            prompts(2, &env, &ctx);
            assert_eq!(
                run("(working-directory)", &env, &ctx),
                string(&root.display().to_string())
            );
            stop(&env, &ctx);
        }

        #[test]
        fn an_interrupt_stops_the_command_and_the_shell_goes_on() {
            let (ctx, env) = editor();
            sh(&env, &ctx);
            enter("sleep 32.5", &env, &ctx);
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let listing = std::process::Command::new("ps")
                    .args(["-eo", "args="])
                    .output()
                    .expect("ps runs");
                if String::from_utf8_lossy(&listing.stdout)
                    .lines()
                    .any(|line| line.trim() == "sleep 32.5")
                {
                    break;
                }
                assert!(Instant::now() < deadline, "the sleep never started");
                std::thread::sleep(Duration::from_millis(5));
            }
            run("(eshell-interrupt)", &env, &ctx);
            prompts(2, &env, &ctx);
            enter("echo alive", &env, &ctx);
            prompts(3, &env, &ctx);
            assert!(contents(&ctx, "*eshell*").contains("\nalive\n"));
            stop(&env, &ctx);
        }

        #[test]
        fn a_shell_that_exits_says_how_and_the_buffer_stays() {
            let (ctx, env) = editor();
            sh(&env, &ctx);
            enter("exit 3", &env, &ctx);
            tick_until("(null (eshell-process))", &env, &ctx);
            assert!(
                contents(&ctx, "*eshell*").ends_with("\n--- exited 3 ---\n"),
                "{:?}",
                contents(&ctx, "*eshell*")
            );
            assert_eq!(
                run(r#"(process-status "*eshell*")"#, &env, &ctx),
                LispExp::proper_list(vec![LispExp::symbol("exit".into()), LispExp::number(3.0)])
            );
        }

        #[test]
        fn eshell_again_in_a_buffer_whose_shell_ended_starts_the_same_shell() {
            let (ctx, env) = editor();
            sh(&env, &ctx);
            run("(eshell-send-eof)", &env, &ctx);
            tick_until("(null (eshell-process))", &env, &ctx);
            assert_eq!(run("(eshell)", &env, &ctx), string("*eshell*"));
            assert_eq!(
                run(r#"(buffer-get 'eshell--shell)"#, &env, &ctx),
                LispExp::symbol("sh".into())
            );
            tick_until(r#"(string-suffix-p "$ " (buffer-string))"#, &env, &ctx);
            stop(&env, &ctx);
        }

        #[test]
        fn closing_the_buffer_stops_its_shell() {
            let (ctx, env) = editor();
            sh(&env, &ctx);
            run(r#"(kill-buffer "*eshell*")"#, &env, &ctx);
            tick_until(r#"(eq (process-status "*eshell*") 'killed)"#, &env, &ctx);
        }

        #[test]
        fn a_lisp_line_is_not_sent_to_the_shell() {
            let (ctx, env) = editor();
            sh(&env, &ctx);
            enter(",(setq eshell-test-value 42)", &env, &ctx);
            enter("echo $?", &env, &ctx);
            prompts(3, &env, &ctx);
            let text = contents(&ctx, "*eshell*");
            assert!(text.ends_with("\n42\n$ echo $?\n0\n$ "), "{text:?}");
            stop(&env, &ctx);
        }

        /// Returns where PROGRAM is on the PATH, if it is.
        fn on_path(program: &str) -> Option<std::path::PathBuf> {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|directory| directory.join(program))
                .find(|candidate| candidate.is_file())
        }

        #[test]
        fn the_sh_shell_works_whichever_shell_sh_is() {
            // bash called sh, as /bin/sh is on Fedora, Arch and macOS: it reads
            // the backslashes of a prompt otherwise than dash does.
            let Some(bash) = on_path("bash") else {
                return;
            };
            let root = std::env::temp_dir().join(format!("rsedit-sh-{}", std::process::id()));
            std::fs::create_dir_all(&root).expect("directory");
            let sh = root.join("sh");
            let _ = std::fs::remove_file(&sh);
            std::os::unix::fs::symlink(&bash, &sh).expect("a link named sh");
            let (ctx, env) = editor();
            run(
                &format!(
                    r#"(setq eshell-shells
                         (cons (cons 'bash-as-sh
                                     (cons :program
                                           (cons {}
                                                 (cdr (cdr (cdr (assq 'sh eshell-shells)))))))
                               eshell-shells))
                       (eshell-start 'bash-as-sh "*eshell*")
                       (switch-to-buffer "*eshell*")"#,
                    quoted(&sh.display().to_string())
                ),
                &env,
                &ctx,
            );
            prompts(1, &env, &ctx);
            let physical = std::env::temp_dir().canonicalize().expect("a temp dir");
            enter(&format!("cd {}", physical.display()), &env, &ctx);
            prompts(2, &env, &ctx);
            enter("echo hello", &env, &ctx);
            prompts(3, &env, &ctx);
            let text = contents(&ctx, "*eshell*");
            let _ = std::fs::remove_dir_all(&root);
            assert!(text.ends_with("$ echo hello\nhello\n$ "), "{text:?}");
            assert!(!text.contains('\u{1b}') && !text.contains("7;file"), "{text:?}");
            assert_eq!(
                run("(working-directory)", &env, &ctx),
                string(&physical.display().to_string())
            );
            stop(&env, &ctx);
        }

        #[test]
        fn bash_runs_with_its_prompt_and_no_escape_left_in_the_buffer() {
            let (ctx, env) = editor();
            if std::process::Command::new("bash")
                .arg("-c")
                .arg("true")
                .status()
                .is_err()
            {
                return;
            }
            let handle = run(r#"(eshell-start 'bash "*eshell*")"#, &env, &ctx);
            assert_eq!(handle, string("*eshell*"));
            run(r#"(switch-to-buffer "*eshell*")"#, &env, &ctx);
            // Whatever the prompt is, it ends in $ or #.
            tick_until(r#"(string-match-p "[#$] $" (buffer-string))"#, &env, &ctx);
            enter("echo from-bash", &env, &ctx);
            tick_until(
                r#"(string-match-p "\nfrom-bash\n" (buffer-string))"#,
                &env,
                &ctx,
            );
            assert!(!contents(&ctx, "*eshell*").contains('\u{1b}'));
            stop(&env, &ctx);
        }
    }
}
