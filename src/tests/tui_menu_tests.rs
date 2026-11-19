//! The menus in the terminal: the bar on the row above the frame, the boxes
//! walked with the keys and the mouse, where a box opens, and what is drawn
//! where while a menu is open.
//!
//! Design: `doc/architettura-editor.typ, <menu>`.
use crate::tui::colour::ColorDepth;
use crate::tui::menu::{OpenMenus, Outcome, bar_label_at, bar_layout};
use crate::tui::open_requested;
use crate::tui::render::render_screen;
use rsedit_core::{
    EditorState, Env, GapBuffer, KeyCode, KeyEvent, KeyModifiers, Menu, MenuItem, MenuKind,
    MouseButton, MouseEvent, MouseKind, Parser, create_global_env, eval, isolate_config_for_tests,
};
use std::sync::Arc;

type Ctx = EditorState<GapBuffer>;

const SCREEN: (usize, usize) = (80, 24);

fn editor() -> (Ctx, Arc<Env<Ctx>>) {
    isolate_config_for_tests();
    let (ctx, env) = create_global_env::<GapBuffer>().expect("global env");
    run(
        r#"(setq mouse-mode t)
           (menu-bar-add '("File" "Open") '(insert "open"))
           (menu-bar-add '("File" "--a") nil)
           (menu-bar-add '("File" "Off") '(insert "off") '(not t))
           (menu-bar-add '("File" "Recent" "One") '(insert "one"))
           (menu-bar-add '("File" "Save") '(insert "save"))
           (menu-bar-add '("Edit" "Guide") '(insert "guide"))
           (menu-bar-add "Run" '(insert "run"))"#,
        &env,
        &ctx,
    );
    let _ = ctx.snapshot(&env, SCREEN.0, SCREEN.1 - 1);
    (ctx, env)
}

fn run(src: &str, env: &Arc<Env<Ctx>>, ctx: &Ctx) {
    let ast = Parser::new(&format!("(progn {src})"))
        .next()
        .expect("test source must parse");
    eval(&ast, env.clone(), ctx).unwrap_or_else(|why| panic!("evaluating {src}: {why:?}"));
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(c),
        modifiers: KeyModifiers {
            ctrl: true,
            ..KeyModifiers::default()
        },
    }
}

/// The current buffer's text.
fn text(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> String {
    let ast = Parser::new("(buffer-string)").next().expect("parses");
    match eval(&ast, env.clone(), ctx) {
        Ok(rsedit_core::ELispExp::String(text)) => text.to_string(),
        other => panic!("buffer-string answered {other:?}"),
    }
}

/// Runs what OUTCOME names, as the loop does.
fn act(outcome: Outcome, ctx: &Ctx, env: &Arc<Env<Ctx>>) {
    if let Outcome::Run(id) = outcome {
        assert!(ctx.run_menu_item(id, env));
    }
}

/// The labels of the bar the editor holds.
fn bar_labels(ctx: &Ctx, env: &Arc<Env<Ctx>>) -> Vec<String> {
    ctx.menu_bar(env)
        .labels()
        .into_iter()
        .map(str::to_string)
        .collect()
}

// -----------------------------------------------------------------
// The bar
// -----------------------------------------------------------------

#[test]
fn the_labels_of_the_bar_are_spaced_and_each_owns_its_gaps() {
    let labels: Vec<String> = ["File", "Buffers", "Help"].map(String::from).to_vec();
    assert_eq!(bar_layout(&labels), [(1, 5), (7, 14), (16, 20)]);
    assert_eq!(bar_label_at(&labels, 0), Some(0));
    assert_eq!(bar_label_at(&labels, 4), Some(0));
    assert_eq!(bar_label_at(&labels, 6), Some(1));
    assert_eq!(bar_label_at(&labels, 15), Some(2));
    assert_eq!(bar_label_at(&labels, 40), None);
}

#[test]
fn a_separator_of_the_bar_takes_one_cell_and_opens_nothing() {
    let labels: Vec<String> = ["File", "--", "Rust", "--", "Help"]
        .map(String::from)
        .to_vec();
    assert_eq!(
        bar_layout(&labels),
        [(1, 5), (7, 8), (10, 14), (16, 17), (19, 23)]
    );
    assert_eq!(bar_label_at(&labels, 6), None, "the gap before the rule");
    assert_eq!(bar_label_at(&labels, 7), None, "the rule");
    assert_eq!(bar_label_at(&labels, 9), Some(2));
    assert_eq!(bar_label_at(&labels, 19), Some(4));
}

/// The editor of [`editor`], with a mode of its own on in *scratch*, which
/// puts Mode on the bar, and Help at the end of it: File Edit Run | Mode |
/// Help.
fn editor_with_mode_menus() -> (Ctx, Arc<Env<Ctx>>) {
    let (ctx, env) = editor();
    run(
        r#"(menu-bar-add '("Help" "Keys") '(insert "keys"))
           (make-mode 'menu-test-mode)
           (set-major-mode 'menu-test-mode)
           (menu-bar-add '("Mode" "Act") '(insert "act") nil :mode 'menu-test-mode)"#,
        &env,
        &ctx,
    );
    (ctx, env)
}

#[test]
fn the_bar_draws_a_rule_between_the_editors_menus_the_modes_and_help() {
    let (ctx, env) = editor_with_mode_menus();
    run("(setq menu-bar-mode t)", &env, &ctx);
    let frame = ctx.snapshot(&env, SCREEN.0, SCREEN.1 - 1);
    assert_eq!(
        frame.menu_bar,
        ["File", "Edit", "Run", "--", "Mode", "--", "Help"]
    );
    let mut out = Vec::new();
    render_screen(&mut out, &frame, ColorDepth::TrueColor, 1, None).expect("render");
    let drawn = String::from_utf8(out).expect("UTF-8");
    let rule = bar_layout(&frame.menu_bar)[3].0 + 1;
    assert!(drawn.contains(&format!("\u{1b}[1;{rule}H│")), "{drawn:?}");
    assert!(
        !drawn.contains(&format!("\u{1b}[1;{rule}H--")),
        "the separator not drawn as its label"
    );
}

#[test]
fn left_and_right_on_the_bar_step_over_its_rules() {
    let (ctx, env) = editor_with_mode_menus();
    let bar = ctx.menu_bar(&env);
    assert_eq!(
        bar.bar_labels(),
        ["File", "Edit", "Run", "--", "Mode", "--", "Help"]
    );
    let mut menus = OpenMenus::bar(bar, 2, SCREEN, true);
    menus.key(&key(KeyCode::Right), SCREEN);
    assert_eq!(menus.bar_open(), Some(4), "Mode, past the rule");
    let outcome = menus.key(&key(KeyCode::Enter), SCREEN);
    act(outcome, &ctx, &env);
    assert!(text(&ctx, &env).starts_with("act"), "{}", text(&ctx, &env));
    menus.key(&key(KeyCode::Right), SCREEN);
    assert_eq!(menus.bar_open(), Some(6), "Help");
    menus.key(&key(KeyCode::Left), SCREEN);
    menus.key(&key(KeyCode::Left), SCREEN);
    assert_eq!(menus.bar_open(), Some(2), "back to Run");
}

#[test]
fn a_press_on_a_rule_of_the_bar_opens_nothing() {
    let (ctx, env) = editor_with_mode_menus();
    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, false);
    let labels = menus.bar_labels().expect("opened from the bar");
    let rule = bar_layout(&labels)[3].0;
    assert_eq!(menus.click(rule, 0, SCREEN), Outcome::Close);
    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, false);
    let mode = bar_layout(&labels)[4].0;
    assert_eq!(menus.click(mode, 0, SCREEN), Outcome::Stay);
    assert_eq!(menus.bar_open(), Some(4));
}

// -----------------------------------------------------------------
// The keys
// -----------------------------------------------------------------

#[test]
fn the_arrows_walk_past_separators_and_greyed_out_entries() {
    let (ctx, env) = editor();
    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, true);
    // Open, then Recent: the separator and Off are not stopped on.
    assert_eq!(menus.key(&key(KeyCode::Down), SCREEN), Outcome::Stay);
    assert_eq!(menus.key(&ctrl('n'), SCREEN), Outcome::Stay);
    // Save, and round to Open again.
    menus.key(&key(KeyCode::Down), SCREEN);
    let outcome = menus.key(&key(KeyCode::Enter), SCREEN);
    act(outcome, &ctx, &env);
    assert!(text(&ctx, &env).starts_with("open"), "{}", text(&ctx, &env));
}

#[test]
fn right_opens_the_menu_inside_and_left_closes_it() {
    let (ctx, env) = editor();
    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, true);
    menus.key(&key(KeyCode::Up), SCREEN);
    menus.key(&key(KeyCode::Up), SCREEN);
    // On Recent.
    menus.key(&key(KeyCode::Right), SCREEN);
    let outcome = menus.key(&key(KeyCode::Enter), SCREEN);
    act(outcome, &ctx, &env);
    assert!(text(&ctx, &env).starts_with("one"), "{}", text(&ctx, &env));

    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, true);
    menus.key(&key(KeyCode::Up), SCREEN);
    menus.key(&key(KeyCode::Up), SCREEN);
    menus.key(&key(KeyCode::Right), SCREEN);
    menus.key(&key(KeyCode::Left), SCREEN);
    assert_eq!(menus.bar_open(), Some(0), "back in File");
    let outcome = menus.key(&key(KeyCode::Enter), SCREEN);
    assert_eq!(outcome, Outcome::Stay, "Enter on Recent opens it again");
}

#[test]
fn left_and_right_on_the_bar_go_from_menu_to_menu() {
    let (ctx, env) = editor();
    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, true);
    menus.key(&key(KeyCode::Right), SCREEN);
    assert_eq!(menus.bar_open(), Some(1));
    menus.key(&key(KeyCode::Right), SCREEN);
    assert_eq!(menus.bar_open(), Some(2), "Run, an entry of its own");
    let outcome = menus.key(&key(KeyCode::Enter), SCREEN);
    act(outcome, &ctx, &env);
    assert!(text(&ctx, &env).starts_with("run"));
    menus.key(&key(KeyCode::Right), SCREEN);
    assert_eq!(menus.bar_open(), Some(0), "round the end");
    menus.key(&key(KeyCode::Left), SCREEN);
    assert_eq!(menus.bar_open(), Some(2));
}

#[test]
fn a_letter_goes_to_the_entry_it_begins_and_escape_closes() {
    let (ctx, env) = editor();
    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, true);
    menus.key(&key(KeyCode::Char('s')), SCREEN);
    let outcome = menus.key(&key(KeyCode::Enter), SCREEN);
    act(outcome, &ctx, &env);
    assert!(text(&ctx, &env).starts_with("save"));

    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, true);
    assert_eq!(menus.key(&key(KeyCode::Esc), SCREEN), Outcome::Close);
    assert_eq!(menus.key(&ctrl('g'), SCREEN), Outcome::Close);
}

// -----------------------------------------------------------------
// The mouse
// -----------------------------------------------------------------

#[test]
fn a_press_on_an_entry_chooses_it_and_one_elsewhere_closes() {
    let (ctx, env) = editor();
    // File's box is under its label: its corner at (0, 1), Open on row 2.
    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, false);
    assert_eq!(menus.click(3, 3, SCREEN), Outcome::Stay, "the rule");
    assert_eq!(menus.click(3, 4, SCREEN), Outcome::Stay, "Off");
    let outcome = menus.click(3, 2, SCREEN);
    act(outcome, &ctx, &env);
    assert!(text(&ctx, &env).starts_with("open"));

    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, false);
    assert_eq!(menus.click(60, 12, SCREEN), Outcome::Close);
}

#[test]
fn a_press_on_another_label_of_the_bar_opens_its_menu() {
    let (ctx, env) = editor();
    let labels = bar_labels(&ctx, &env);
    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, false);
    let edit = bar_layout(&labels)[1].0;
    assert_eq!(menus.click(edit, 0, SCREEN), Outcome::Stay);
    assert_eq!(menus.bar_open(), Some(1));
    assert_eq!(
        menus.click(edit, 0, SCREEN),
        Outcome::Close,
        "the open one's label closes it"
    );
}

#[test]
fn a_box_near_an_edge_opens_where_it_fits() {
    let (ctx, env) = editor();
    let mut menus = OpenMenus::context(ctx.menu_bar(&env), (79, 23), SCREEN, true);
    // Its three entries, File, Edit and Run, in a box that ends on the last
    // row and column.
    let mut out = Vec::new();
    menus
        .draw(
            &mut out,
            &rsedit_core::Theme::default(),
            ColorDepth::TrueColor,
            SCREEN,
        )
        .expect("draw");
    let drawn = String::from_utf8(out).expect("UTF-8");
    assert!(
        drawn.contains("\u{1b}[24;"),
        "the bottom edge is on the last row"
    );
    assert!(!drawn.contains("\u{1b}[25;"), "and nothing below it");
    // And a press on its first entry chooses File, a menu.
    let width = drawn
        .split('\u{1b}')
        .find_map(|chunk| {
            chunk
                .contains('┌')
                .then(|| chunk.chars().filter(|&c| c == '─').count())
        })
        .expect("a top edge");
    let x = 80 - (width + 2) + 2;
    assert_eq!(menus.click(x, 24 - 5 + 1, SCREEN), Outcome::Stay);
}

#[test]
fn a_move_over_an_entry_selects_it_for_the_keys() {
    let (ctx, env) = editor();
    let mut menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, false);
    menus.hover(3, 6);
    let outcome = menus.key(&key(KeyCode::Enter), SCREEN);
    act(outcome, &ctx, &env);
    assert!(text(&ctx, &env).starts_with("save"));
}

// -----------------------------------------------------------------
// The screen
// -----------------------------------------------------------------

#[test]
fn with_the_bar_the_frame_is_drawn_a_row_down_and_the_bar_on_top() {
    let (ctx, env) = editor();
    run("(setq menu-bar-mode t)", &env, &ctx);
    let frame = ctx.snapshot(&env, SCREEN.0, SCREEN.1 - 1);
    assert_eq!(frame.menu_bar, ["File", "Edit", "Run"]);
    let mut out = Vec::new();
    render_screen(&mut out, &frame, ColorDepth::TrueColor, 1, None).expect("render");
    let drawn = String::from_utf8(out).expect("UTF-8");
    // The bar's labels, on the first row.
    assert!(drawn.contains("\u{1b}[1;2HFile"), "{drawn:?}");
    // The status line of the only window, on the row above the echo area:
    // the last but one of the screen, not of the frame.
    let status = format!("\u{1b}[{};1H", SCREEN.1 - 1);
    assert!(drawn.contains(&status), "{drawn:?}");
    assert!(
        drawn.ends_with("\u{1b}[?25h"),
        "the cursor shown at the end"
    );
}

#[test]
fn while_a_menu_is_open_it_is_drawn_over_the_frame_and_the_cursor_is_hidden() {
    let (ctx, env) = editor();
    let frame = ctx.snapshot(&env, SCREEN.0, SCREEN.1);
    let menus = OpenMenus::bar(ctx.menu_bar(&env), 0, SCREEN, true);
    let mut out = Vec::new();
    render_screen(&mut out, &frame, ColorDepth::TrueColor, 0, Some(&menus)).expect("render");
    let drawn = String::from_utf8(out).expect("UTF-8");
    assert!(drawn.contains("Open"), "the box");
    assert!(drawn.contains("Recent"), "{drawn:?}");
    assert!(drawn.contains('▸'), "a menu inside is marked");
    assert!(
        drawn.contains("\u{1b}[1;2HFile"),
        "the bar, shown while one of its menus is open"
    );
    assert!(!drawn.ends_with("\u{1b}[?25h"), "no cursor over a menu");
}

#[test]
fn a_right_click_opens_the_context_menu_under_the_pointer() {
    let (ctx, env) = editor();
    run(
        r#"(context-menu-add nil "Here" '(insert "here"))"#,
        &env,
        &ctx,
    );
    let _ = ctx.snapshot(&env, SCREEN.0, SCREEN.1 - 1);
    ctx.handle_mouse_event(
        MouseEvent {
            kind: MouseKind::Down(MouseButton::Right),
            column: 5,
            row: 3,
            modifiers: KeyModifiers::default(),
        },
        &env,
    );
    let frame = ctx.snapshot(&env, SCREEN.0, SCREEN.1 - 1);
    let request = frame.menu.clone().expect("a menu");
    assert_eq!(request.kind, MenuKind::Context);
    // A row lower on the screen, below the bar.
    let mut menus = open_requested(request, &frame, 1, SCREEN);
    let outcome = menus.click(7, 5, SCREEN);
    act(outcome, &ctx, &env);
    assert!(text(&ctx, &env).starts_with("here"), "{}", text(&ctx, &env));
}

#[test]
fn a_menu_opened_from_the_keys_opens_at_the_cursor() {
    let (ctx, env) = editor();
    run(
        r#"(context-menu-add nil "Here" '(insert "here"))"#,
        &env,
        &ctx,
    );
    let frame = ctx.snapshot(&env, SCREEN.0, SCREEN.1);
    let request = rsedit_core::MenuRequest {
        kind: MenuKind::Context,
        menu: ctx.context_menu(&env),
        at: None,
    };
    let mut menus = open_requested(request, &frame, 0, SCREEN);
    // Its first entry selected, for the keys.
    let outcome = menus.key(&key(KeyCode::Enter), SCREEN);
    act(outcome, &ctx, &env);
    assert!(text(&ctx, &env).starts_with("here"));
}

#[test]
fn an_empty_menu_has_nothing_to_choose() {
    let mut menus = OpenMenus::context(Menu::default(), (0, 0), SCREEN, true);
    assert_eq!(menus.key(&key(KeyCode::Down), SCREEN), Outcome::Stay);
    assert_eq!(menus.key(&key(KeyCode::Enter), SCREEN), Outcome::Stay);
    let _ = MenuItem::Separator;
}
