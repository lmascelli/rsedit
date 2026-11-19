//! The menus: what Lisp registers, how a menu comes out when it opens --
//! conditions, generated entries, keys, the groups of a context menu, the
//! menus of the modes on the bar -- what choosing an entry runs, what a right
//! click and `menu-bar-open` ask the front end, and the double click that
//! chooses a line of a list.
//!
//! Design: `doc/architettura-editor.typ, <menu>`.
#[cfg(test)]
mod tests {
    use crate::buffer::{BufferTrait, gap_buffer::GapBuffer};
    use crate::editor::{EditorState, create_global_env};
    use crate::input::{KeyModifiers, MouseButton, MouseEvent, MouseKind};
    use crate::tests::shipped::load_default_modules;
    use crate::ui::{Menu, MenuItem, MenuItemId, MenuKind};
    use risp::{Env, EvalError, LispExp, Parser, eval};
    use std::sync::Arc;

    type Ctx = EditorState<GapBuffer>;

    const W: usize = 80;
    const H: usize = 24;

    fn eval_str(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> Result<LispExp<Ctx>, EvalError<Ctx>> {
        let ast = Parser::new(&format!("(progn {src})"))
            .next()
            .expect("test source must parse");
        eval(&ast, env.clone(), ctx)
    }

    fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) -> LispExp<Ctx> {
        eval_str(src, env, ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"))
    }

    /// An editor with the mouse on, *scratch* holding LINES numbered lines,
    /// and a frame composed so the windows know where they are.
    fn editor(lines: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
        env.set_variable("frame-width".into(), LispExp::number(W as f64));
        env.set_variable("frame-height".into(), LispExp::number(H as f64));
        run("(setq mouse-mode t)", &env, &ctx);
        let text: String = (0..lines).map(|n| format!("line {n}\n")).collect();
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from(text.as_str());
            b.text.cursor_move(0, 0);
        });
        let _ = ctx.snapshot(&env, W, H);
        (ctx, env)
    }

    /// The same, with `defcommand' and `define-minor-mode' to write with, and
    /// `message' for the second to call.
    fn editor_with_commands(lines: usize) -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor(lines);
        for module in ["commands", "debug"] {
            run(&crate::tests::shipped::module_source(module), &env, &ctx);
        }
        (ctx, env)
    }

    /// The labels of MENU's entries, a separator as `--`.
    fn labels(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .map(|item| item.label().unwrap_or("--").to_string())
            .collect()
    }

    /// The entry of MENU labelled LABEL, at its top level.
    fn item<'a>(menu: &'a Menu, label: &str) -> &'a MenuItem {
        menu.items
            .iter()
            .find(|item| item.label() == Some(label))
            .unwrap_or_else(|| panic!("no {label} in {:?}", labels(menu)))
    }

    /// The menu inside MENU labelled LABEL.
    fn submenu<'a>(menu: &'a Menu, label: &str) -> &'a Menu {
        match item(menu, label) {
            MenuItem::Submenu { menu, .. } => menu,
            other => panic!("{label} is no menu: {other:?}"),
        }
    }

    /// The id of the action labelled LABEL, which must be enabled or not as
    /// ENABLED says.
    fn action(menu: &Menu, label: &str, enabled: bool) -> MenuItemId {
        match item(menu, label) {
            MenuItem::Action {
                enabled: is, id, ..
            } => {
                assert_eq!(*is, enabled, "{label} enabled");
                *id
            }
            other => panic!("{label} is no action: {other:?}"),
        }
    }

    /// The keys shown beside the action labelled LABEL.
    fn keys_of(menu: &Menu, label: &str) -> String {
        match item(menu, label) {
            MenuItem::Action { keys, .. } => keys.clone(),
            other => panic!("{label} is no action: {other:?}"),
        }
    }

    fn mouse(ctx: &Ctx, env: &Arc<Env<Ctx>>, kind: MouseKind, column: u16, row: u16) -> bool {
        ctx.handle_mouse_event(
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::default(),
            },
            env,
        )
    }

    fn point(ctx: &Ctx) -> (usize, usize) {
        ctx.with_current_buffer(|b| b.text.cursor_pos())
    }

    // ----------------------------------------------------------------
    // Registering
    // ----------------------------------------------------------------

    #[test]
    fn an_entry_goes_at_the_end_of_its_path_and_the_menus_on_the_way_are_made() {
        let (ctx, env) = editor(3);
        run(
            r#"(menu-bar-add '("Tools" "Lines" "Count") 'count-words)
               (menu-bar-add '("Tools" "Grep") 'grep)"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        assert_eq!(labels(&bar), ["Tools"]);
        let tools = submenu(&bar, "Tools");
        assert_eq!(labels(tools), ["Lines", "Grep"]);
        assert_eq!(labels(submenu(tools, "Lines")), ["Count"]);
    }

    #[test]
    fn adding_an_entry_again_replaces_it_where_it_was() {
        // Loading a module twice must not double its entries.
        let (ctx, env) = editor(3);
        let source = r#"(menu-bar-add '("M" "A") 'undo)
                        (menu-bar-add '("M" "--one") nil)
                        (menu-bar-add '("M" "B") 'redo)"#;
        run(source, &env, &ctx);
        run(source, &env, &ctx);
        run(r#"(menu-bar-add '("M" "A") 'yank)"#, &env, &ctx);
        let bar = ctx.menu_bar(&env);
        let menu = submenu(&bar, "M");
        assert_eq!(labels(menu), ["A", "--", "B"]);
        let id = action(menu, "A", true);
        assert_eq!(
            ctx.menu_item_command(id),
            LispExp::symbol("yank".into()),
            "the second A is in the place of the first"
        );
    }

    #[test]
    fn two_separators_of_a_menu_are_told_apart_by_their_labels() {
        let (ctx, env) = editor(3);
        run(
            r#"(menu-bar-add '("M" "A") 'undo)
               (menu-bar-add '("M" "--x") nil)
               (menu-bar-add '("M" "B") 'undo)
               (menu-bar-add '("M" "--y") nil)
               (menu-bar-add '("M" "C") 'undo)"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        assert_eq!(labels(submenu(&bar, "M")), ["A", "--", "B", "--", "C"]);
    }

    #[test]
    fn an_entry_cannot_take_the_place_of_a_menu_nor_hold_one() {
        let (ctx, env) = editor(3);
        run(r#"(menu-bar-add '("File" "Open") 'find-file)"#, &env, &ctx);
        assert!(
            eval_str(r#"(menu-bar-add "File" 'find-file)"#, &env, &ctx).is_err(),
            "File is a menu"
        );
        assert!(
            eval_str(
                r#"(menu-bar-add '("File" "Open" "Deeper") 'undo)"#,
                &env,
                &ctx
            )
            .is_err(),
            "Open is an entry"
        );
        assert!(
            eval_str("(menu-bar-add nil 'undo)", &env, &ctx).is_err(),
            "a path with no label names nothing"
        );
        assert!(
            eval_str(r#"(menu-bar-add "Save" 42)"#, &env, &ctx).is_err(),
            "a number runs nothing"
        );
    }

    #[test]
    fn an_entry_is_taken_away_with_everything_in_it() {
        let (ctx, env) = editor(3);
        run(
            r#"(menu-bar-add '("A" "One") 'undo)
               (menu-bar-add '("B" "Two") 'undo)"#,
            &env,
            &ctx,
        );
        assert!(!run(r#"(menu-bar-remove "A")"#, &env, &ctx).is_nil());
        assert!(run(r#"(menu-bar-remove "A")"#, &env, &ctx).is_nil());
        assert!(!run(r#"(menu-bar-remove '("B" "Two"))"#, &env, &ctx).is_nil());
        let bar = ctx.menu_bar(&env);
        assert_eq!(labels(&bar), ["B"], "an emptied menu of the bar stays");
        assert!(submenu(&bar, "B").is_empty());
    }

    // ----------------------------------------------------------------
    // Resolving
    // ----------------------------------------------------------------

    #[test]
    fn an_entry_is_greyed_out_while_its_condition_says_nil() {
        let (ctx, env) = editor(3);
        run(
            r#"(defvar my-flag nil)
               (menu-bar-add '("M" "Var") 'undo 'my-flag)
               (menu-bar-add '("M" "Form") 'undo '(not my-flag))
               (menu-bar-add '("M" "Function") 'undo 'use-region-p)"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        let menu = submenu(&bar, "M");
        action(menu, "Var", false);
        action(menu, "Form", true);
        action(menu, "Function", false);

        run("(setq my-flag t) (set-mark) (forward-char)", &env, &ctx);
        let bar = ctx.menu_bar(&env);
        let menu = submenu(&bar, "M");
        action(menu, "Var", true);
        action(menu, "Form", false);
        action(menu, "Function", true);
    }

    #[test]
    fn a_condition_that_fails_greys_its_entry_out() {
        let (ctx, env) = editor(3);
        run(
            r#"(menu-bar-add '("M" "Broken") 'undo '(no-such-function))"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        action(submenu(&bar, "M"), "Broken", false);
    }

    #[test]
    fn an_entry_whose_command_does_not_exist_is_greyed_out() {
        // A module left out by `default-configuration-skip' leaves its
        // entries behind it.
        let (ctx, env) = editor(3);
        run(
            r#"(menu-bar-add '("M" "Gone") 'no-such-command)
               (menu-bar-add '("M" "Gone too") '(no-such-command 1))"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        let menu = submenu(&bar, "M");
        action(menu, "Gone", false);
        action(menu, "Gone too", false);
    }

    #[test]
    fn an_entry_shows_the_shortest_keys_that_run_its_command() {
        let (ctx, env) = editor(3);
        run(
            r#"(define-key nil "C-c u u" 'undo)
               (define-key nil "C-z" 'undo)
               (menu-bar-add '("M" "Undo") 'undo)
               (menu-bar-add '("M" "Nothing") 'count-words)
               (menu-bar-add '("M" "A form") '(undo))"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        let menu = submenu(&bar, "M");
        assert_eq!(keys_of(menu, "Undo"), "C-z");
        assert_eq!(keys_of(menu, "Nothing"), "");
        assert_eq!(keys_of(menu, "A form"), "", "a form shows no keys");
    }

    #[test]
    fn generated_entries_are_asked_for_each_time_the_menu_opens() {
        let (ctx, env) = editor(3);
        run(
            r#"(defvar my-names '("one"))
               (defun my-entries ()
                 (append (mapcar (lambda (name) (list name (list 'message name))) my-names)
                         (list "--" (list "Off" 'undo nil))))
               (menu-bar-add '("M" "First") 'undo)
               (menu-bar-add-dynamic "M" 'my-entries)
               (menu-bar-add '("M" "Last") 'undo)"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        let menu = submenu(&bar, "M");
        assert_eq!(labels(menu), ["First", "one", "--", "Off", "Last"]);
        action(menu, "Off", false);

        run(r#"(setq my-names '("two" "three"))"#, &env, &ctx);
        let bar = ctx.menu_bar(&env);
        assert_eq!(
            labels(submenu(&bar, "M")),
            ["First", "two", "three", "--", "Off", "Last"]
        );
    }

    #[test]
    fn an_empty_menu_inside_another_is_left_out_and_separators_do_not_pile_up() {
        let (ctx, env) = editor(3);
        run(
            r#"(defun my-none () nil)
               (menu-bar-add '("M" "--a") nil)
               (menu-bar-add '("M" "A") 'undo)
               (menu-bar-add '("M" "--b") nil)
               (menu-bar-add-dynamic '("M" "Empty") 'my-none)
               (menu-bar-add '("M" "--c") nil)
               (menu-bar-add '("M" "B") 'undo)
               (menu-bar-add '("M" "--d") nil)"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        assert_eq!(labels(submenu(&bar, "M")), ["A", "--", "B"]);
    }

    #[test]
    fn the_context_menu_has_the_modes_entries_first_then_those_of_every_buffer() {
        let (ctx, env) = editor_with_commands(3);
        run(
            r#"(make-mode 'my-mode)
               (define-minor-mode my-minor "M" "A minor mode for the test.")
               (context-menu-add nil "Everywhere" 'undo)
               (context-menu-add 'my-mode "Major" 'undo)
               (context-menu-add 'my-minor "Minor" 'undo)
               (context-menu-add 'other-mode "Elsewhere" 'undo)
               (set-major-mode 'my-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            labels(&ctx.context_menu(&env)),
            ["Major", "--", "Everywhere"]
        );
        run("(my-minor)", &env, &ctx);
        assert_eq!(
            labels(&ctx.context_menu(&env)),
            ["Major", "--", "Minor", "--", "Everywhere"]
        );
    }

    // ----------------------------------------------------------------
    // The menus of the modes on the bar
    // ----------------------------------------------------------------

    /// An editor whose bar has File and Help of its own, whose *scratch* is
    /// in my-mode, with Major on the bar, and where the minor modes one and
    /// two put One and Two on it: neither is on yet.
    fn editor_with_mode_menus() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor_with_commands(3);
        run(
            r#"(make-mode 'my-mode)
               (define-minor-mode one "1" "A minor mode for the test.")
               (define-minor-mode two "2" "Another.")
               (menu-bar-add '("File" "Open") 'find-file)
               (menu-bar-add '("Help" "Guide") '(manpage "guide"))
               (menu-bar-add '("Major" "Act") '(insert "major") nil :mode 'my-mode)
               (menu-bar-add '("One" "Act") '(insert "one") nil :mode 'one)
               (menu-bar-add '("Two" "Act") '(insert "two") nil :mode 'two)
               (menu-bar-add '("Elsewhere" "Act") 'undo nil :mode 'other-mode)
               (set-major-mode 'my-mode)"#,
            &env,
            &ctx,
        );
        (ctx, env)
    }

    #[test]
    fn the_menus_of_the_modes_go_between_the_editors_and_help() {
        let (ctx, env) = editor_with_mode_menus();
        assert_eq!(
            labels(&ctx.menu_bar(&env)),
            ["File", "--", "Major", "--", "Help"]
        );
        // The minor modes' after the major mode's, in the order they were
        // turned on, with no line between two of them.
        run("(two) (one)", &env, &ctx);
        assert_eq!(
            labels(&ctx.menu_bar(&env)),
            ["File", "--", "Major", "--", "Two", "One", "--", "Help"]
        );
        run("(two)", &env, &ctx);
        assert_eq!(
            labels(&ctx.menu_bar(&env)),
            ["File", "--", "Major", "--", "One", "--", "Help"]
        );
    }

    #[test]
    fn a_buffer_whose_modes_have_no_menus_shows_the_editors_and_help() {
        let (ctx, env) = editor_with_mode_menus();
        run("(set-major-mode 'fundamental-mode)", &env, &ctx);
        assert_eq!(labels(&ctx.menu_bar(&env)), ["File", "--", "Help"]);
        run(r#"(menu-bar-remove "Help")"#, &env, &ctx);
        assert_eq!(
            labels(&ctx.menu_bar(&env)),
            ["File"],
            "no line before nothing"
        );
    }

    #[test]
    fn an_entry_of_a_modes_menu_runs_as_any_other() {
        let (ctx, env) = editor_with_mode_menus();
        let bar = ctx.menu_bar(&env);
        let id = action(submenu(&bar, "Major"), "Act", true);
        assert!(ctx.run_menu_item(id, &env));
        assert!(matches!(
            run("(buffer-string)", &env, &ctx),
            LispExp::String(text) if text.starts_with("major")
        ));
    }

    #[test]
    fn menu_bar_final_items_says_which_menus_end_the_bar() {
        let (ctx, env) = editor_with_mode_menus();
        run(r#"(menu-bar-add '("Tools" "Grep") 'grep)"#, &env, &ctx);
        assert_eq!(
            labels(&ctx.menu_bar(&env)),
            ["File", "Tools", "--", "Major", "--", "Help"]
        );
        // In the order of the list, each once, and one that is not there
        // passed over.
        run(
            r#"(setq menu-bar-final-items '("Help" "Nowhere" "File" "Help"))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            labels(&ctx.menu_bar(&env)),
            ["Tools", "--", "Major", "--", "Help", "File"]
        );
        run("(setq menu-bar-final-items nil)", &env, &ctx);
        assert_eq!(
            labels(&ctx.menu_bar(&env)),
            ["File", "Help", "Tools", "--", "Major"]
        );
    }

    #[test]
    fn the_menus_of_a_mode_are_taken_away_with_mode() {
        let (ctx, env) = editor_with_mode_menus();
        run(
            r#"(menu-bar-add '("Major" "More") 'undo nil :mode 'my-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            labels(submenu(&ctx.menu_bar(&env), "Major")),
            ["Act", "More"]
        );
        assert!(run(r#"(menu-bar-remove '("Major" "Act"))"#, &env, &ctx).is_nil());
        assert!(
            !run(
                r#"(menu-bar-remove '("Major" "Act") :mode 'my-mode)"#,
                &env,
                &ctx
            )
            .is_nil()
        );
        assert_eq!(labels(submenu(&ctx.menu_bar(&env), "Major")), ["More"]);
        assert!(!run(r#"(menu-bar-remove "Major" :mode 'my-mode)"#, &env, &ctx).is_nil());
        assert_eq!(labels(&ctx.menu_bar(&env)), ["File", "--", "Help"]);
    }

    #[test]
    fn a_mode_fills_a_menu_of_its_own_each_time_it_opens() {
        let (ctx, env) = editor_with_mode_menus();
        run(
            r#"(defvar my-count 0)
               (defun my-entries ()
                 (setq my-count (+ my-count 1))
                 (list (list (format "Opened %s" my-count) 'undo)))
               (menu-bar-add-dynamic "Major" 'my-entries :mode 'my-mode)"#,
            &env,
            &ctx,
        );
        assert_eq!(
            labels(submenu(&ctx.menu_bar(&env), "Major")),
            ["Act", "Opened 1"]
        );
        assert_eq!(
            labels(submenu(&ctx.menu_bar(&env), "Major")),
            ["Act", "Opened 2"]
        );
    }

    #[test]
    fn the_menu_bars_keyword_is_mode_and_only_mode() {
        let (ctx, env) = editor_with_mode_menus();
        for src in [
            r#"(menu-bar-add '("M" "A") 'undo nil :major 'my-mode)"#,
            r#"(menu-bar-add '("M" "A") 'undo nil :mode)"#,
            r#"(menu-bar-add '("M" "A") 'undo nil :mode 'my-mode :mode 'my-mode)"#,
            r#"(menu-bar-add '("M" "A") 'undo nil :mode 3)"#,
            r#"(menu-bar-remove "M" :mode)"#,
        ] {
            assert!(eval_str(src, &env, &ctx).is_err(), "{src} is refused");
        }
        // ENABLE may be left out before the keyword; nil is the editor's own.
        run(
            r#"(menu-bar-add '("Major" "Short") 'undo :mode 'my-mode)
               (menu-bar-add '("Own" "A") 'undo nil :mode nil)"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        assert_eq!(labels(submenu(&bar, "Major")), ["Act", "Short"]);
        assert_eq!(labels(&bar), ["File", "Own", "--", "Major", "--", "Help"]);
    }

    #[test]
    fn the_frame_names_the_menus_of_the_modes_with_a_line_between_groups() {
        let (ctx, env) = editor_with_mode_menus();
        run("(setq menu-bar-mode t) (one)", &env, &ctx);
        let names = ["File", "--", "Major", "--", "One", "--", "Help"];
        assert_eq!(ctx.snapshot(&env, W, H).menu_bar, names);
        assert_eq!(ctx.menu_bar(&env).bar_labels(), names, "one for each entry");
        // While a prompt is open, the bar stays that of the buffer it was
        // opened from rather than becoming the prompt's.
        run(r#"(minibuffer-read "Q:" 'ignore nil nil)"#, &env, &ctx);
        assert_eq!(ctx.snapshot(&env, W, H).menu_bar, names);
    }

    // ----------------------------------------------------------------
    // Choosing
    // ----------------------------------------------------------------

    #[test]
    fn choosing_an_entry_runs_its_command_as_a_key_runs_it() {
        let (ctx, env) = editor_with_commands(3);
        run(
            r#"(defcommand my-mark-here () nil (insert "X"))
               (menu-bar-add '("M" "Mark") 'my-mark-here)
               (menu-bar-add '("M" "Form") '(insert "Y"))"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        let menu = submenu(&bar, "M");
        assert!(ctx.run_menu_item(action(menu, "Mark", true), &env));
        assert!(ctx.run_menu_item(action(menu, "Form", true), &env));
        assert!(
            ctx.with_current_buffer(|b| b.text.to_string())
                .starts_with("XYline 0")
        );
        assert!(
            ctx.last_command_is("insert"),
            "remembered as a key's command is"
        );
        run("(undo)", &env, &ctx);
        assert!(
            ctx.with_current_buffer(|b| b.text.to_string())
                .starts_with("Xline 0"),
            "one step of undo each"
        );
    }

    #[test]
    fn an_entry_of_a_menu_resolved_again_since_is_refused() {
        let (ctx, env) = editor(3);
        run(r#"(menu-bar-add '("M" "Mark") '(insert "X"))"#, &env, &ctx);
        let bar = ctx.menu_bar(&env);
        let stale = action(submenu(&bar, "M"), "Mark", true);
        let _ = ctx.menu_bar(&env);
        assert!(!ctx.run_menu_item(stale, &env));
        assert!(
            ctx.with_current_buffer(|b| b.text.to_string())
                .starts_with("line 0")
        );
    }

    #[test]
    fn a_greyed_out_entry_is_refused() {
        let (ctx, env) = editor(3);
        run(
            r#"(menu-bar-add '("M" "Mark") '(insert "X") nil)"#,
            &env,
            &ctx,
        );
        run(
            r#"(menu-bar-add '("M" "Off") '(insert "X") '(= 1 2))"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        // A nil condition is no condition, as an argument left out is.
        action(submenu(&bar, "M"), "Mark", true);
        let off = action(submenu(&bar, "M"), "Off", false);
        assert!(!ctx.run_menu_item(off, &env));
        assert!(
            ctx.with_current_buffer(|b| b.text.to_string())
                .starts_with("line 0")
        );
    }

    #[test]
    fn a_context_menu_resolved_does_not_spoil_the_bar_that_is_open() {
        // A window keeps its bar's menu open while a right click asks for a
        // context menu: each kind has its own ids.
        let (ctx, env) = editor(3);
        run(
            r#"(menu-bar-add '("M" "Bar") '(insert "B"))
               (context-menu-add nil "Context" '(insert "C"))"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        let context = ctx.context_menu(&env);
        assert!(ctx.run_menu_item(action(&context, "Context", true), &env));
        assert!(ctx.run_menu_item(action(submenu(&bar, "M"), "Bar", true), &env));
        assert!(
            ctx.with_current_buffer(|b| b.text.to_string())
                .starts_with("CBline 0")
        );
    }

    #[test]
    fn while_a_prompt_is_open_no_menu_opens_and_no_entry_runs() {
        let (ctx, env) = editor(3);
        run(
            r#"(menu-bar-add '("M" "Mark") '(insert "X"))
               (context-menu-add nil "Undo" 'undo)"#,
            &env,
            &ctx,
        );
        let bar = ctx.menu_bar(&env);
        let id = action(submenu(&bar, "M"), "Mark", true);
        run(r#"(minibuffer-read "Q:" 'ignore nil nil)"#, &env, &ctx);
        assert!(run("(menu-bar-open)", &env, &ctx).is_nil());
        assert!(run("(context-menu-open)", &env, &ctx).is_nil());
        assert!(ctx.snapshot(&env, W, H).menu.is_none());
        assert!(!ctx.run_menu_item(id, &env));
    }

    // ----------------------------------------------------------------
    // What the front end is asked
    // ----------------------------------------------------------------

    #[test]
    fn a_right_click_asks_for_the_context_menu_where_it_landed() {
        let (ctx, env) = editor(10);
        run(r#"(context-menu-add nil "Undo" 'undo)"#, &env, &ctx);
        assert!(mouse(&ctx, &env, MouseKind::Down(MouseButton::Right), 3, 2));
        assert_eq!(point(&ctx), (2, 3), "point goes where the click landed");
        let frame = ctx.snapshot(&env, W, H);
        let request = frame.menu.expect("a menu to open");
        assert_eq!(request.kind, MenuKind::Context);
        assert_eq!(request.at, Some((3, 2)));
        assert_eq!(labels(&request.menu), ["Undo"]);
        assert!(
            ctx.snapshot(&env, W, H).menu.is_none(),
            "asked on one frame only"
        );
    }

    #[test]
    fn a_right_click_in_the_selection_keeps_it() {
        let (ctx, env) = editor(10);
        run(
            r#"(context-menu-add nil "Copy" 'kill-ring-save 'use-region-p)"#,
            &env,
            &ctx,
        );
        // From line 1 to line 3.
        run("(goto-line 2) (set-mark) (goto-line 4)", &env, &ctx);
        mouse(&ctx, &env, MouseKind::Down(MouseButton::Right), 2, 2);
        assert_eq!(point(&ctx), (3, 0), "point stays where it was");
        let menu = ctx.snapshot(&env, W, H).menu.expect("a menu").menu;
        action(&menu, "Copy", true);

        mouse(&ctx, &env, MouseKind::Down(MouseButton::Right), 2, 6);
        assert_eq!(point(&ctx), (6, 2), "outside it, a click");
        let menu = ctx.snapshot(&env, W, H).menu.expect("a menu").menu;
        action(&menu, "Copy", false);
    }

    #[test]
    fn an_empty_context_menu_asks_for_nothing() {
        let (ctx, env) = editor(3);
        mouse(&ctx, &env, MouseKind::Down(MouseButton::Right), 1, 1);
        assert!(ctx.snapshot(&env, W, H).menu.is_none());
    }

    #[test]
    fn menu_bar_open_asks_for_the_first_menu_of_the_bar() {
        let (ctx, env) = editor(3);
        run(
            r#"(menu-bar-add '("File" "Open") 'find-file)
               (menu-bar-add '("Help" "Guide") '(manpage "guide"))"#,
            &env,
            &ctx,
        );
        assert!(!run("(menu-bar-open)", &env, &ctx).is_nil());
        let request = ctx.snapshot(&env, W, H).menu.expect("a menu to open");
        assert_eq!(request.kind, MenuKind::Bar { open: 0 });
        assert_eq!(request.at, None);
        assert_eq!(labels(&request.menu), ["File", "--", "Help"]);
    }

    #[test]
    fn the_frame_names_the_menus_of_the_bar_only_while_menu_bar_mode_is_on() {
        let (ctx, env) = editor(3);
        run(r#"(menu-bar-add '("File" "Open") 'find-file)"#, &env, &ctx);
        assert!(ctx.snapshot(&env, W, H).menu_bar.is_empty());
        run("(setq menu-bar-mode t)", &env, &ctx);
        assert_eq!(ctx.snapshot(&env, W, H).menu_bar, ["File"]);
    }

    #[test]
    fn the_items_of_a_menu_read_back_as_lisp() {
        let (ctx, env) = editor(3);
        run(
            r#"(define-key nil "C-z" 'undo)
               (menu-bar-add '("M" "Undo") 'undo)
               (menu-bar-add '("M" "--") nil)
               (menu-bar-add '("M" "Sub" "Off") 'undo '(= 1 2))"#,
            &env,
            &ctx,
        );
        assert_eq!(
            run("(menu-bar-items)", &env, &ctx),
            run(
                r#"'(("M" (("Undo" undo t "C-z") "--" ("Sub" (("Off" undo nil "C-z"))))))"#,
                &env,
                &ctx
            )
        );
    }

    // ----------------------------------------------------------------
    // The double click
    // ----------------------------------------------------------------

    #[test]
    fn a_double_click_runs_the_command_its_mode_names() {
        let (ctx, env) = editor_with_commands(10);
        run(
            r#"(make-mode 'my-list-mode)
               (defvar my-chosen nil)
               (defcommand my-choose () nil (setq my-chosen (line-number-at-point)))
               (put 'my-list-mode 'double-click-command 'my-choose)
               (set-major-mode 'my-list-mode)"#,
            &env,
            &ctx,
        );
        assert!(mouse(&ctx, &env, MouseKind::Down(MouseButton::Left), 2, 4));
        assert!(
            run("my-chosen", &env, &ctx).is_nil(),
            "one click only moves"
        );
        assert!(mouse(&ctx, &env, MouseKind::Down(MouseButton::Left), 2, 4));
        assert_eq!(run("my-chosen", &env, &ctx), LispExp::number(5.0));
        let marked = ctx.with_current_buffer(|b| b.mark.is_some_and(|m| m.active));
        assert!(!marked, "and selects nothing");
    }

    #[test]
    fn a_double_click_where_no_mode_names_a_command_starts_a_selection() {
        let (ctx, env) = editor(10);
        mouse(&ctx, &env, MouseKind::Down(MouseButton::Left), 2, 4);
        mouse(&ctx, &env, MouseKind::Down(MouseButton::Left), 2, 4);
        let marked = ctx.with_current_buffer(|b| b.mark.is_some_and(|m| m.active));
        assert!(marked);
    }

    // ----------------------------------------------------------------
    // What the editor comes with
    // ----------------------------------------------------------------

    /// The editor as init.lisp leaves it, every module loaded, the mouse on.
    fn shipped_editor() -> (Ctx, Arc<Env<Ctx>>) {
        let (ctx, env) = editor(10);
        load_default_modules(&env, &ctx);
        run("(setq mouse-mode t)", &env, &ctx);
        let _ = ctx.snapshot(&env, W, H);
        (ctx, env)
    }

    #[test]
    fn the_menu_bar_comes_with_file_edit_buffers_and_help() {
        let (ctx, env) = shipped_editor();
        assert!(!run("menu-bar-mode", &env, &ctx).is_nil());
        let bar = ctx.menu_bar(&env);
        assert_eq!(labels(&bar), ["File", "Edit", "Buffers", "--", "Help"]);
        let edit = submenu(&bar, "Edit");
        action(edit, "Cut", false);
        action(edit, "Paste", true);
        assert_eq!(keys_of(edit, "Replace..."), "M-%");
        submenu(edit, "Keyboard Macro");
        assert_eq!(keys_of(submenu(&bar, "File"), "Open File..."), "C-x C-f");
        let buffers = labels(submenu(&bar, "Buffers"));
        assert!(
            buffers.iter().any(|label| label.starts_with("*scratch*")),
            "{buffers:?}"
        );
        assert_eq!(buffers.last().map(String::as_str), Some("List All Buffers"));
        submenu(submenu(&bar, "Help"), "Describe");
    }

    /// Puts the current buffer in MAJOR, with MINORS on, as their modes
    /// leave it but without running their hooks: no server started, no
    /// listing read.
    fn put_in_modes(ctx: &Ctx, major: &str, minors: &[&str]) {
        ctx.with_current_buffer_mut(|b| {
            b.current_mode = major.into();
            b.minor_modes = minors
                .iter()
                .map(|name| crate::buffer::ActiveMinor {
                    name: name.to_string(),
                    lighter: String::new(),
                })
                .collect();
        });
    }

    #[test]
    fn a_rust_buffer_with_a_server_has_the_rust_and_lsp_menus_before_help() {
        let (ctx, env) = shipped_editor();
        put_in_modes(&ctx, "rust-mode", &["lsp-mode"]);
        let bar = ctx.menu_bar(&env);
        assert_eq!(
            labels(&bar),
            [
                "File", "Edit", "Buffers", "--", "Rust", "--", "LSP", "--", "Help"
            ]
        );
        action(submenu(&bar, "Rust"), "Build", true);
        action(submenu(&bar, "Rust"), "Start Language Server", false);
        assert_eq!(keys_of(submenu(&bar, "LSP"), "Find Definition"), "M-.");
    }

    /// Returns the name of the function the command of ACTION, an entry of
    /// a menu resolved, calls: its name, or the head of its form.
    fn called(ctx: &Ctx, id: MenuItemId) -> Option<String> {
        let command = ctx.menu_item_command(id);
        let head = match &command {
            LispExp::Symbol(_) => Some(command.clone()),
            other => other.iter().next(),
        };
        match head {
            Some(LispExp::Symbol(name)) => Some(name.to_string()),
            _ => None,
        }
    }

    /// Returns the functions MENU's actions call that do not exist, with the
    /// labels of where they are.
    fn missing(ctx: &Ctx, env: &Arc<Env<Ctx>>, menu: &Menu, path: &str) -> Vec<String> {
        let mut out = Vec::new();
        for item in &menu.items {
            match item {
                MenuItem::Action { label, id, .. } => match called(ctx, *id) {
                    Some(name) if env.get_function(&name).is_some() => {}
                    other => out.push(format!("{path} > {label}: {other:?}")),
                },
                MenuItem::Submenu { label, menu } => {
                    out.extend(missing(ctx, env, menu, &format!("{path} > {label}")))
                }
                MenuItem::Separator => {}
            }
        }
        out
    }

    #[test]
    fn every_entry_of_the_shipped_menus_calls_a_function_there_is() {
        let (ctx, env) = shipped_editor();
        let modes: &[(&str, &[&str], &str)] = &[
            ("fundamental-mode", &[], "Edit"),
            ("rust-mode", &["lsp-mode"], "Rust"),
            ("python-mode", &[], "Python"),
            ("c-mode", &[], "C"),
            ("cpp-mode", &[], "C++"),
            ("cmake-mode", &[], "CMake"),
            ("bash-mode", &[], "Bash"),
            ("toml-mode", &[], "TOML"),
            ("risp-mode", &[], "Risp"),
            ("dired-mode", &[], "Dired"),
            ("buffer-list-mode", &[], "Buffer List"),
            ("compilation-mode", &[], "Compile"),
            ("occur-mode", &[], "Occur"),
            ("findings-list-mode", &[], "Findings"),
            ("help-mode", &[], "Page"),
            ("manpage-mode", &[], "Manual"),
            ("theme-list-mode", &[], "Themes"),
            ("shell-output-mode", &[], "Shell"),
            ("lsp-locations-mode", &[], "Locations"),
        ];
        let mut wrong = Vec::new();
        for (major, minors, menu) in modes {
            put_in_modes(&ctx, major, minors);
            let bar = ctx.menu_bar(&env);
            if !labels(&bar).iter().any(|label| label == menu) {
                wrong.push(format!("{major}: no {menu} in {:?}", labels(&bar)));
            }
            wrong.extend(missing(&ctx, &env, &bar, major));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn a_buffer_chosen_from_the_buffers_menu_is_shown() {
        let (ctx, env) = shipped_editor();
        run(r#"(buffer-create "notes")"#, &env, &ctx);
        let bar = ctx.menu_bar(&env);
        let id = action(submenu(&bar, "Buffers"), "notes", true);
        assert!(ctx.run_menu_item(id, &env));
        assert_eq!(
            run("(current-buffer)", &env, &ctx),
            LispExp::string("notes".into())
        );
    }

    #[test]
    fn the_shipped_context_menu_offers_copy_only_with_a_selection() {
        let (ctx, env) = shipped_editor();
        let menu = ctx.context_menu(&env);
        assert_eq!(
            labels(&menu),
            [
                "Undo",
                "Redo",
                "--",
                "Cut",
                "Copy",
                "Paste",
                "--",
                "Select All"
            ]
        );
        action(&menu, "Copy", false);
        run("(set-mark) (forward-char)", &env, &ctx);
        action(&ctx.context_menu(&env), "Copy", true);
    }

    #[test]
    fn the_shipped_context_menu_offers_the_servers_actions_in_lsp_mode() {
        let (ctx, env) = shipped_editor();
        // On in the buffer's list, as the mode leaves it, without a server
        // started for it.
        ctx.with_current_buffer_mut(|b| {
            b.minor_modes.push(crate::buffer::ActiveMinor {
                name: "lsp-mode".into(),
                lighter: "LSP".into(),
            })
        });
        let menu = ctx.context_menu(&env);
        assert_eq!(
            labels(&menu)[..5],
            [
                "Find Definition",
                "Find References",
                "Go Back",
                "Code Action...",
                "--"
            ]
        );
    }

    #[test]
    fn a_double_click_on_a_line_of_an_occur_list_visits_its_match() {
        let (ctx, env) = shipped_editor();
        ctx.with_buffer_mut("*scratch*", |b| {
            b.text = GapBuffer::from("one TODO\ntwo\nthree TODO\n");
            b.text.cursor_move(0, 0);
        });
        run(r#"(occur "TODO")"#, &env, &ctx);
        let frame = ctx.snapshot(&env, W, H);
        let list = frame
            .views
            .iter()
            .find(|view| view.buffer_name == "*Occur*")
            .expect("the list is shown");
        // The third line of the list: the second match, line 3.
        let (x, y) = (list.rect.x as u16 + 2, list.rect.y as u16 + 2);
        mouse(&ctx, &env, MouseKind::Down(MouseButton::Left), x, y);
        mouse(&ctx, &env, MouseKind::Down(MouseButton::Left), x, y);
        assert_eq!(
            run("(list (current-buffer) (line-number-at-point))", &env, &ctx),
            run(r#"(list "*scratch*" 3)"#, &env, &ctx)
        );
    }
}
