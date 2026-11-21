//! The password prompt `read-passwd` opens: what is typed goes into the
//! password and never into the buffer, which shows a `read-hide-char' for
//! each character; the keys that edit it; Tab, which shows it; CONFIRM and
//! DEFAULT; and that nothing typed there reaches the kill ring, the history,
//! a keyboard macro or the buffer's undo. Then the cache, and
//! `process-send-password`.
//!
//! Design: `doc/architettura-editor.typ, <password>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::feature::minibuffer::PROMPT_BUFFER;
    use crate::feature::password::{NOT_REPEATED, PasswordCache, Secret};
    use crate::input::{KeyCode, KeyEvent, parse_key_sequence};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    type Ctx = EditorState<GapBuffer>;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// An editor with the global keys, the floating prompt and `got`, which
    /// the callbacks below set.
    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/base/common-keymaps.lisp"),
            include_str!("../../lisp/base/minibuffer.lisp"),
        ] {
            run(source, &env, &ctx);
        }
        run(
            "(defvar got nil) (defvar cancelled nil)
             (defun keep (password) (setq got password))
             (defun gave-up () (setq cancelled t))",
            &env,
            &ctx,
        );
        (ctx, env)
    }

    /// Presses KEYS, written as `define-key` writes them, through the key
    /// handler, as a person would.
    fn press(keys: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for key in parse_key_sequence(keys).unwrap_or_else(|| panic!("{keys} does not parse")) {
            ctx.handle_key_event(key, env);
        }
    }

    /// Types TEXT a character at a time, as the keyboard would.
    fn type_in(text: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
        for ch in text.chars() {
            ctx.handle_key_event(KeyEvent::new(KeyCode::Char(ch)), env);
        }
    }

    /// What the prompt's buffer holds, or `None` when there is no prompt.
    fn shown(ctx: &Ctx) -> Option<String> {
        ctx.with_buffer(PROMPT_BUFFER, |buf| buf.text.to_string())
    }

    /// The question of the prompt that is open, as its window draws it.
    fn question(ctx: &Ctx) -> Option<String> {
        ctx.windows(|windows| {
            windows.root().window_ids().into_iter().find_map(|id| {
                windows
                    .root()
                    .window(id)?
                    .prompt
                    .as_ref()
                    .map(|p| p.text.clone())
            })
        })
    }

    fn got(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        run("got", env, ctx)
    }

    fn string(text: &str) -> LispExp<Ctx> {
        LispExp::string(text.to_string())
    }

    // ----------------------------------------------------------------
    // The secret
    // ----------------------------------------------------------------

    #[test]
    fn a_secret_is_edited_as_text_and_never_printed() {
        let mut secret = Secret::from_text("pasword");
        assert_eq!(secret.insert(3, "s"), 4);
        assert_eq!(secret.reveal(), "password");
        secret.delete(4, 0);
        assert_eq!(secret.reveal(), "word");
        assert_eq!(secret.bytes_with("\n"), b"word\n");
        assert_eq!(format!("{secret:?}"), "Secret(4 characters)");
        // Growing past the room it started with keeps the text whole.
        let long: String = std::iter::repeat_n('x', 1000).collect();
        secret.insert(4, &long);
        assert_eq!(secret.len(), 1004);
        assert!(secret.duplicate() == secret);
        secret.clear();
        assert!(secret.is_empty());
    }

    // ----------------------------------------------------------------
    // Typing at the prompt
    // ----------------------------------------------------------------

    #[test]
    fn what_is_typed_is_hidden_in_the_buffer_and_given_to_the_callback() {
        let (ctx, env) = editor();
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        assert_eq!(ctx.get_current_buffer_name(), PROMPT_BUFFER);
        assert_eq!(question(&ctx).as_deref(), Some("Password: "));
        type_in("s3cret", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("******"));
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("s3cret"));
        assert_eq!(shown(&ctx), None, "the prompt is closed");
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
    }

    #[test]
    fn the_keys_of_the_prompt_edit_the_password_where_the_cursor_is() {
        let (ctx, env) = editor();
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        type_in("acd", &env, &ctx);
        // The cursor moves over the stars with the ordinary keys.
        press("C-b C-b", &env, &ctx);
        type_in("b", &env, &ctx);
        press("C-e <backspace>", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("***"));
        press("C-a C-d", &env, &ctx);
        type_in("x", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("xbc"));

        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        type_in("abcdef", &env, &ctx);
        press("C-b C-b C-k", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("****"));
        // C-u is the prompt's own, not the prefix argument's.
        press("C-u", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some(""));
        type_in("z", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("z"));
    }

    #[test]
    fn tab_shows_the_password_and_hides_it_again() {
        let (ctx, env) = editor();
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        type_in("pw", &env, &ctx);
        press("<tab>", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("pw"));
        // Typed where the cursor is while it shows, too.
        press("C-a", &env, &ctx);
        type_in("x", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("xpw"));
        press("<tab>", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("***"));
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("xpw"));
    }

    #[test]
    fn read_hide_char_says_what_is_drawn_and_nil_draws_nothing() {
        let (ctx, env) = editor();
        run(r#"(setq read-hide-char "•")"#, &env, &ctx);
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        type_in("ab", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("••"));
        press("<ret>", &env, &ctx);

        run("(setq read-hide-char nil)", &env, &ctx);
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        type_in("abc", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some(""));
        // With nothing drawn, the cursor is always at the end.
        press("C-a <backspace>", &env, &ctx);
        type_in("d", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("abd"));
    }

    #[test]
    fn a_paste_and_a_yank_go_into_the_password_without_line_breaks() {
        let (ctx, env) = editor();
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        ctx.handle_paste("pass\n".to_string(), &env);
        assert_eq!(shown(&ctx).as_deref(), Some("****"));
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("pass"));

        // The kill ring is the editor's own, and C-y takes from it.
        run(
            r#"(insert "from the ring") (beginning-of-line) (kill-line)"#,
            &env,
            &ctx,
        );
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        press("C-y", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("from the ring"));
    }

    // ----------------------------------------------------------------
    // Answering it
    // ----------------------------------------------------------------

    #[test]
    fn confirm_asks_twice_and_starts_over_when_the_two_differ() {
        let (ctx, env) = editor();
        run(r#"(read-passwd "New password: " 'keep t)"#, &env, &ctx);
        type_in("one", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(question(&ctx).as_deref(), Some("Confirm password: "));
        assert_eq!(shown(&ctx).as_deref(), Some(""), "the second starts empty");
        type_in("two", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(ctx.get_echo_message(), NOT_REPEATED);
        assert_eq!(question(&ctx).as_deref(), Some("New password: "));
        assert!(got(&env, &ctx).is_nil());
        type_in("one", &env, &ctx);
        press("<ret>", &env, &ctx);
        type_in("one", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("one"));
        assert_eq!(shown(&ctx), None);
    }

    #[test]
    fn an_empty_answer_is_the_default_and_the_keywords_work_as_the_places() {
        let (ctx, env) = editor();
        run(
            r#"(read-passwd "Password: " 'keep :default "fallback")"#,
            &env,
            &ctx,
        );
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("fallback"));

        run(
            r#"(read-passwd "Password: " 'keep nil "fallback")"#,
            &env,
            &ctx,
        );
        type_in("typed", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("typed"));

        assert!(
            eval_str(
                r#"(read-passwd "Password: " 'keep t :confirm t)"#,
                &env,
                &ctx
            )
            .is_err(),
            "CONFIRM in its place and by keyword"
        );
        assert_eq!(shown(&ctx), None, "a wrong call opens nothing");
    }

    #[test]
    fn escape_and_c_g_close_it_and_call_on_cancel() {
        let (ctx, env) = editor();
        for key in ["<esc>", "C-g"] {
            run("(setq cancelled nil)", &env, &ctx);
            run(
                r#"(read-passwd "Password: " 'keep :on-cancel 'gave-up)"#,
                &env,
                &ctx,
            );
            type_in("abc", &env, &ctx);
            press(key, &env, &ctx);
            assert!(run("cancelled", &env, &ctx).is_truthy(), "{key}");
            assert!(got(&env, &ctx).is_nil(), "{key}");
            assert_eq!(shown(&ctx), None, "{key}");
            assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
        }
    }

    #[test]
    fn minibuffer_confirm_in_a_password_prompt_answers_it() {
        let (ctx, env) = editor();
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        type_in("abc", &env, &ctx);
        run("(minibuffer-confirm)", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("abc"));
    }

    #[test]
    fn the_floating_prompt_reads_a_password_as_well() {
        let (ctx, env) = editor();
        run("(toggle-floating-minibuffer)", &env, &ctx);
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        assert_eq!(ctx.windows(|windows| windows.floating().len()), 1);
        type_in("abc", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("***"));
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("abc"));
    }

    #[test]
    fn a_reader_that_leaves_out_the_mode_is_refused_and_its_prompt_closed() {
        let (ctx, env) = editor();
        run(
            "(defun careless-prompt (prompt on-confirm on-change on-cancel &optional mode key)
               (default-minibuffer-prompt prompt on-confirm on-change on-cancel nil key))
             (setq *minibuffer-read-function* 'careless-prompt)",
            &env,
            &ctx,
        );
        assert!(eval_str(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx).is_err());
        assert_eq!(shown(&ctx), None);
        assert_eq!(ctx.get_current_buffer_name(), "*scratch*");
    }

    #[test]
    fn one_prompt_at_a_time() {
        let (ctx, env) = editor();
        run(r#"(minibuffer-read "Name: " nil nil nil)"#, &env, &ctx);
        assert!(eval_str(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx).is_err());
    }

    // ----------------------------------------------------------------
    // Where the password does not go
    // ----------------------------------------------------------------

    #[test]
    fn the_buffer_refuses_every_other_edit_and_has_nothing_to_undo() {
        let (ctx, env) = editor();
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        type_in("abc", &env, &ctx);
        // Neither an insertion nor an undo through the ordinary doors.
        run(r#"(insert "zzz")"#, &env, &ctx);
        let _ = eval_str("(undo)", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("***"));
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("abc"));
    }

    #[test]
    fn nothing_is_killed_or_copied_from_a_password_prompt() {
        let (ctx, env) = editor();
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        type_in("hunter2", &env, &ctx);
        press("<tab>", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some("hunter2"));
        // The global kills, on the password shown.
        run("(kill-whole-line)", &env, &ctx);
        run(
            "(beginning-of-line) (set-mark) (end-of-line) (kill-ring-save)",
            &env,
            &ctx,
        );
        assert_eq!(ctx.current_kill(), None);
        assert_eq!(shown(&ctx).as_deref(), Some("hunter2"));
        // Nor as a rectangle.
        run(
            "(beginning-of-line) (set-mark) (end-of-line) (copy-rectangle-as-kill)",
            &env,
            &ctx,
        );
        assert!(run("(killed-rectangle)", &env, &ctx).is_nil());
        press("C-a C-k", &env, &ctx);
        assert_eq!(ctx.current_kill(), None, "C-k deletes, and kills nothing");
        press("<esc>", &env, &ctx);
    }

    #[test]
    fn no_history_keeps_a_password() {
        let (ctx, env) = editor();
        run(r#"(read-passwd "Password: " 'keep)"#, &env, &ctx);
        type_in("abc", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert!(run(r#"(minibuffer-history "Password: ")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn a_macro_records_nothing_typed_at_a_password_prompt_and_stops_at_one() {
        let (ctx, env) = editor();
        run(
            r#"(defcommand ask () nil (read-passwd "Password: " 'keep))
               (define-key nil "C-c p" 'ask)"#,
            &env,
            &ctx,
        );
        press("C-x (", &env, &ctx);
        press("C-c p", &env, &ctx);
        type_in("secret", &env, &ctx);
        press("<ret> C-x )", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("secret"));
        let keys = run("(kbd-macro-keys)", &env, &ctx);
        assert_eq!(keys, string("C-c p"));

        run("(setq got nil)", &env, &ctx);
        run("(kmacro-call-macro)", &env, &ctx);
        assert_eq!(
            shown(&ctx).as_deref(),
            Some(""),
            "the replay opened the prompt"
        );
        assert_eq!(ctx.get_echo_message(), "Macro stopped at a password prompt");
        // Replaying from inside the prompt would type into the password.
        run("(kmacro-call-macro)", &env, &ctx);
        assert_eq!(shown(&ctx).as_deref(), Some(""));
        type_in("again", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("again"));
    }

    // ----------------------------------------------------------------
    // The cache
    // ----------------------------------------------------------------

    #[test]
    fn the_cache_keeps_a_password_until_it_expires() {
        let mut cache = PasswordCache::default();
        let now = Instant::now();
        cache.add(
            "k",
            Secret::from_text("pw"),
            Some(Duration::from_secs(10)),
            now,
        );
        cache.add("for ever", Secret::from_text("pw"), None, now);
        assert_eq!(cache.get("k", now).as_deref(), Some("pw"));
        let later = now + Duration::from_secs(11);
        assert!(!cache.contains("k", later));
        assert!(cache.contains("for ever", later));
        assert_eq!(cache.clear(), 1);
        assert!(cache.is_empty());
    }

    #[test]
    fn password_cache_add_read_and_remove() {
        let (ctx, env) = editor();
        assert!(run(r#"(password-cache-add "host" "pw")"#, &env, &ctx).is_truthy());
        assert_eq!(
            run(r#"(password-read-from-cache "host")"#, &env, &ctx),
            string("pw")
        );
        assert!(run(r#"(password-in-cache-p 'host)"#, &env, &ctx).is_truthy());
        assert!(run(r#"(password-cache-remove "host")"#, &env, &ctx).is_truthy());
        assert!(run(r#"(password-read-from-cache "host")"#, &env, &ctx).is_nil());

        run(
            r#"(password-cache-add "a" "1") (password-cache-add "b" "2")"#,
            &env,
            &ctx,
        );
        assert_eq!(run("(password-reset)", &env, &ctx), LispExp::number(2.0));
        assert!(run(r#"(password-in-cache-p "a")"#, &env, &ctx).is_nil());

        // Off, it keeps nothing and finds nothing.
        run(r#"(password-cache-add "kept" "pw")"#, &env, &ctx);
        run("(setq password-cache nil)", &env, &ctx);
        assert!(run(r#"(password-read-from-cache "kept")"#, &env, &ctx).is_nil());
        assert!(run(r#"(password-cache-add "other" "pw")"#, &env, &ctx).is_nil());
        run("(setq password-cache t)", &env, &ctx);
        assert!(run(r#"(password-in-cache-p "other")"#, &env, &ctx).is_nil());

        // An expiry of nothing keeps nothing for a moment.
        run("(setq password-cache-expiry 0)", &env, &ctx);
        run(r#"(password-cache-add "gone" "pw")"#, &env, &ctx);
        assert!(run(r#"(password-in-cache-p "gone")"#, &env, &ctx).is_nil());
    }

    #[test]
    fn password_read_answers_from_the_cache_and_asks_when_it_cannot() {
        let (ctx, env) = editor();
        run(r#"(password-cache-add "host" "cached")"#, &env, &ctx);
        run(r#"(password-read "Password: " 'keep "host")"#, &env, &ctx);
        assert_eq!(got(&env, &ctx), string("cached"));
        assert_eq!(shown(&ctx), None, "nothing was asked");

        run(r#"(password-read "Password: " 'keep "other")"#, &env, &ctx);
        type_in("typed", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(got(&env, &ctx), string("typed"));
        assert!(
            run(r#"(password-in-cache-p "other")"#, &env, &ctx).is_nil(),
            "password-read keeps nothing"
        );

        run(
            r#"(password-read-and-add "Password: " 'keep "other")"#,
            &env,
            &ctx,
        );
        type_in("kept", &env, &ctx);
        press("<ret>", &env, &ctx);
        assert_eq!(
            run(r#"(password-read-from-cache "other")"#, &env, &ctx),
            string("kept")
        );
    }

    // ----------------------------------------------------------------
    // To a process
    // ----------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn process_send_password_writes_what_is_typed_to_the_process() {
        let (ctx, env) = editor();
        run(
            "(defvar outputs nil)
             (defun record (name kind value)
               (if (eq kind 'output) (setq outputs (cons value outputs))))",
            &env,
            &ctx,
        );
        assert!(
            run(
                r#"(process-send-password 'nobody "Password: ")"#,
                &env,
                &ctx
            )
            .is_nil()
        );
        assert_eq!(shown(&ctx), None, "no process, no prompt");

        run(r#"(process-start 'p "cat" nil 'record)"#, &env, &ctx);
        assert!(run(r#"(process-send-password 'p "Password: ")"#, &env, &ctx).is_truthy());
        type_in("pw", &env, &ctx);
        press("<ret>", &env, &ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while run("outputs", &env, &ctx).is_nil() {
            assert!(Instant::now() < deadline, "cat did not answer");
            if !ctx.tick(&env) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        assert_eq!(run("(car outputs)", &env, &ctx), string("pw"));
        run("(process-stop 'p)", &env, &ctx);
    }
}
