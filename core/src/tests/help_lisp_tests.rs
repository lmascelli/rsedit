//! The help pages themselves: what goes on them, and getting back.
//!
//! # What is worth testing here
//!
//! A help page is believed, so the failures that matter are the quiet ones:
//! a page that says a key runs the global binding while the mode's is what
//! runs, a page for a name that does not exist that looks like a page for a
//! name that does, a back stack that walks between the last two pages for
//! ever.
//!
//! The other half is that the pages are made of the *editor's* answers rather
//! than the module's own reading of the keymaps. `describe-key` is the test of
//! that: it goes through the real key path -- keys are pressed here, not
//! simulated -- so a page reporting anything but what those keys would have
//! done is a failing test rather than a surprise months later.
#[cfg(test)]
mod tests {
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyCode, KeyEvent, KeyModifiers};
    use risp::{Env, LispExp, Parser, eval};
    use crate::{ELispExp, buffer::gap_buffer::GapBuffer};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    /// The modules a help page needs under it: `defcommand`, `message`, the
    /// keymaps it reports on, the minibuffer its prompts open, and the manual
    /// it hands `C-h m` to.
    const SHIPPED: [(&str, &str); 7] = [
        ("commands", include_str!("../../lisp/commands.lisp")),
        ("debug", include_str!("../../lisp/debug.lisp")),
        (
            "common-keymaps",
            include_str!("../../lisp/common-keymaps.lisp"),
        ),
        ("indent", include_str!("../../lisp/indent.lisp")),
        ("minibuffer", include_str!("../../lisp/minibuffer.lisp")),
        ("manpage", include_str!("../../lisp/manpage.lisp")),
        ("help", include_str!("../../lisp/help.lisp")),
    ];

    fn editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(80.0));
        env.set_variable("frame-height".into(), LispExp::number(24.0));
        for (name, source) in SHIPPED {
            let ast = Parser::new(&format!("(progn {source})"))
                .next()
                .unwrap_or_else(|why| panic!("{name}.lisp must parse: {why:?}"));
            eval(&ast, env.clone(), &ctx)
                .unwrap_or_else(|why| panic!("loading {name}.lisp: {why:?}"));
        }
        (ctx, env)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("the test must parse");
        eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// What the help buffer says.
    fn page(ctx: &Ctx) -> String {
        ctx.with_buffer("*Help*", |buf| buf.text.to_string())
            .unwrap_or_default()
    }

    fn press(ch: char, ctrl: bool, ctx: &Ctx, env: &Arc<Env<Ctx>>) {
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers {
                    ctrl,
                    ..KeyModifiers::default()
                },
            },
            env,
        );
    }

    fn assert_says(page: &str, what: &str) {
        assert!(page.contains(what), "the page should say {what:?}:\n{page}");
    }

    // ----------------------------------------------------------------
    // A function
    // ----------------------------------------------------------------

    #[test]
    fn a_function_page_carries_its_signature_its_keys_and_its_documentation() {
        let (ctx, env) = editor();
        run(r#"(help-function "find-file")"#, &env, &ctx);
        let page = page(&ctx);
        assert_says(&page, "find-file");
        // Bound in common-keymaps, and a command that asks for a file name.
        assert_says(&page, "Bound to: C-x C-f");
        assert_says(&page, "A command. It asks for:");
        assert_says(&page, "fFind file: ");
    }

    #[test]
    fn a_lisp_functions_signature_comes_from_its_parameter_list() {
        let (ctx, env) = editor();
        run(
            r#"(defun greet (who &optional loudly) "Say hello." nil)
               (help-function "greet")"#,
            &env,
            &ctx,
        );
        let page = page(&ctx);
        assert_says(&page, "(greet who &optional loudly)");
        assert_says(&page, "Say hello.");
    }

    #[test]
    fn a_primitives_signature_comes_from_the_first_line_of_its_documentation() {
        // A primitive is a Rust function with no parameter list to read, and
        // `function-arglist` says so by answering nil. The call is written in
        // the docstring for exactly this reason.
        let (ctx, env) = editor();
        run(r#"(help-function "goto-char")"#, &env, &ctx);
        assert_says(&page(&ctx), "(goto-char POSITION)");
    }

    #[test]
    fn a_name_that_is_no_function_gets_a_page_saying_so() {
        // Not an empty page: a page with nothing on it reads as "this exists
        // and is undocumented", which is a different and much more confusing
        // answer than "there is no such thing".
        let (ctx, env) = editor();
        run(r#"(help-function "no-such-function")"#, &env, &ctx);
        assert_says(&page(&ctx), "no-such-function is not a function.");
    }

    // ----------------------------------------------------------------
    // A variable
    // ----------------------------------------------------------------

    #[test]
    fn a_variable_page_carries_its_value_and_its_documentation() {
        let (ctx, env) = editor();
        run(
            r#"(defvar fill-column 70 "Where lines are wrapped.")
               (help-variable "fill-column")"#,
            &env,
            &ctx,
        );
        let page = page(&ctx);
        assert_says(&page, "Value: 70");
        assert_says(&page, "Where lines are wrapped.");
    }

    #[test]
    fn a_documented_variable_with_no_value_says_which_of_the_two_it_is() {
        // Documented and bound are separate questions -- see `variable-doc`
        // -- and printing "Value: nil" for a variable nothing has set would
        // be inventing a value it does not have.
        let (ctx, env) = editor();
        run(
            r#"(defvar never-set nil "Set by whatever runs first.")
               (help-variable "never-set")"#,
            &env,
            &ctx,
        );
        assert_says(&page(&ctx), "Value: nil");

        run(
            r#"(put 'unbound-but-known 'variable-documentation "Documented, unset.")
               (help-variable "unbound-but-known")"#,
            &env,
            &ctx,
        );
        assert_says(&page(&ctx), "Not set.");
    }

    // ----------------------------------------------------------------
    // A key
    // ----------------------------------------------------------------

    #[test]
    fn describe_key_reports_the_binding_the_keys_would_have_run() {
        let (ctx, env) = editor();
        run("(describe-key)", &env, &ctx);
        press('x', true, &ctx, &env);
        press('f', true, &ctx, &env);
        let page = page(&ctx);
        assert_says(&page, "Key: C-x C-f");
        assert_says(&page, "runs find-file");
        assert_says(&page, "global keymap");
        // And the function's own page is folded in, so looking up a key
        // answers the question the key was being looked up for.
        assert_says(&page, "A command. It asks for:");
    }

    #[test]
    fn describe_key_reports_the_mode_binding_where_a_mode_has_one() {
        // The failure the whole mechanism exists to prevent: reporting the
        // global binding while the mode's is what runs.
        let (ctx, env) = editor();
        run(
            r#"(define-key nil "C-c 8" 'global-answer)
               (make-mode 'probe-mode)
               (define-key 'probe-mode "C-c 8" 'mode-answer)
               (buffer-create "*probe*" 'probe-mode)
               (switch-to-buffer "*probe*")
               (describe-key)"#,
            &env,
            &ctx,
        );
        press('c', true, &ctx, &env);
        press('8', false, &ctx, &env);
        let page = page(&ctx);
        assert_says(&page, "runs mode-answer");
        assert_says(&page, "mode keymap");
    }

    #[test]
    fn describe_key_on_an_unbound_key_says_so_rather_than_typing_it() {
        let (ctx, env) = editor();
        run("(describe-key)", &env, &ctx);
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('y'),
                modifiers: KeyModifiers {
                    ctrl: true,
                    alt: true,
                    ..KeyModifiers::default()
                },
            },
            &env,
        );
        assert_says(&page(&ctx), "C-M-y is not bound to anything.");
    }

    #[test]
    fn describe_key_names_the_prefix_argument_for_what_it_is() {
        let (ctx, env) = editor();
        run("(describe-key)", &env, &ctx);
        press('u', true, &ctx, &env);
        assert_says(&page(&ctx), "C-u is read as a prefix argument.");
    }

    // ----------------------------------------------------------------
    // Half-way through a prefix
    // ----------------------------------------------------------------

    #[test]
    fn the_help_key_during_a_prefix_says_what_the_prefix_continues_with() {
        // End to end through the real key path: `C-x` is pending, `C-h` is
        // bound to nothing after it, and the editor turns that into a
        // question instead of "C-x C-h is undefined".
        let (ctx, env) = editor();
        press('x', true, &ctx, &env);
        press('h', true, &ctx, &env);
        let page = page(&ctx);
        assert_says(&page, "C-x continues with:");
        assert_says(&page, "C-x C-f");
    }

    #[test]
    fn the_help_key_follows_the_variable_that_names_it() {
        // Moving the prefix has to move the prefix help too, or the one
        // person who had to move it loses the feature silently.
        let (ctx, env) = editor();
        run(r#"(setq help-key "C-c j") (install-help-keys)"#, &env, &ctx);
        press('x', true, &ctx, &env);
        press('h', true, &ctx, &env);
        assert_eq!(page(&ctx), "", "C-h should no longer ask anything");

        // Its last key is what asks half-way through a sequence: there is
        // only ever one key to press there. `j' rather than `h' because
        // `C-x h' is mark-whole-buffer -- a real binding wins over the
        // question, which is the whole reason this only fires on sequences
        // that mean nothing.
        press('x', true, &ctx, &env);
        press('j', false, &ctx, &env);
        assert_says(&page(&ctx), "C-x continues with:");
    }

    #[test]
    fn a_sequence_going_nowhere_is_still_just_undefined() {
        let (ctx, env) = editor();
        ctx.handle_key_event(
            KeyEvent {
                code: KeyCode::Char('y'),
                modifiers: KeyModifiers {
                    ctrl: true,
                    alt: true,
                    ..KeyModifiers::default()
                },
            },
            &env,
        );
        press('h', true, &ctx, &env);
        assert_eq!(page(&ctx), "", "nothing should have been described");
    }

    // ----------------------------------------------------------------
    // The back stack
    // ----------------------------------------------------------------

    #[test]
    fn going_back_returns_to_the_page_before() {
        let (ctx, env) = editor();
        run(
            r#"(help-function "find-file") (help-function "goto-char")"#,
            &env,
            &ctx,
        );
        assert_says(&page(&ctx), "(goto-char POSITION)");
        run("(help-back)", &env, &ctx);
        assert_says(&page(&ctx), "Bound to: C-x C-f");
    }

    #[test]
    fn going_back_twice_reaches_the_third_page_rather_than_oscillating() {
        // The bug this is here for: a back that pushes the page it is leaving
        // walks between the last two for ever, and looks like it is working.
        let (ctx, env) = editor();
        run(
            r#"(help-function "find-file")
               (help-function "goto-char")
               (help-function "insert")"#,
            &env,
            &ctx,
        );
        run("(help-back)", &env, &ctx);
        assert_says(&page(&ctx), "(goto-char POSITION)");
        run("(help-back)", &env, &ctx);
        assert_says(&page(&ctx), "Bound to: C-x C-f");
    }

    #[test]
    fn there_is_nothing_to_go_back_to_at_the_first_page() {
        let (ctx, env) = editor();
        run(r#"(help-function "insert") (help-back)"#, &env, &ctx);
        assert_says(&page(&ctx), "insert");
    }

    #[test]
    fn a_page_revisited_is_rebuilt_rather_than_replayed() {
        // The stack holds what it takes to build a page, not the text of one,
        // so going back to a page shows what is true now.
        let (ctx, env) = editor();
        run(
            r#"(defvar watched 1 "The first description.")
               (help-variable "watched")
               (help-function "insert")
               (defvar watched 1 "The second description.")
               (help-back)"#,
            &env,
            &ctx,
        );
        assert_says(&page(&ctx), "The second description.");
    }

    // ----------------------------------------------------------------
    // The rest of the tree
    // ----------------------------------------------------------------

    #[test]
    fn the_bindings_page_groups_by_the_map_a_binding_comes_from() {
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'probe-mode)
               (define-key 'probe-mode "C-c 7" 'mode-answer)
               (buffer-create "*probe*" 'probe-mode)
               (switch-to-buffer "*probe*")
               (describe-bindings)"#,
            &env,
            &ctx,
        );
        let page = page(&ctx);
        assert_says(&page, "Mode -- probe-mode");
        assert_says(&page, "C-c 7");
        assert_says(&page, "Global -- everywhere");
    }

    #[test]
    fn a_bindings_page_revisited_still_describes_the_buffer_it_was_about() {
        // Going back happens from inside the help buffer, so a page that
        // asked "what is bound here" when it was made must not answer it
        // again from where it is being read -- that would describe the help
        // buffer, which binds `q' and `l' and nothing anybody asked about.
        let (ctx, env) = editor();
        run(
            r#"(make-mode 'probe-mode)
               (define-key 'probe-mode "C-c 7" 'mode-answer)
               (buffer-create "*probe*" 'probe-mode)
               (switch-to-buffer "*probe*")
               (describe-bindings)
               (help-function "insert")
               (help-back)"#,
            &env,
            &ctx,
        );
        let page = page(&ctx);
        assert_says(&page, "*probe*");
        assert_says(&page, "Mode -- probe-mode");
    }

    #[test]
    fn apropos_finds_functions_and_variables_by_pattern() {
        let (ctx, env) = editor();
        run(
            r#"(defvar apropos-probe-variable 1 "A variable to find.")
               (defun apropos-probe-function () "A function to find." nil)
               (help-apropos "apropos-probe")"#,
            &env,
            &ctx,
        );
        let page = page(&ctx);
        assert_says(&page, "Functions");
        assert_says(&page, "A function to find.");
        assert_says(&page, "Variables");
        assert_says(&page, "A variable to find.");
    }

    #[test]
    fn apropos_says_when_nothing_matches() {
        let (ctx, env) = editor();
        run(r#"(help-apropos "zzz-nothing-like-this")"#, &env, &ctx);
        assert_says(&page(&ctx), "Nothing matches");
    }

    #[test]
    fn where_is_says_which_keys_reach_a_command() {
        let (ctx, env) = editor();
        run(r#"(help-where-is "find-file")"#, &env, &ctx);
        assert_says(&page(&ctx), "find-file is on C-x C-f");
    }

    #[test]
    fn the_help_page_lists_the_keys_as_they_are_now() {
        let (ctx, env) = editor();
        run(r#"(setq help-key "C-c h") (help-for-help)"#, &env, &ctx);
        assert_says(&page(&ctx), "C-c h f");
        // And it says which single key to press half-way through a sequence.
        assert_says(&page(&ctx), "C-x h ");
    }

    // ----------------------------------------------------------------
    // Reading one
    // ----------------------------------------------------------------

    #[test]
    fn a_help_page_cannot_be_typed_into() {
        let (ctx, env) = editor();
        run(r#"(help-function "insert")"#, &env, &ctx);
        let before = page(&ctx);
        press('z', false, &ctx, &env);
        assert_eq!(page(&ctx), before);
    }

    #[test]
    fn a_name_on_the_page_can_be_followed() {
        let (ctx, env) = editor();
        run(r#"(help-function "find-file")"#, &env, &ctx);
        // The first line is the name itself, which is the simplest thing on
        // the page to put the cursor on.
        run("(goto-char 1)", &env, &ctx);
        press('K', false, &ctx, &env);
        assert_says(&page(&ctx), "find-file");
    }

    #[test]
    fn following_something_that_is_neither_says_so() {
        let (ctx, env) = editor();
        run(
            r#"(help-apropos "zzz-nothing-like-this") (goto-char 1)"#,
            &env,
            &ctx,
        );
        press('K', false, &ctx, &env);
        // The page did not change into something else.
        assert_says(&page(&ctx), "Nothing matches");
    }

    #[test]
    fn the_commands_are_registered_so_they_can_be_run_by_name() {
        let (ctx, env) = editor();
        for name in [
            "describe-function",
            "describe-variable",
            "describe-key",
            "describe-bindings",
            "describe-mode",
            "apropos",
            "where-is-command",
            "help-for-help",
            "help-back",
            "help-follow",
        ] {
            let answer = run(&format!("(commandp '{name})"), &env, &ctx);
            assert!(
                !answer.is_nil(),
                "{name} should be a command anybody can run with M-x"
            );
        }
        let _: ELispExp<GapBuffer> = LispExp::nil();
    }
}
