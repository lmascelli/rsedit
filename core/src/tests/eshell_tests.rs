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
            include_str!("../../lisp/features/completion-at-point.lisp"),
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
    // Completion
    // ----------------------------------------------------------------

    /// A directory of its own for one test, with FILES in it: a name ending
    /// in / is a directory.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(test: &str, files: &[&str]) -> Self {
            let root = std::env::temp_dir()
                .join(format!("rsedit-eshell-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("directory");
            for file in files {
                let path = root.join(file);
                if file.ends_with('/') {
                    std::fs::create_dir_all(&path).expect("directory");
                } else {
                    std::fs::write(&path, "").expect("file");
                }
            }
            Self(root)
        }

        fn path(&self) -> String {
            self.0.display().to_string()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A shell buffer for sh, in DIRECTORY, with LINE typed after its prompt.
    fn typed_at_prompt(line: &str, directory: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        shell_buffer("$ ", env, ctx);
        run(
            &format!(
                "(buffer-put 'eshell--shell 'sh) (set-buffer-directory {}) (insert {})",
                quoted(directory),
                quoted(line)
            ),
            env,
            ctx,
        );
    }

    /// What `eshell-completion' answers: the text it would replace, and the
    /// values it offers, each with its description.
    fn completion(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Option<(String, Vec<(String, String)>)> {
        let answer = run("(eshell-completion)", env, ctx);
        if answer.is_nil() {
            return None;
        }
        let parts: Vec<LispExp<Ctx>> = answer.iter().collect();
        let region = match run(
            &format!(
                "(buffer-substring {} {})",
                lisp_number(&parts[0]),
                lisp_number(&parts[1])
            ),
            env,
            ctx,
        ) {
            LispExp::String(text) => text.to_string(),
            other => panic!("{other:?}"),
        };
        let candidates = parts[2]
            .iter()
            .map(|candidate| match candidate {
                LispExp::String(value) => (value.to_string(), String::new()),
                LispExp::Cons(cell) => match (&cell.car, &cell.cdr) {
                    (LispExp::String(value), LispExp::String(about)) => {
                        (value.to_string(), about.to_string())
                    }
                    other => panic!("{other:?}"),
                },
                other => panic!("{other:?}"),
            })
            .collect();
        Some((region, candidates))
    }

    fn lisp_number(exp: &LispExp<Ctx>) -> String {
        match exp {
            LispExp::Number(n) => format!("{n}"),
            other => panic!("not a number: {other:?}"),
        }
    }

    fn values(candidates: &[(String, String)]) -> Vec<&str> {
        candidates.iter().map(|(value, _)| value.as_str()).collect()
    }

    #[test]
    fn a_line_is_read_into_words_as_a_posix_shell_reads_it() {
        let (ctx, env) = editor();
        assert_eq!(
            run(
                r#"(mapcar (lambda (word) (nth 1 word))
                           (eshell--words "make && ls 'a b' c\\ d \"e" (assq 'posix eshell--syntaxes)))"#,
                &env,
                &ctx
            ),
            strings(&["ls", "a b", "c d", "e"])
        );
        assert_eq!(
            run(
                r#"(eshell--words "ls " (assq 'posix eshell--syntaxes))"#,
                &env,
                &ctx
            ),
            run("'((0 \"ls\" nil) (3 \"\" nil))", &env, &ctx)
        );
    }

    #[test]
    fn an_argument_is_completed_over_the_files_of_the_shells_directory() {
        let (ctx, env) = editor();
        let here = Scratch::new("files", &["alpha.txt", "alpine/", "beta"]);
        typed_at_prompt("cat al", &here.path(), &env, &ctx);
        let (region, candidates) = completion(&env, &ctx).expect("an answer");
        assert_eq!(region, "al");
        assert_eq!(
            candidates,
            [
                ("alpha.txt".to_string(), "file".to_string()),
                ("alpine/".to_string(), "directory".to_string()),
                ("beta".to_string(), "file".to_string()),
            ]
        );
    }

    #[test]
    fn a_name_is_offered_escaped_or_after_the_quote_it_was_started_with() {
        let (ctx, env) = editor();
        let here = Scratch::new("quoting", &["My Documents/"]);
        typed_at_prompt("ls My", &here.path(), &env, &ctx);
        assert_eq!(
            values(&completion(&env, &ctx).expect("an answer").1),
            ["My\\ Documents/"]
        );
        run("(eshell-kill-input) (insert \"ls 'My\")", &env, &ctx);
        let (region, candidates) = completion(&env, &ctx).expect("an answer");
        assert_eq!(region, "'My");
        assert_eq!(values(&candidates), ["'My Documents/"]);
    }

    #[test]
    fn cd_is_offered_directories_alone_even_behind_sudo() {
        let (ctx, env) = editor();
        let here = Scratch::new("cd", &["alpha.txt", "alpine/"]);
        typed_at_prompt("sudo cd al", &here.path(), &env, &ctx);
        assert_eq!(
            values(&completion(&env, &ctx).expect("an answer").1),
            ["alpine/"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_command_is_completed_over_the_path_and_the_shells_own_commands() {
        let (ctx, env) = editor();
        let bin = Scratch::new("path", &["rsedit-test-tool", "subdirectory/"]);
        let here = Scratch::new("path-here", &[]);
        run(&format!("(setenv \"PATH\" {})", quoted(&bin.path())), &env, &ctx);
        // After a ; a command starts again. What is offered is all of them:
        // matching what is typed is `completion-at-point''s.
        typed_at_prompt("echo hi; rsedit-te", &here.path(), &env, &ctx);
        let (region, candidates) = completion(&env, &ctx).expect("an answer");
        assert_eq!(region, "rsedit-te");
        assert!(
            candidates.contains(&("rsedit-test-tool".to_string(), "command".to_string())),
            "{candidates:?}"
        );
        assert!(
            candidates.contains(&("export".to_string(), "builtin".to_string())),
            "{candidates:?}"
        );
        assert!(!values(&candidates).contains(&"subdirectory/"));
        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "$ echo hi; rsedit-test-tool");
    }

    #[test]
    fn a_variable_is_completed_over_the_environment() {
        let (ctx, env) = editor();
        let here = Scratch::new("variables", &[]);
        run(r#"(setenv "RSEDIT_ESHELL_VARIABLE" "x")"#, &env, &ctx);
        typed_at_prompt("echo $RSEDIT_ESH", &here.path(), &env, &ctx);
        let (region, candidates) = completion(&env, &ctx).expect("an answer");
        assert_eq!(region, "$RSEDIT_ESH");
        assert!(candidates.contains(&(
            "$RSEDIT_ESHELL_VARIABLE".to_string(),
            "variable".to_string()
        )));
        run("(eshell-kill-input) (insert \"echo ${RSEDIT_ESH\")", &env, &ctx);
        assert!(
            values(&completion(&env, &ctx).expect("an answer").1)
                .contains(&"${RSEDIT_ESHELL_VARIABLE}")
        );
    }

    #[test]
    fn make_is_offered_the_targets_of_the_makefile() {
        let (ctx, env) = editor();
        let here = Scratch::new("make", &[]);
        std::fs::write(
            here.0.join("Makefile"),
            "build: main.o\n\tcc -o x\ntest: build\nVARIABLE := 1\n.PHONY: build\n%.o: %.c\n",
        )
        .expect("a makefile");
        typed_at_prompt("make ", &here.path(), &env, &ctx);
        assert_eq!(
            completion(&env, &ctx).expect("an answer").1,
            [
                ("build".to_string(), "target".to_string()),
                ("test".to_string(), "target".to_string()),
            ]
        );
    }

    #[test]
    fn git_is_offered_its_commands_then_its_branches() {
        let (ctx, env) = editor();
        let here = Scratch::new("git", &[".git/refs/heads/feature/", ".git/refs/tags/"]);
        std::fs::write(here.0.join(".git/refs/heads/main"), "0\n").expect("a ref");
        std::fs::write(here.0.join(".git/refs/heads/feature/x"), "0\n").expect("a ref");
        std::fs::write(
            here.0.join(".git/packed-refs"),
            "# pack-refs\n1111 refs/tags/v1\n2222 refs/heads/old\n",
        )
        .expect("packed refs");
        typed_at_prompt("git ch", &here.path(), &env, &ctx);
        let commands = completion(&env, &ctx).expect("an answer").1;
        assert!(values(&commands).contains(&"checkout"), "{commands:?}");
        run("(eshell-kill-input) (insert \"git checkout \")", &env, &ctx);
        let refs = completion(&env, &ctx).expect("an answer").1;
        for wanted in [("main", "branch"), ("feature/x", "branch"), ("v1", "tag"), ("old", "branch")] {
            assert!(
                refs.contains(&(wanted.0.to_string(), wanted.1.to_string())),
                "{wanted:?} not in {refs:?}"
            );
        }
    }

    #[test]
    fn the_hosts_of_ssh_come_from_its_configuration_and_known_hosts() {
        let (ctx, env) = editor();
        assert_eq!(
            run(
                r#"(eshell--hosts-of "Host build box\nHost *.example\n  HostName x\n"
                                     "box,10.0.0.1 ssh-ed25519 A\n[git.example]:2222 ssh-rsa B\n|1|hash= ssh-rsa C\n")"#,
                &env,
                &ctx
            ),
            strings(&["build", "box", "10.0.0.1", "git.example"])
        );
    }

    #[test]
    fn a_lisp_line_is_completed_over_the_editors_names() {
        let (ctx, env) = editor();
        let here = Scratch::new("lisp", &[]);
        typed_at_prompt(",(eshell-history-del", &here.path(), &env, &ctx);
        let (region, candidates) = completion(&env, &ctx).expect("an answer");
        assert_eq!(region, "eshell-history-del");
        assert!(values(&candidates).contains(&"eshell-history-delete-duplicates"));
    }

    #[test]
    fn nothing_is_completed_above_the_prompt() {
        let (ctx, env) = editor();
        let here = Scratch::new("above", &["alpha"]);
        typed_at_prompt("ls al", &here.path(), &env, &ctx);
        run("(goto-char 1)", &env, &ctx);
        assert!(completion(&env, &ctx).is_none());
    }

    #[test]
    fn the_shells_completion_is_the_only_one_its_buffer_is_offered() {
        let (ctx, env) = editor();
        let here = Scratch::new("only", &["alpha.txt"]);
        // A global source that would offer a second candidate for the same
        // text, were it asked.
        run(
            "(defun capf-everywhere ()
               (let ((bounds (bounds-of-thing-at-point 'symbol)))
                 (list (nth 0 bounds) (nth 1 bounds) (list \"alphabet\"))))
             (add-completion-function nil 'capf-everywhere)",
            &env,
            &ctx,
        );
        typed_at_prompt("cat alph", &here.path(), &env, &ctx);
        run("(completion-at-point)", &env, &ctx);
        assert_eq!(contents(&ctx, "*eshell*"), "$ cat alpha.txt");
        assert_eq!(
            run("(car (nth 1 (key-binding \"<tab>\")))", &env, &ctx),
            LispExp::symbol("completion-at-point".into())
        );
    }

    // ----------------------------------------------------------------
    // Real shells
    // ----------------------------------------------------------------

    // ----------------------------------------------------------------
    // A password asked for
    // ----------------------------------------------------------------

    #[test]
    fn the_lines_that_ask_for_a_password_are_told_from_the_others() {
        let (ctx, env) = editor();
        for (line, asks) in [
            ("Password: ", true),
            ("[sudo] password for leo: ", true),
            ("[sudo] password di leo: ", true),
            (
                "Enter passphrase for key '/home/leo/.ssh/id_ed25519': ",
                true,
            ),
            ("Password for 'https://leo@example.com': ", true),
            ("Passwort:", true),
            ("passwd: password updated successfully", false),
            ("$ ", false),
            ("Username: ", false),
        ] {
            let matched = run(
                &format!(
                    "(string-match-p eshell-password-prompt-regexp {})",
                    quoted(line)
                ),
                &env,
                &ctx,
            );
            assert_eq!(matched.is_truthy(), asks, "{line:?}");
        }
    }

    #[test]
    fn a_password_prompt_is_seen_once_where_the_output_ends() {
        let (ctx, env) = editor();
        shell_buffer("", &env, &ctx);
        // Watched only in a buffer whose shell runs.
        run(r#"(buffer-put 'eshell--process "*eshell*")"#, &env, &ctx);
        let prompt = "[sudo] password for leo: ";
        assert_eq!(
            run(&format!("(eshell--output {})", quoted(prompt)), &env, &ctx),
            string(prompt)
        );
        // Nothing more on that line: not asked again.
        assert!(run(r#"(eshell--output "")"#, &env, &ctx).is_nil());
        // A second try, on a line of its own, is.
        assert_eq!(
            run(
                &format!(
                    "(eshell--output {})",
                    quoted(&format!("\nSorry, try again.\n{prompt}"))
                ),
                &env,
                &ctx
            ),
            string(prompt)
        );
        // A line that ends is not a question.
        assert!(run(r#"(eshell--output "\nPassword: \n")"#, &env, &ctx).is_nil());
        run("(setq eshell-watch-for-password-prompt nil)", &env, &ctx);
        assert!(run(r#"(eshell--output "Password: ")"#, &env, &ctx).is_nil());
    }

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
        fn a_password_asked_for_is_read_hidden_and_reaches_the_program() {
            use crate::input::{KeyCode, KeyEvent};
            let (ctx, env) = editor();
            sh(&env, &ctx);
            enter(
                r#"printf 'Password: '; read -r pw; echo "${#pw} characters""#,
                &env,
                &ctx,
            );
            tick_until(r#"(member "*Minibuffer*" (all-buffer-names))"#, &env, &ctx);
            assert_eq!(ctx.get_current_buffer_name(), "*Minibuffer*");
            for ch in "s3cr3t".chars() {
                ctx.handle_key_event(KeyEvent::new(KeyCode::Char(ch)), &env);
            }
            assert_eq!(contents(&ctx, "*Minibuffer*"), "******");
            ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
            assert_eq!(ctx.get_current_buffer_name(), "*eshell*");
            prompts(2, &env, &ctx);
            let text = contents(&ctx, "*eshell*");
            // A line started after the question, as a terminal would.
            assert!(text.contains("Password: \n6 characters\n$ "), "{text:?}");
            assert!(!text.contains("s3cr3t"), "{text:?}");
            let history = run("(eshell-history)", &env, &ctx);
            assert!(!format!("{history:?}").contains("s3cr3t"), "{history:?}");
            stop(&env, &ctx);
        }

        #[test]
        fn eshell_send_invisible_sends_a_line_unseen() {
            use crate::input::{KeyCode, KeyEvent};
            let (ctx, env) = editor();
            run("(setq eshell-watch-for-password-prompt nil)", &env, &ctx);
            sh(&env, &ctx);
            // Sent once the program reads: a line sent before, in the pipe
            // with the command, a shell may take as more commands.
            enter(
                r#"printf 'Word? '; read -r word; echo "got ${#word}""#,
                &env,
                &ctx,
            );
            tick_until(r#"(string-suffix-p "Word? " (buffer-string))"#, &env, &ctx);
            run("(eshell-send-invisible)", &env, &ctx);
            assert_eq!(ctx.get_current_buffer_name(), "*Minibuffer*");
            // The question is the line the output ends with.
            let question = ctx.windows(|windows| {
                windows.root().window_ids().into_iter().find_map(|id| {
                    windows
                        .root()
                        .window(id)?
                        .prompt
                        .as_ref()
                        .map(|p| p.text.clone())
                })
            });
            assert_eq!(question.as_deref(), Some("Word? "));
            for ch in "abcd".chars() {
                ctx.handle_key_event(KeyEvent::new(KeyCode::Char(ch)), &env);
            }
            ctx.handle_key_event(KeyEvent::new(KeyCode::Enter), &env);
            prompts(2, &env, &ctx);
            let text = contents(&ctx, "*eshell*");
            assert!(text.contains("got 4\n$ "), "{text:?}");
            assert!(!text.contains("abcd"), "{text:?}");
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
