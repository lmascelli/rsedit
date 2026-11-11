//! `insert`, and the read-only buffer the two doors refuse to change: a minor
//! mode of any buffer, `read-only-mode`, tested without opening a view such as
//! dired.
//!
//! Design: `doc/architettura-editor.typ, <due-porte>`.
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
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    fn setup() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        (ctx, env)
    }

    fn text(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match run("(buffer-string)", env, ctx) {
            LispExp::String(s) => (*s).clone(),
            other => panic!("expected a string, got {other:?}"),
        }
    }

    // ---------------- insert ----------------

    #[test]
    fn insert_puts_a_whole_string_in_at_point() {
        let (ctx, env) = setup();
        run(r#"(insert "hello")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "hello");
    }

    #[test]
    fn insert_takes_several_arguments_in_order() {
        let (ctx, env) = setup();
        run(r#"(insert "a" "b" "c")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "abc");
    }

    /// A number is formatted rather than refused, so a listing can put a count
    /// in a line without converting it first -- and it is formatted the way
    /// `format`'s `%s` would, because two conversions would let the echo area
    /// and a buffer disagree about the same number.
    #[test]
    fn insert_formats_what_is_not_a_string_the_way_format_does() {
        let (ctx, env) = setup();
        run(r#"(insert "n=" 42 " " nil)"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "n=42 nil");
    }

    #[test]
    fn insert_can_put_in_a_newline() {
        let (ctx, env) = setup();
        run(r#"(insert "a\nb")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "a\nb");
    }

    /// The reason `insert` exists rather than a loop over `self-insert`: one
    /// call is one edit, so it is one undo step however long the text is.
    #[test]
    fn a_whole_insert_undoes_in_one_step() {
        let (ctx, env) = setup();
        run(r#"(insert "a long line of text")"#, &env, &ctx);
        run("(undo)", &env, &ctx);
        assert_eq!(text(&env, &ctx), "");
    }

    #[test]
    fn several_arguments_are_still_one_undo_step() {
        let (ctx, env) = setup();
        run(r#"(insert "one" "two" "three")"#, &env, &ctx);
        run("(undo)", &env, &ctx);
        assert_eq!(text(&env, &ctx), "");
    }

    #[test]
    fn insert_at_point_goes_where_point_is_not_at_the_end() {
        let (ctx, env) = setup();
        run(r#"(insert "ac")"#, &env, &ctx);
        run("(goto-char 1)", &env, &ctx);
        run(r#"(insert "b")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "abc");
    }

    #[test]
    fn inserting_nothing_is_not_an_error() {
        let (ctx, env) = setup();
        run(r#"(insert "")"#, &env, &ctx);
        run("(insert)", &env, &ctx);
        assert_eq!(text(&env, &ctx), "");
    }

    // ---------------- read-only ----------------

    #[test]
    fn a_buffer_is_writable_until_it_is_told_otherwise() {
        let (ctx, env) = setup();
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::nil());
        run("(set-buffer-read-only t)", &env, &ctx);
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::t());
        run("(set-buffer-read-only nil)", &env, &ctx);
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::nil());
    }

    #[test]
    fn a_read_only_buffer_refuses_an_insert_and_says_so() {
        let (ctx, env) = setup();
        run(r#"(insert "kept")"#, &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);

        run(r#"(insert "added")"#, &env, &ctx);

        assert_eq!(text(&env, &ctx), "kept");
        assert_eq!(ctx.get_echo_message(), "Buffer is read-only");
    }

    /// Every door, not just the one a test happened to pick: the flag is
    /// checked in `insert_text` and `delete_range`, and each of these commands
    /// goes through one of them.
    #[test]
    fn a_read_only_buffer_refuses_every_editing_command() {
        let (ctx, env) = setup();
        run(r#"(insert "first line\nsecond line")"#, &env, &ctx);
        run("(beginning-of-buffer)", &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);
        let before = text(&env, &ctx);

        for command in [
            r#"(self-insert "x")"#,
            "(insert-newline)",
            "(delete-char)",
            "(kill-line)",
            "(kill-whole-line)",
            "(kill-word)",
            "(kill-paragraph)",
            "(clear-buffer)",
            "(end-of-buffer)(delete-backward-char)",
        ] {
            run(command, &env, &ctx);
            assert_eq!(
                text(&env, &ctx),
                before,
                "{command} should have changed nothing"
            );
        }
    }

    /// Reading is the point of a read-only buffer, so moving around one has to
    /// work -- the flag is on the text, not on the cursor.
    #[test]
    fn point_moves_freely_in_a_read_only_buffer() {
        let (ctx, env) = setup();
        run(r#"(insert "first\nsecond\nthird")"#, &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);

        run("(beginning-of-buffer)", &env, &ctx);
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(1.0)
        );
        run("(next-line)(next-line)", &env, &ctx);
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(3.0)
        );
        run("(end-of-line)", &env, &ctx);
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("third".into())
        );
    }

    /// A kill that reported the text but left it in the buffer would put a
    /// copy on the ring, and the next `yank` into a writable buffer would
    /// duplicate text that was never cut.
    #[test]
    fn a_refused_kill_puts_nothing_on_the_kill_ring() {
        let (ctx, env) = setup();
        run(r#"(insert "protected")"#, &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);
        let before = run("(kill-ring-length)", &env, &ctx);

        run("(beginning-of-buffer)(kill-line)", &env, &ctx);

        assert_eq!(run("(kill-ring-length)", &env, &ctx), before);
        assert_eq!(text(&env, &ctx), "protected");
    }

    #[test]
    fn a_refused_region_kill_puts_nothing_on_the_kill_ring() {
        let (ctx, env) = setup();
        run(r#"(insert "protected")"#, &env, &ctx);
        run("(beginning-of-buffer)(set-mark)(end-of-buffer)", &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);
        let before = run("(kill-ring-length)", &env, &ctx);

        run("(kill-region)", &env, &ctx);

        assert_eq!(run("(kill-ring-length)", &env, &ctx), before);
        assert_eq!(text(&env, &ctx), "protected");
        assert_eq!(ctx.get_echo_message(), "Buffer is read-only");
    }

    /// A yank into a read-only buffer must not claim to have happened: the
    /// yank position it would record is where the *next* `yank-pop` replaces
    /// text, and recording one for text that was never inserted would have
    /// `M-y` delete something else.
    #[test]
    fn a_refused_yank_reports_and_records_nothing() {
        let (ctx, env) = setup();
        run(r#"(kill-new "from the ring")"#, &env, &ctx);
        run(r#"(insert "protected")"#, &env, &ctx);
        run("(set-buffer-read-only t)", &env, &ctx);

        run("(yank)", &env, &ctx);

        assert_eq!(text(&env, &ctx), "protected");
        assert_eq!(ctx.get_echo_message(), "Buffer is read-only");
        // `yank-pop` has nothing to replace, so it refuses rather than
        // deleting whatever happens to be at the recorded position.
        assert!(eval_str("(yank-pop)", &env, &ctx).is_err());
    }

    /// The flag is per buffer, not per editor: a protected view and an
    /// ordinary file are open at the same time, and typing has to work in one
    /// of them.
    #[test]
    fn read_only_is_a_property_of_one_buffer() {
        let (ctx, env) = setup();
        run("(set-buffer-read-only t)", &env, &ctx);
        run(
            r#"(buffer-create "writable")(switch-to-buffer "writable")"#,
            &env,
            &ctx,
        );

        run(r#"(insert "typed")"#, &env, &ctx);

        assert_eq!(text(&env, &ctx), "typed");
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::nil());
        run(r#"(switch-to-buffer "*scratch*")"#, &env, &ctx);
        assert_eq!(run("(buffer-read-only-p)", &env, &ctx), LispExp::t());
    }

    // ---------------- read-only-mode ----------------

    #[test]
    fn a_read_only_buffer_has_read_only_mode_on() {
        let (ctx, env) = setup();
        assert!(run("(minor-mode-p 'read-only-mode)", &env, &ctx).is_nil());
        run("(set-buffer-read-only t)", &env, &ctx);
        assert!(run("(minor-mode-p 'read-only-mode)", &env, &ctx).is_truthy());
        run("(set-buffer-read-only nil)", &env, &ctx);
        assert!(run("(minor-modes)", &env, &ctx).is_nil());
    }

    #[test]
    fn read_only_mode_turned_on_as_a_minor_mode_refuses_an_insert() {
        let (ctx, env) = setup();
        run("(minor-mode-on 'read-only-mode)", &env, &ctx);
        assert!(run("(buffer-read-only-p)", &env, &ctx).is_truthy());
        run(r#"(insert "x")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "");
        run("(minor-mode-off 'read-only-mode)", &env, &ctx);
        run(r#"(insert "x")"#, &env, &ctx);
        assert_eq!(text(&env, &ctx), "x");
    }

    #[test]
    fn the_command_goes_the_other_way_and_reads_its_argument() {
        let (ctx, env) = setup();
        assert!(run("(read-only-mode)", &env, &ctx).is_truthy());
        assert!(run("(buffer-read-only-p)", &env, &ctx).is_truthy());
        assert_eq!(ctx.get_echo_message(), "read-only-mode on");
        assert!(run("(read-only-mode)", &env, &ctx).is_nil());
        assert!(run("(buffer-read-only-p)", &env, &ctx).is_nil());
        assert_eq!(ctx.get_echo_message(), "read-only-mode off");
        for (arg, on) in [
            ("1", true),
            ("1", true),
            ("0", false),
            ("-1", false),
            ("'-", false),
        ] {
            run(&format!("(read-only-mode {arg})"), &env, &ctx);
            assert_eq!(
                run("(buffer-read-only-p)", &env, &ctx).is_truthy(),
                on,
                "after {arg}"
            );
        }
        assert!(run("(commandp 'read-only-mode)", &env, &ctx).is_truthy());
    }

    #[test]
    fn a_read_only_buffer_stays_so_when_its_major_mode_changes() {
        let (ctx, env) = setup();
        run(
            r#"(make-mode 'prose-mode)
               (make-minor-mode 'one-mode "One")
               (minor-mode-on 'one-mode)
               (set-buffer-read-only t)
               (set-major-mode 'prose-mode)"#,
            &env,
            &ctx,
        );
        assert!(run("(buffer-read-only-p)", &env, &ctx).is_truthy());
        assert!(run("(minor-mode-p 'one-mode)", &env, &ctx).is_nil());
    }

    #[test]
    fn the_command_runs_the_modes_hook_and_set_buffer_read_only_does_not() {
        let (ctx, env) = setup();
        run(
            r#"(setq seen 0)
               (defun count-it () (setq seen (+ seen 1)))
               (add-hook 'read-only-mode "minor-mode-hook" 'count-it)
               (set-buffer-read-only t)
               (set-buffer-read-only nil)"#,
            &env,
            &ctx,
        );
        assert_eq!(run("seen", &env, &ctx), LispExp::number(0.0));
        run("(read-only-mode) (read-only-mode)", &env, &ctx);
        assert_eq!(run("seen", &env, &ctx), LispExp::number(2.0));
    }

    #[test]
    fn the_status_line_shows_a_read_only_buffer() {
        let (ctx, env) = setup();
        run(r#"(setq mode-line-format "%m%n")"#, &env, &ctx);
        let line = |ctx: &Ctx| {
            ctx.snapshot(&env, 80, 24).views[0]
                .mode_line
                .clone()
                .expect("a status line")
        };
        assert_eq!(line(&ctx).trim(), "fundamental-mode");
        run("(read-only-mode 1)", &env, &ctx);
        assert_eq!(line(&ctx).trim(), "fundamental-mode RO");
    }

    #[test]
    fn appending_to_a_read_only_buffer_leaves_its_minor_modes_as_they_were() {
        let (ctx, env) = setup();
        run(
            r#"(make-minor-mode 'one-mode "One")
               (set-buffer-read-only t)
               (minor-mode-on 'one-mode)"#,
            &env,
            &ctx,
        );
        assert!(ctx.append_to_buffer("*scratch*", "output"));
        assert_eq!(text(&env, &ctx), "output");
        assert_eq!(
            ctx.minor_modes_of("*scratch*"),
            vec!["read-only-mode", "one-mode"]
        );
    }

    // ---------------- buffer-create's mode ----------------

    #[test]
    fn a_buffer_can_be_created_in_a_named_mode() {
        let (ctx, env) = setup();
        run("(make-mode 'view-mode)", &env, &ctx);
        run(r#"(buffer-create "*view*" 'view-mode)"#, &env, &ctx);

        let mode = ctx
            .with_buffer("*view*", |b| b.current_mode.clone())
            .expect("the buffer");
        assert_eq!(mode, "view-mode");
    }

    #[test]
    fn a_buffer_created_without_a_mode_is_in_fundamental_mode() {
        let (ctx, env) = setup();
        run(r#"(buffer-create "*plain*")"#, &env, &ctx);

        let mode = ctx
            .with_buffer("*plain*", |b| b.current_mode.clone())
            .expect("the buffer");
        assert_eq!(mode, "fundamental-mode");
    }

    /// Re-creating a buffer is how a view is refreshed, and a refresh that
    /// reset the mode would take the keymap away from the very buffer being
    /// refreshed.
    #[test]
    fn an_existing_buffer_keeps_the_mode_it_has() {
        let (ctx, env) = setup();
        run("(make-mode 'view-mode)(make-mode 'other-mode)", &env, &ctx);
        run(r#"(buffer-create "*view*" 'view-mode)"#, &env, &ctx);

        run(r#"(buffer-create "*view*" 'other-mode)"#, &env, &ctx);

        let mode = ctx
            .with_buffer("*view*", |b| b.current_mode.clone())
            .expect("the buffer");
        assert_eq!(mode, "view-mode");
    }

    // ---------------- reading a line ----------------

    #[test]
    fn the_current_line_is_the_one_point_is_on_without_its_newline() {
        let (ctx, env) = setup();
        run(r#"(insert "first\nsecond\nthird")"#, &env, &ctx);
        run("(beginning-of-buffer)(next-line)", &env, &ctx);

        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("second".into())
        );
    }

    #[test]
    fn the_current_line_of_an_empty_buffer_is_empty() {
        let (ctx, env) = setup();
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("".into())
        );
    }

    /// The numbering matches `goto-line`'s, which is what makes "remember
    /// where I was, rebuild, go back" a two-line idiom rather than an
    /// off-by-one hunt.
    #[test]
    fn the_line_number_counts_from_one_and_round_trips_through_goto_line() {
        let (ctx, env) = setup();
        run(r#"(insert "a\nb\nc\nd")"#, &env, &ctx);
        run("(beginning-of-buffer)", &env, &ctx);
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(1.0)
        );

        run("(goto-line 3)", &env, &ctx);
        assert_eq!(
            run("(line-number-at-point)", &env, &ctx),
            LispExp::number(3.0)
        );
        assert_eq!(
            run("(current-line)", &env, &ctx),
            LispExp::string("c".into())
        );
    }

    // ---------------- a refusal is said ----------------

    /// Presses C-c and then KEY: what a binding of the test's own runs as a
    /// command, the way a key would.
    fn press_c_c(ctx: &Ctx, env: &Arc<Env<Ctx>>, key: char) {
        use crate::input::{KeyCode, KeyEvent, KeyModifiers};
        let ctrl = KeyModifiers {
            ctrl: true,
            ..Default::default()
        };
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: ctrl,
            },
            env,
        );
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(key),
                modifiers: KeyModifiers::default(),
            },
            env,
        );
    }

    /// A kill, a verb, a join in a read-only buffer changed nothing and said
    /// nothing: a key that seemed broken, not a buffer that was protected.
    #[test]
    fn a_command_refused_by_a_read_only_buffer_says_so() {
        let (ctx, env) = setup();
        run(
            r#"(insert "one two\nthree") (goto-char 1)
               (define-key nil "C-c k" 'kill-line)
               (define-key nil "C-c w" 'kill-word)
               (define-key nil "C-c u" 'upcase-word)
               (define-key nil "C-c t" 'transpose-chars)
               (define-key nil "C-c j" 'join-line)
               (set-buffer-read-only t)"#,
            &env,
            &ctx,
        );
        // Each where it has something to change: `join-line` joins the line
        // point is on to the one above it.
        for (key, at) in [('k', 1), ('w', 1), ('u', 1), ('t', 1), ('j', 10)] {
            run(&format!("(goto-char {at})"), &env, &ctx);
            ctx.set_echo_message("");
            press_c_c(&ctx, &env, key);
            assert_eq!(ctx.get_echo_message(), "Buffer is read-only", "C-c {key}");
            assert_eq!(text(&env, &ctx), "one two\nthree", "C-c {key}");
        }
    }

    /// A command in a buffer that takes its edits says nothing of a refusal
    /// there was not: the flag a refusal raises does not outlive its command.
    #[test]
    fn a_refusal_is_said_after_its_own_command_only() {
        let (ctx, env) = setup();
        run(
            r#"(insert "one two") (goto-char 0)
               (define-key nil "C-c k" 'kill-line)
               (define-key nil "C-c f" 'forward-char)
               (set-buffer-read-only t)"#,
            &env,
            &ctx,
        );
        press_c_c(&ctx, &env, 'k');
        ctx.set_echo_message("");
        press_c_c(&ctx, &env, 'f');
        assert_eq!(ctx.get_echo_message(), "");
    }

    /// The block was read before the deletion that was refused, and kept: a
    /// yank elsewhere then put in a block that had never been taken out.
    #[test]
    fn a_rectangle_in_a_read_only_buffer_is_neither_cut_nor_kept() {
        let (ctx, env) = setup();
        run(
            r#"(insert "abcd\nefgh\nijkl")
               (goto-char 1) (set-mark) (goto-char 13)
               (rectangle-mark-mode)
               (set-buffer-read-only t)"#,
            &env,
            &ctx,
        );
        let point = run("(point)", &env, &ctx);
        assert_eq!(run("(kill-rectangle)", &env, &ctx), LispExp::nil());
        assert_eq!(
            run("(killed-rectangle)", &env, &ctx),
            LispExp::nil(),
            "nothing kept"
        );
        assert_eq!(run("(point)", &env, &ctx), point, "point where it was");
        assert_eq!(ctx.get_echo_message(), "Buffer is read-only");
        for command in [
            "(delete-rectangle)",
            "(open-rectangle)",
            "(clear-rectangle)",
            r#"(string-rectangle "x")"#,
        ] {
            assert_eq!(run(command, &env, &ctx), LispExp::nil(), "{command}");
        }
        assert_eq!(text(&env, &ctx), "abcd\nefgh\nijkl");
    }

    /// The counter moved on before the insertion was refused, and the refusal
    /// was an error worded unlike every other.
    #[test]
    fn the_macro_counter_does_not_move_on_for_a_number_it_could_not_insert() {
        let (ctx, env) = setup();
        run(
            "(kmacro-set-counter 5) (set-buffer-read-only t)",
            &env,
            &ctx,
        );
        assert_eq!(run("(kmacro-insert-counter)", &env, &ctx), LispExp::nil());
        assert_eq!(ctx.get_echo_message(), "Buffer is read-only");
        run("(set-buffer-read-only nil)", &env, &ctx);
        assert_eq!(
            run("(kmacro-insert-counter)", &env, &ctx),
            LispExp::string("5".to_string())
        );
    }

    // ---------------- why work did not start ----------------

    /// A command that could not get its thread said the editor's work had
    /// shut down, also when it had not.
    #[test]
    fn a_thread_refused_says_why() {
        let (ctx, _env) = setup();
        assert_eq!(
            crate::primitives::shell::cannot_start_thread(&ctx, "the command"),
            "Cannot start the command: the system gave no thread for it"
        );
        ctx.shutdown(std::time::Duration::from_millis(100));
        assert_eq!(
            crate::primitives::shell::cannot_start_thread(&ctx, "the command"),
            "The editor's background work has shut down"
        );
    }
}
