//! Where a buffer's role comes from: the keyword forms of the functions that
//! make buffers, beside their positional ones, checked whole before anything
//! is made; an existing buffer asked to be of another role; the role a
//! prompt's reader is given, and refused, by `minibuffer-read`; and the role
//! every buffer the editor ships makes ends up with, reached the way a person
//! reaches it -- keys, commands -- rather than through the constructor.
//!
//! Design: `doc/architettura-editor.typ, <ruoli>`.
#[cfg(test)]
mod tests {
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const PROMPT: &str = "*Minibuffer*";

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// Runs SRC, which must fail, and returns what the error says.
    fn refused(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        match eval_str(src, env, ctx) {
            Ok(value) => panic!("{src} should have failed, and gave {value:?}"),
            Err(why) => format!("{why:?}"),
        }
    }

    fn text(exp: &LispExp<Ctx>) -> String {
        match exp {
            LispExp::String(text) => text.to_string(),
            LispExp::Symbol(name) => name.to_string(),
            other => format!("{other:?}"),
        }
    }

    /// The role of the buffer NAME, as Lisp has it, or "nil".
    fn role(name: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> String {
        text(&run(&format!("(buffer-role {name:?})"), env, ctx))
    }

    fn mode(name: &str, ctx: &Ctx) -> String {
        ctx.with_buffer(name, |buf| buf.current_mode.clone())
            .unwrap_or_else(|| panic!("no buffer {name}"))
    }

    /// The editor with commands.lisp, debug.lisp and minibuffer.lisp, and a
    /// major mode, `prose-mode`, of no consequence.
    fn small() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for source in [
            include_str!("../../lisp/base/commands.lisp"),
            include_str!("../../lisp/base/debug.lisp"),
            include_str!("../../lisp/base/minibuffer.lisp"),
        ] {
            eval_str(source, &env, &ctx).expect("loading the shipped lisp");
        }
        run("(make-mode 'prose-mode)", &env, &ctx);
        (ctx, env)
    }

    /// The editor with every module the default init.lisp loads.
    fn booted() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        crate::tests::shipped::load_default_modules(&env, &ctx);
        (ctx, env)
    }

    fn key(code: KeyCode, ctrl: bool, alt: bool) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers {
                ctrl,
                alt,
                ..Default::default()
            },
        }
    }

    fn press(ctx: &Ctx, env: &Arc<Env<Ctx>>, code: KeyCode) {
        ctx.handle_key_event(key(code, false, false), env);
    }

    fn type_text(ctx: &Ctx, env: &Arc<Env<Ctx>>, text: &str) {
        for c in text.chars() {
            press(ctx, env, KeyCode::Char(c));
        }
    }

    /// Types COMMAND at M-x, by keys, and runs it.
    fn m_x(ctx: &Ctx, env: &Arc<Env<Ctx>>, command: &str) {
        ctx.handle_key_event(key(KeyCode::Char('x'), false, true), env);
        assert_eq!(role(PROMPT, env, ctx), "prompt", "M-x opens a prompt");
        type_text(ctx, env, command);
        press(ctx, env, KeyCode::Enter);
    }

    // ----------------------------------------------------------------
    // buffer-create
    // ----------------------------------------------------------------

    #[test]
    fn buffer_create_called_as_it_always_was_makes_an_editing_buffer() {
        let (ctx, env) = small();
        assert_eq!(
            text(&run(r#"(buffer-create "plain")"#, &env, &ctx)),
            "plain"
        );
        run(r#"(buffer-create "prose" 'prose-mode)"#, &env, &ctx);
        assert_eq!(role("plain", &env, &ctx), "editing");
        assert_eq!(mode("plain", &ctx), "fundamental-mode");
        assert_eq!(role("prose", &env, &ctx), "editing");
        assert_eq!(mode("prose", &ctx), "prose-mode");
    }

    #[test]
    fn buffer_create_takes_mode_and_role_by_keyword_and_the_role_is_there_for_the_first_hook() {
        let (ctx, env) = small();
        run(
            r#"(setq roles-seen nil)
               (defun note-role () (setq roles-seen (cons (buffer-role) roles-seen)))
               (add-hook 'prose-mode "buffer-created-hook" 'note-role)
               (add-hook 'prose-mode "major-mode-hook" 'note-role)"#,
            &env,
            &ctx,
        );
        run(
            r#"(buffer-create "listing" :role 'view :mode 'prose-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(role("listing", &env, &ctx), "view");
        assert_eq!(mode("listing", &ctx), "prose-mode");
        assert_eq!(
            text(&run("(prin1-to-string roles-seen)", &env, &ctx)),
            "(view view)"
        );
        // The positional MODE, and a keyword after it.
        run(
            r#"(buffer-create "page" 'prose-mode :role 'document-view)"#,
            &env,
            &ctx,
        );
        assert_eq!(role("page", &env, &ctx), "document-view");
        assert_eq!(mode("page", &ctx), "prose-mode");
        // nil for either is not giving it.
        run(r#"(buffer-create "bare" :mode nil :role nil)"#, &env, &ctx);
        assert_eq!(role("bare", &env, &ctx), "editing");
        assert_eq!(mode("bare", &ctx), "fundamental-mode");
    }

    #[test]
    fn a_buffer_create_wrong_anywhere_makes_nothing() {
        let (ctx, env) = small();
        for (call, says) in [
            (
                r#"(buffer-create "x" :role 'viewer)"#,
                "no buffer role is called viewer",
            ),
            (
                r#"(buffer-create "x" :colour 'red)"#,
                "unknown keyword :colour",
            ),
            (
                r#"(buffer-create "x" :role 'view :role 'view)"#,
                "given twice",
            ),
            (r#"(buffer-create "x" :role)"#, "has no value"),
            (
                r#"(buffer-create "x" 'prose-mode :mode 'prose-mode)"#,
                "both",
            ),
            (
                r#"(buffer-create "x" :role 'view 'prose-mode)"#,
                "where a keyword",
            ),
        ] {
            let why = refused(call, &env, &ctx);
            assert!(why.contains(says), "{call}: {why}");
            assert!(!ctx.has_buffer("x"), "{call} made the buffer");
        }
        refused(r#"(buffer-create "x" 'prose-mode 'other)"#, &env, &ctx);
        assert!(!ctx.has_buffer("x"));
    }

    #[test]
    fn an_existing_buffer_asked_for_a_role_it_is_not_is_an_error_and_left_as_it_is() {
        let (ctx, env) = small();
        run(
            r#"(buffer-create "page" :role 'document-view :mode 'prose-mode)
               (buffer-create "file")"#,
            &env,
            &ctx,
        );
        // Its own role, one it derives from, or none: handed back, mode and
        // all, as it always was.
        for call in [
            r#"(buffer-create "page" :role 'document-view)"#,
            r#"(buffer-create "page" :role 'view)"#,
            r#"(buffer-create "page")"#,
            r#"(buffer-create "page" 'fundamental-mode)"#,
            r#"(buffer-create "page" :mode 'fundamental-mode :role nil)"#,
        ] {
            assert_eq!(text(&run(call, &env, &ctx)), "page", "{call}");
        }
        assert_eq!(mode("page", &ctx), "prose-mode");
        assert_eq!(role("page", &env, &ctx), "document-view");

        let why = refused(r#"(buffer-create "file" :role 'view)"#, &env, &ctx);
        assert!(why.contains("of role editing, not view"), "{why}");
        // Derived the other way is not derived.
        let why = refused(r#"(buffer-create "page" :role 'editing)"#, &env, &ctx);
        assert!(why.contains("of role document-view, not editing"), "{why}");
        run(r#"(buffer-create "list" :role 'view)"#, &env, &ctx);
        refused(r#"(buffer-create "list" :role 'document-view)"#, &env, &ctx);
        assert_eq!(role("file", &env, &ctx), "editing");
        assert_eq!(role("list", &env, &ctx), "view");
    }

    #[test]
    fn buffer_role_derived_p_follows_the_hierarchy_and_refuses_what_is_no_role() {
        let (ctx, env) = small();
        for (role, ancestor, derived) in [
            ("document-view", "view", true),
            ("document-view", "document-view", true),
            ("view", "document-view", false),
            ("prompt", "prompt", true),
            ("completion", "prompt", false),
            ("editing", "view", false),
        ] {
            assert_eq!(
                run(
                    &format!("(buffer-role-derived-p '{role} '{ancestor})"),
                    &env,
                    &ctx
                )
                .is_truthy(),
                derived,
                "{role} from {ancestor}"
            );
        }
        refused("(buffer-role-derived-p 'shelf 'view)", &env, &ctx);
        refused("(buffer-role-derived-p 'view nil)", &env, &ctx);
    }

    // ----------------------------------------------------------------
    // make-floating-window
    // ----------------------------------------------------------------

    #[test]
    fn make_floating_window_keeps_its_places_and_takes_keywords() {
        let (ctx, env) = small();
        run(
            r#"(make-floating-window "*old*" 2 1 40 6 "Old" 'prose-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(role("*old*", &env, &ctx), "editing");
        assert_eq!(mode("*old*", &ctx), "prose-mode");
        run("(close-floating-window)", &env, &ctx);

        run(
            r#"(make-floating-window "*new*" 2 1 40 6 :mode 'prose-mode :title "New" :role 'view)"#,
            &env,
            &ctx,
        );
        assert_eq!(role("*new*", &env, &ctx), "view");
        assert_eq!(mode("*new*", &ctx), "prose-mode");
        assert_eq!(ctx.get_current_buffer_name(), "*new*");
        run("(close-floating-window)", &env, &ctx);

        run(
            r#"(make-floating-window "*mixed*" 2 1 40 6 "Mixed" :role 'document-view)"#,
            &env,
            &ctx,
        );
        assert_eq!(role("*mixed*", &env, &ctx), "document-view");
    }

    #[test]
    fn a_make_floating_window_wrong_anywhere_opens_nothing() {
        let (ctx, env) = small();
        let floats = |ctx: &Ctx| ctx.windows(|windows| windows.floating().len());
        let before = ctx.get_focused_window_id();
        for call in [
            r#"(make-floating-window "*f*" 2 1 40 6 :role 'shelf)"#,
            r#"(make-floating-window "*f*" 2 1 40 6 "T" :title "T")"#,
            r#"(make-floating-window "*f*" 2 1 40 6 :mode 'prose-mode :mode 'prose-mode)"#,
            r#"(make-floating-window "*f*" 2 1 40 6 :width 3)"#,
        ] {
            refused(call, &env, &ctx);
            assert!(!ctx.has_buffer("*f*"), "{call} made the buffer");
            assert_eq!(floats(&ctx), 0, "{call} opened a window");
            assert_eq!(ctx.get_focused_window_id(), before, "{call} moved focus");
        }
    }

    // ----------------------------------------------------------------
    // minibuffer-read and its readers
    // ----------------------------------------------------------------

    fn prompt_open(env: &Arc<Env<Ctx>>, ctx: &Ctx) -> bool {
        ctx.has_buffer(PROMPT) || run("*minibuffer-on-confirm*", env, ctx).is_truthy()
    }

    #[test]
    fn both_shipped_readers_give_the_prompt_its_role_whatever_its_major_mode() {
        for floating in [false, true] {
            let (ctx, env) = small();
            if floating {
                run(
                    "(setq *minibuffer-read-function* 'floating-minibuffer-prompt)",
                    &env,
                    &ctx,
                );
            }
            // As it was always called.
            run(r#"(minibuffer-read "Q:" 'ignore nil nil)"#, &env, &ctx);
            assert_eq!(role(PROMPT, &env, &ctx), "prompt", "floating {floating}");
            assert_eq!(mode(PROMPT, &ctx), "minibuffer-mode");
            run("(minibuffer-cancel)", &env, &ctx);
            // A major mode of the caller's, and the role all the same.
            run(
                r#"(minibuffer-read "Q:" 'ignore :mode 'prose-mode :role 'prompt)"#,
                &env,
                &ctx,
            );
            assert_eq!(role(PROMPT, &env, &ctx), "prompt", "floating {floating}");
            assert_eq!(mode(PROMPT, &ctx), "prose-mode");
            // prose-mode has no `minibuffer-cleanup` to run when it closes:
            // closed, all the same.
            run("(minibuffer-cancel)", &env, &ctx);
            assert!(!ctx.has_buffer(PROMPT));
        }
    }

    #[test]
    fn a_role_that_is_not_a_prompt_opens_no_prompt_and_sets_nothing() {
        for reader in ["default-minibuffer-prompt", "floating-minibuffer-prompt"] {
            let (ctx, env) = small();
            run(
                &format!("(setq *minibuffer-read-function* '{reader})"),
                &env,
                &ctx,
            );
            for call in [
                r#"(minibuffer-read "Q:" 'ignore :role 'view)"#,
                r#"(minibuffer-read "Q:" 'ignore :role 'shelf)"#,
                r#"(minibuffer-read "Q:" 'ignore :key 'k :key 'k)"#,
                r#"(minibuffer-read "Q:" 'ignore nil :on-change 'ignore)"#,
                r#"(minibuffer-read "Q:" 'ignore :history 'k)"#,
            ] {
                refused(call, &env, &ctx);
                assert!(!prompt_open(&env, &ctx), "{reader}: {call} opened a prompt");
            }
            // The readers themselves, called directly, refuse before they
            // set anything as well.
            let why = refused(
                &format!(r#"({reader} "Q:" 'ignore nil nil nil nil 'completion)"#),
                &env,
                &ctx,
            );
            assert!(why.contains("not completion"), "{reader}: {why}");
            assert!(!prompt_open(&env, &ctx), "{reader} opened a prompt");
        }
    }

    #[test]
    fn the_keywords_reach_the_prompt_as_the_places_did() {
        let (ctx, env) = small();
        run(
            r#"(setq answer nil) (setq cancelled nil)
               (minibuffer-read "Q:" (lambda (text) (setq answer text))
                                :key 'my-history
                                :on-cancel (lambda () (setq cancelled t))
                                :on-change (lambda (text) (list "alpha" "beta")))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            text(&run("*minibuffer-history-key*", &env, &ctx)),
            "my-history"
        );
        run("(minibuffer-complete)", &env, &ctx);
        run("(minibuffer-confirm)", &env, &ctx);
        assert_eq!(text(&run("answer", &env, &ctx)), "alpha");
        assert_eq!(
            text(&run("(car (minibuffer-history 'my-history))", &env, &ctx)),
            "alpha"
        );
        run(
            r#"(minibuffer-read "Q:" 'ignore nil :on-cancel (lambda () (setq cancelled t)))"#,
            &env,
            &ctx,
        );
        run("(minibuffer-cancel)", &env, &ctx);
        assert!(run("cancelled", &env, &ctx).is_truthy());
    }

    /// A reader of the user's, written before roles, that notes what it is
    /// called with and opens the editor's prompt.
    fn with_legacy_reader() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = small();
        run(
            r#"(setq reader-got nil)
               (defun my-reader (prompt on-confirm on-change on-cancel &optional mode key)
                 (setq reader-got (list prompt on-change on-cancel mode key))
                 (default-minibuffer-prompt prompt on-confirm on-change on-cancel mode key))
               (setq *minibuffer-read-function* 'my-reader)"#,
            &env,
            &ctx,
        );
        (ctx, env)
    }

    #[test]
    fn a_reader_of_its_own_is_called_by_position_as_it_always_was() {
        let (ctx, env) = with_legacy_reader();
        run(r#"(minibuffer-read "Q:" 'ignore)"#, &env, &ctx);
        assert_eq!(
            text(&run("(prin1-to-string reader-got)", &env, &ctx)),
            r#"("Q:" nil nil nil nil)"#
        );
        assert_eq!(role(PROMPT, &env, &ctx), "prompt");
        run("(minibuffer-cancel)", &env, &ctx);
        run(
            r#"(minibuffer-read "Q:" 'ignore :key 'k :mode 'prose-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            text(&run("(prin1-to-string reader-got)", &env, &ctx)),
            r#"("Q:" nil nil prose-mode k)"#
        );
        run("(minibuffer-cancel)", &env, &ctx);
    }

    #[test]
    fn a_reader_that_does_not_take_a_role_is_refused_one_before_it_is_called() {
        let (ctx, env) = with_legacy_reader();
        let why = refused(
            r#"(minibuffer-read "Q:" 'ignore :role 'prompt)"#,
            &env,
            &ctx,
        );
        assert!(why.contains("minibuffer-reader-role"), "{why}");
        assert!(
            run("reader-got", &env, &ctx).is_nil(),
            "the reader was called"
        );
        assert!(!prompt_open(&env, &ctx));
        // A lambda has nowhere to say so.
        run(
            "(setq *minibuffer-read-function* (lambda (&rest all) (setq reader-got all)))",
            &env,
            &ctx,
        );
        refused(
            r#"(minibuffer-read "Q:" 'ignore :role 'prompt)"#,
            &env,
            &ctx,
        );
        assert!(run("reader-got", &env, &ctx).is_nil());
    }

    #[test]
    fn a_reader_that_says_it_takes_a_role_gets_it_seventh() {
        let (ctx, env) = small();
        run(
            r#"(setq reader-got nil)
               (defun my-reader (prompt on-confirm on-change on-cancel &optional mode key role)
                 (setq reader-got (list mode key role))
                 (default-minibuffer-prompt prompt on-confirm on-change on-cancel mode key role))
               (put 'my-reader 'minibuffer-reader-role t)
               (setq *minibuffer-read-function* 'my-reader)"#,
            &env,
            &ctx,
        );
        run(
            r#"(minibuffer-read "Q:" 'ignore :role 'prompt)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            text(&run("(prin1-to-string reader-got)", &env, &ctx)),
            "(nil nil prompt)"
        );
        assert_eq!(role(PROMPT, &env, &ctx), "prompt");
        run("(minibuffer-cancel)", &env, &ctx);
        // Without :role, called as any reader is.
        run(r#"(minibuffer-read "Q:" 'ignore)"#, &env, &ctx);
        assert_eq!(
            text(&run("(prin1-to-string reader-got)", &env, &ctx)),
            "(nil nil nil)"
        );
        run("(minibuffer-cancel)", &env, &ctx);
    }

    // ----------------------------------------------------------------
    // What the shipped modules make, reached as a person reaches it
    // ----------------------------------------------------------------

    #[test]
    fn m_x_opens_a_prompt_split_or_floating_and_its_completions_a_completion_list() {
        for floating in [false, true] {
            let (ctx, env) = booted();
            if floating {
                run(
                    "(setq *minibuffer-read-function* 'floating-minibuffer-prompt)",
                    &env,
                    &ctx,
                );
            }
            ctx.handle_key_event(key(KeyCode::Char('x'), false, true), &env);
            assert_eq!(role(PROMPT, &env, &ctx), "prompt", "floating {floating}");
            type_text(&ctx, &env, "buffer-li");
            press(&ctx, &env, KeyCode::Tab);
            assert_eq!(
                role("*Completions*", &env, &ctx),
                "completion",
                "floating {floating}"
            );
        }
    }

    #[test]
    fn an_incremental_search_reads_in_a_prompt_of_its_own_mode() {
        let (ctx, env) = booted();
        ctx.handle_key_event(key(KeyCode::Char('s'), true, false), &env);
        assert_eq!(role(PROMPT, &env, &ctx), "prompt");
        assert_eq!(mode(PROMPT, &ctx), "isearch-mode");
    }

    #[test]
    fn completion_at_point_lists_its_candidates_in_a_completion_buffer() {
        let (ctx, env) = booted();
        run(
            r#"(insert "ab")
               (defun my-source () (list 0 2 (list "abc" "abd")))
               (set-completion-functions nil '(my-source))
               (completion-at-point)"#,
            &env,
            &ctx,
        );
        assert_eq!(role("*Completions*", &env, &ctx), "completion");
        assert_eq!(role("*scratch*", &env, &ctx), "editing");
    }

    #[test]
    fn help_and_manual_pages_are_document_views() {
        let (ctx, env) = booted();
        m_x(&ctx, &env, "describe-function");
        type_text(&ctx, &env, "buffer-create");
        press(&ctx, &env, KeyCode::Enter);
        let help = text(&run("help-buffer-name", &env, &ctx));
        assert_eq!(mode(&help, &ctx), "help-mode");
        assert_eq!(role(&help, &env, &ctx), "document-view");

        run(r#"(manpage--show "demo" "NAME\n    demo\n")"#, &env, &ctx);
        let page = text(&run("manpage-buffer-name", &env, &ctx));
        assert_eq!(mode(&page, &ctx), "manpage-mode");
        assert_eq!(role(&page, &env, &ctx), "document-view");
    }

    #[test]
    fn the_listings_and_logs_are_views() {
        let (ctx, env) = booted();
        let directory = std::env::temp_dir();
        for (command, buffer) in [
            ("buffer-list", "buffer-list-buffer-name"),
            ("theme-list", "theme-list-buffer-name"),
        ] {
            m_x(&ctx, &env, command);
            let name = text(&run(buffer, &env, &ctx));
            assert_eq!(role(&name, &env, &ctx), "view", "{command}");
        }
        // The list of a directory's findings, since *scratch* has no file.
        run(
            &format!("(findings-list-under {:?})", directory.to_string_lossy()),
            &env,
            &ctx,
        );
        let findings = text(&run("findings-list-buffer-name", &env, &ctx));
        assert_eq!(role(&findings, &env, &ctx), "view");
        run(
            &format!("(dired {:?})", directory.to_string_lossy()),
            &env,
            &ctx,
        );
        let dired = ctx.get_current_buffer_name();
        assert_eq!(mode(&dired, &ctx), "dired-mode");
        assert_eq!(role(&dired, &env, &ctx), "view");

        m_x(&ctx, &env, "switch-to-messages");
        assert_eq!(role("*Messages*", &env, &ctx), "view");

        run(r#"(backtrace-show "it failed" nil)"#, &env, &ctx);
        assert_eq!(role("*Backtrace*", &env, &ctx), "view");
        run("(backtrace-dismiss)", &env, &ctx);

        run(r#"(lsp--log "srv" "started")"#, &env, &ctx);
        assert_eq!(role("*srv*", &env, &ctx), "view");
    }

    #[test]
    fn occur_results_and_shell_output_are_views() {
        let (ctx, env) = booted();
        run(r#"(insert "one TODO\ntwo\nthree TODO\n")"#, &env, &ctx);
        let before = ctx.buffer_names();
        run(r#"(occur "TODO")"#, &env, &ctx);
        let listing = ctx
            .buffer_names()
            .into_iter()
            .find(|name| !before.contains(name))
            .expect("occur made a listing");
        assert_eq!(role(&listing, &env, &ctx), "view");

        for call in [
            r#"(shell-command-start "true")"#,
            r#"(shell-command-start "true" 'compilation-mode 'compile)"#,
        ] {
            let output = text(&run(call, &env, &ctx));
            assert_eq!(role(&output, &env, &ctx), "view", "{call}");
        }
    }

    #[test]
    fn files_scratch_and_the_repl_are_for_editing() {
        let (ctx, env) = booted();
        assert_eq!(role("*scratch*", &env, &ctx), "editing");
        m_x(&ctx, &env, "risp-repl");
        let repl = text(&run("risp-repl-buffer", &env, &ctx));
        assert_eq!(role(&repl, &env, &ctx), "editing");
        assert_eq!(mode(&repl, &ctx), "risp-mode");

        let dir = std::env::temp_dir().join(format!("rsedit-roles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "hello\n").unwrap();
        run(
            &format!("(find-file {:?})", file.to_string_lossy()),
            &env,
            &ctx,
        );
        assert_eq!(role(&ctx.get_current_buffer_name(), &env, &ctx), "editing");
        run(
            &format!("(find-file {:?})", dir.join("new.txt").to_string_lossy()),
            &env,
            &ctx,
        );
        assert_eq!(role(&ctx.get_current_buffer_name(), &env, &ctx), "editing");
        std::fs::remove_dir_all(&dir).ok();
    }
}
