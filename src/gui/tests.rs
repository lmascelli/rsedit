//! The window front end: egui's keys, text and IME turned into the
//! editor's keys and text, the shortcuts the platform takes for itself, and
//! the commands the platform's shortcuts run through the core, and the
//! menus.
//!
//! Design: `doc/architettura-editor.typ, <gui>`.
use super::Core;
use super::app::EditorApp;
use super::keys::{character_shortcut, special_key};
use super::paint::Grid;
use eframe::egui::{self, Pos2, Rect, Vec2};
use rsedit_core::{
    ELispExp, FrameSnapshot, GapBuffer, KeyCode, KeyModifiers, Parser, create_global_env, eval,
};

/// Returns an editor with the shipped modules the window relies on loaded,
/// vim among them, kept for the rest of the test run as `EditorApp`
/// borrows it.
fn editor() -> Core<'static, GapBuffer> {
    rsedit_core::isolate_config_for_tests();
    let (state, env) = create_global_env::<GapBuffer>().expect("core");
    let state: &'static _ = Box::leak(Box::new(state));
    let core = Core { state, env };
    core.state.resize(core.env.clone(), 100, 32);
    // Unit tests use the checked-in modules, independently of where Cargo
    // places the test executable or whether build output has been copied.
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("core/lisp");
    core.env.set_variable(
        "lisp-path".into(),
        ELispExp::proper_list(
            ["base", "features", "languages"]
                .iter()
                .map(|kind| ELispExp::string(data.join(kind).to_str().unwrap().into()))
                .collect(),
        ),
    );
    for module in ["common-keymaps", "minibuffer", "clipboard", "vim"] {
        let loaded = core
            .value("require", vec![Core::<GapBuffer>::quote(module)])
            .unwrap_or_else(|why| panic!("load {module}: {why}"));
        assert!(
            !loaded.is_nil(),
            "module {module} not found under {}: {:?}",
            data.display(),
            core.state.get_logs()
        );
    }
    assert!(core.env.get_function("vim-mode").is_some());
    core
}

/// Returns the value of SOURCE, evaluated in the editor.
fn expression(core: &Core<'static, GapBuffer>, source: &str) -> ELispExp<GapBuffer> {
    let ast = Parser::new(source).next().expect("parse");
    eval(&ast, core.env.clone(), core.state).expect("eval")
}

fn wait_for(core: &Core<'static, GapBuffer>, ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !ready() {
        core.state.tick(&core.env);
        assert!(
            std::time::Instant::now() < deadline,
            "file I/O did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Returns the editor's frame, at the size `editor` gives it.
fn frame(core: &Core<'static, GapBuffer>) -> FrameSnapshot {
    core.state.snapshot(&core.env, 100, 32)
}

#[test]
fn meta_symbols_use_the_actual_text_even_without_a_logical_key() {
    for fallback in [None, Some('\\'), Some(','), Some('.')] {
        for symbol in ['<', '>'] {
            for shift in [false, true] {
                let text = symbol.to_string();
                let event = character_shortcut(
                    fallback,
                    KeyModifiers {
                        alt: true,
                        shift,
                        ..Default::default()
                    },
                    Some(&text),
                )
                .unwrap();
                assert_eq!(event.code, KeyCode::Char(symbol));
                assert!(event.modifiers.alt);
                assert!(!event.modifiers.shift);
            }
        }
    }
    let event = character_shortcut(
        Some('2'),
        KeyModifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        },
        Some("\""),
    )
    .unwrap();
    assert_eq!(
        event.code,
        KeyCode::Char('"'),
        "do not substitute US Shift+2 (@) for the reported character"
    );
    assert!(event.modifiers.ctrl);
    assert!(!event.modifiers.shift);
}

#[test]
fn meta_angle_brackets_move_without_inserting_in_plain_and_vim_buffers() {
    for vim in [false, true] {
        let core = editor();
        core.text("one\ntwo\n");
        if vim {
            core.value("global-vim-mode", vec![ELispExp::t()]).unwrap();
        }
        core.value("goto-char", vec![ELispExp::number(0.)]).unwrap();
        core.text_input(
            ">",
            KeyModifiers {
                alt: true,
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(
            expression(&core, "(point)"),
            ELispExp::number(8.),
            "M->, vim={vim}"
        );
        core.text_input(
            "<",
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(
            expression(&core, "(point)"),
            ELispExp::number(0.),
            "M-<, vim={vim}"
        );
        assert_eq!(
            expression(&core, "(buffer-string)"),
            ELispExp::string("one\ntwo\n".into())
        );
    }
}

#[test]
fn altgr_and_non_ascii_composition_stay_literal_text() {
    let core = editor();
    core.text_input(
        "@",
        KeyModifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        },
    );
    core.text_input(
        "é",
        KeyModifiers {
            alt: true,
            ..Default::default()
        },
    );
    core.text_input(
        "ae",
        KeyModifiers {
            alt: true,
            ..Default::default()
        },
    );
    assert_eq!(
        expression(&core, "(buffer-string)"),
        ELispExp::string("@éae".into())
    );
}

#[test]
fn shortcuts_without_text_keep_the_shift_fallback() {
    let event = character_shortcut(
        Some('.'),
        KeyModifiers {
            alt: true,
            shift: true,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(event.code, KeyCode::Char('>'));
    assert!(event.modifiers.alt);
    assert!(!event.modifiers.shift);
    let event = character_shortcut(
        Some('-'),
        KeyModifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(event.code, KeyCode::Char('_'));
    assert!(event.modifiers.ctrl);
    assert!(!event.modifiers.shift);
}

#[test]
fn typing_unicode_and_pasting_share_the_core_buffer() {
    let core = editor();
    core.text("caffè 🦀");
    core.state.handle_paste("\nseconda riga".into(), &core.env);
    assert_eq!(
        expression(&core, "(buffer-string)"),
        ELispExp::string("caffè 🦀\nseconda riga".into())
    );
    assert!(
        frame(&core)
            .focused_view()
            .unwrap()
            .lines
            .iter()
            .any(|line| line.contains("caffè"))
    );
}

#[test]
fn gui_commands_keep_vim_and_undo_dispatch() {
    let core = editor();
    core.value("global-vim-mode", vec![ELispExp::t()]).unwrap();
    core.text("ihello");
    core.key(KeyCode::Esc, KeyModifiers::default());
    assert_eq!(core.vim_state(), "normal");
    core.command("undo");
    assert_eq!(
        expression(&core, "(buffer-string)"),
        ELispExp::string("".into())
    );
    assert!(!core.prompt_open());
    assert!(
        !core
            .state
            .get_logs()
            .iter()
            .any(|line| line.contains("Vim: "))
    );
}

#[test]
fn a_prompt_blocks_the_commands_of_the_platforms_shortcuts() {
    let core = editor();
    core.command("find-file");
    assert!(core.prompt_open());
    let before = expression(&core, "(current-buffer)");
    core.command("undo");
    assert_eq!(expression(&core, "(current-buffer)"), before);
    core.key(KeyCode::Esc, KeyModifiers::default());
    assert!(!core.prompt_open());
}

#[test]
fn file_open_is_data_not_an_interpolated_lisp_program() {
    let core = editor();
    let name = if cfg!(target_os = "windows") {
        format!("rsedit-gui-{}-' space.txt", std::process::id())
    } else {
        format!("rsedit-gui-{}-quote\".txt", std::process::id())
    };
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, "file text").unwrap();
    core.open(&path).unwrap();
    wait_for(&core, || {
        core.value("buffer-file-name", vec![]).unwrap()
            == ELispExp::string(path.display().to_string())
    });
    assert_eq!(
        expression(&core, "(buffer-string)"),
        ELispExp::string("file text".into())
    );
    core.text("x");
    core.command("save-buffer");
    wait_for(&core, || {
        core.value("buffer-modified-p", vec![]).unwrap().is_nil()
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "xfile text");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn window_close_uses_core_save_confirmation() {
    let core = editor();
    let path = std::env::temp_dir().join(format!("rsedit-gui-{}-unsaved.txt", std::process::id()));
    core.open(&path).unwrap();
    wait_for(&core, || {
        core.value("buffer-file-name", vec![]).unwrap()
            == ELispExp::string(path.display().to_string())
    });
    core.text("unsaved");
    core.request_quit();
    assert!(core.state.is_running());
    assert!(core.prompt_open() || !frame(&core).prompt.is_empty());
}

/// Returns a window on a fresh editor, without a native window.
fn app() -> EditorApp<'static, GapBuffer> {
    EditorApp {
        core: editor(),
        font_size: 16.,
        size: (100, 32),
        preedit: String::new(),
        dragging: false,
        wheel: 0.,
        pending: false,
        clipboard_v_seen: false,
        last_input_frame: None,
        bar_labels: Vec::new(),
        bar_menu: None,
        bar_rects: Vec::new(),
        popup: None,
        popups: 0,
        right_press: None,
    }
}

/// Passes EVENTS, with MODIFIERS held, to the window as one frame of egui.
fn input(
    app: &mut EditorApp<'static, GapBuffer>,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) {
    let ctx = egui::Context::default();
    let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 640.));
    let grid = Grid::new(area, Vec2::new(8., 20.));
    let _ = ctx.run(
        egui::RawInput {
            events,
            modifiers,
            screen_rect: Some(area),
            ..Default::default()
        },
        |ctx| app.input(ctx, area, grid),
    );
}

#[test]
fn meta_angle_bracket_text_without_a_logical_key_reaches_the_core() {
    let mut app = app();
    app.core.text("one\ntwo\n");
    app.core
        .value("goto-char", vec![ELispExp::number(0.)])
        .unwrap();
    let meta = egui::Modifiers {
        alt: true,
        shift: true,
        ..Default::default()
    };
    input(&mut app, vec![egui::Event::Text(">".into())], meta);
    assert_eq!(
        app.core.value("point", vec![]).unwrap(),
        ELispExp::number(8.)
    );
    assert_eq!(
        app.core.value("buffer-string", vec![]).unwrap(),
        ELispExp::string("one\ntwo\n".into())
    );
}

#[test]
fn paired_meta_symbol_prefers_actual_text_and_dispatches_only_once() {
    let mut app = app();
    app.core.text("abc");
    app.core
        .value("goto-char", vec![ELispExp::number(0.)])
        .unwrap();
    app.core
        .value(
            "define-key",
            vec![
                ELispExp::nil(),
                ELispExp::string("M->".into()),
                Core::<GapBuffer>::quote("forward-char"),
            ],
        )
        .unwrap();
    let meta = egui::Modifiers {
        alt: true,
        shift: true,
        ..Default::default()
    };
    input(
        &mut app,
        vec![
            egui::Event::Key {
                key: egui::Key::Backslash,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: meta,
            },
            egui::Event::Text(">".into()),
        ],
        meta,
    );
    assert_eq!(
        app.core.value("point", vec![]).unwrap(),
        ELispExp::number(1.)
    );
    assert_eq!(
        app.core.value("buffer-string", vec![]).unwrap(),
        ELispExp::string("abc".into())
    );
}

#[test]

fn a_text_key_event_is_not_inserted_twice() {
    let mut app = app();
    input(
        &mut app,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::Text("a".into()),
            egui::Event::Text("è".into()),
        ],
        egui::Modifiers::default(),
    );
    assert_eq!(
        app.core.value("buffer-string", vec![]).unwrap(),
        ELispExp::string("aè".into())
    );
}

#[test]
#[cfg(not(target_os = "macos"))]
fn intercepted_ctrl_x_still_opens_a_file_prompt_and_preserves_its_first_letter() {
    let mut app = app();
    let control = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    // C-x C-f and then f, in one frame, as egui-winit sends them: Ctrl-X as
    // a Cut, and every press as a Key, with a Text after it only when Ctrl is
    // not held.
    input(
        &mut app,
        vec![
            egui::Event::Cut,
            egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: control,
            },
            egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::Text("f".into()),
        ],
        control,
    );
    assert!(app.core.prompt_open());
    assert_eq!(
        app.core.value("buffer-string", vec![]).unwrap(),
        ELispExp::string("f".into())
    );
}

#[test]
fn intercepted_ctrl_v_selects_a_vim_block_instead_of_inserting_clipboard_text() {
    let mut app = app();
    app.core
        .value("global-vim-mode", vec![ELispExp::t()])
        .unwrap();
    app.core.text("iabc");
    app.core.key(KeyCode::Esc, KeyModifiers::default());
    let control = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    input(
        &mut app,
        vec![egui::Event::Paste("clipboard".into())],
        control,
    );
    assert_eq!(app.core.vim_state(), "visual-block");
    assert_eq!(
        app.core.value("buffer-string", vec![]).unwrap(),
        ELispExp::string("abc".into())
    );
}

#[test]
fn ime_preedit_is_temporary_and_commit_is_inserted_once() {
    let mut app = app();
    input(
        &mut app,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit("è".into()))],
        egui::Modifiers::default(),
    );
    assert_eq!(
        app.core.value("buffer-string", vec![]).unwrap(),
        ELispExp::string("".into())
    );
    input(
        &mut app,
        vec![egui::Event::Ime(egui::ImeEvent::Commit("è".into()))],
        egui::Modifiers::default(),
    );
    assert_eq!(
        app.core.value("buffer-string", vec![]).unwrap(),
        ELispExp::string("è".into())
    );
    assert!(app.preedit.is_empty());
}

/// Delete, Insert, Home, End, the page keys and the function keys reach the
/// core by their names, as from the terminal, and what they do is the
/// keymaps': the window translates none of them into other keys.
#[test]
fn the_editing_and_function_keys_reach_the_core_by_name() {
    for (from, to) in [
        (egui::Key::Delete, KeyCode::Delete),
        (egui::Key::Insert, KeyCode::Insert),
        (egui::Key::Home, KeyCode::Home),
        (egui::Key::End, KeyCode::End),
        (egui::Key::PageUp, KeyCode::PageUp),
        (egui::Key::PageDown, KeyCode::PageDown),
        (egui::Key::F1, KeyCode::F(1)),
        (egui::Key::F12, KeyCode::F(12)),
    ] {
        assert_eq!(special_key(from), Some(to), "{from:?}");
    }
}

#[test]
fn a_right_click_in_the_grid_asks_the_editor_for_its_context_menu() {
    let mut app = app();
    expression(
        &app.core,
        r#"(progn (setq mouse-mode t) (context-menu-add nil "Here" 'undo))"#,
    );
    let _ = frame(&app.core);
    let button = |pressed| egui::Event::PointerButton {
        pos: Pos2::new(20., 30.),
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    input(&mut app, vec![button(true)], egui::Modifiers::NONE);
    assert!(frame(&app.core).menu.is_none(), "nothing yet, on the press");
    input(&mut app, vec![button(false)], egui::Modifiers::NONE);
    let request = frame(&app.core).menu.expect("a menu to open");
    assert_eq!(request.kind, rsedit_core::MenuKind::Context);
    assert_eq!(request.at, Some((2, 1)), "the cell under the pointer");
}

#[test]
fn a_menu_of_the_bar_the_keys_open_shows_under_its_label() {
    let mut app = app();
    expression(
        &app.core,
        r#"(progn (menu-bar-add '("File" "Open") 'find-file)
                  (menu-bar-add '("Help" "Entry") 'describe-key))"#,
    );
    let label = Rect::from_min_size(Pos2::new(40., 0.), Vec2::new(30., 18.));
    app.bar_rects = vec![Rect::NOTHING, label];
    let request = rsedit_core::MenuRequest {
        kind: rsedit_core::MenuKind::Bar { open: 1 },
        menu: app.core.state.menu_bar(&app.core.env),
        at: None,
    };
    let area = Rect::from_min_size(Pos2::new(0., 20.), Vec2::new(800., 640.));
    app.open_requested(request, Grid::new(area, Vec2::new(8., 20.)), area, None);
    let popup = app.popup.as_ref().expect("a popup");
    assert_eq!(popup.beside, label);
    assert_eq!(popup.menu.labels(), ["Entry"], "Help's own entries");
}

#[test]
fn while_a_menu_is_open_nothing_typed_or_clicked_reaches_the_buffer() {
    let mut app = app();
    expression(&app.core, r#"(context-menu-add nil "Here" 'undo)"#);
    let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 640.));
    let request = rsedit_core::MenuRequest {
        kind: rsedit_core::MenuKind::Context,
        menu: app.core.state.context_menu(&app.core.env),
        at: Some((3, 3)),
    };
    app.open_requested(request, Grid::new(area, Vec2::new(8., 20.)), area, None);
    let before = expression(&app.core, "(buffer-string)");
    input(
        &mut app,
        vec![
            egui::Event::Text("x".into()),
            egui::Event::PointerButton {
                pos: Pos2::new(200., 200.),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        egui::Modifiers::NONE,
    );
    assert_eq!(expression(&app.core, "(buffer-string)"), before);
    assert_eq!(expression(&app.core, "(point)"), ELispExp::number(0.));
}

#[test]
fn a_popup_shows_its_entries_and_chooses_nothing_unclicked() {
    let mut app = app();
    expression(
        &app.core,
        r#"(context-menu-add nil '("More" "Inside") 'undo)"#,
    );
    let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 640.));
    let request = rsedit_core::MenuRequest {
        kind: rsedit_core::MenuKind::Context,
        menu: app.core.state.context_menu(&app.core.env),
        at: Some((3, 3)),
    };
    app.open_requested(request, Grid::new(area, Vec2::new(8., 20.)), area, None);
    let ctx = egui::Context::default();
    let mut chosen = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(area),
            ..Default::default()
        },
        |ctx| chosen = app.popup.as_mut().expect("open").show(ctx),
    );
    assert_eq!(chosen, None);
    assert!(app.popup.as_ref().is_some_and(|popup| popup.open));
}

/// Runs one frame of egui with EVENTS, the menu bar of APP drawn at the top.
fn bar_frame(
    app: &mut EditorApp<'static, GapBuffer>,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> Option<rsedit_core::MenuItemId> {
    let mut chosen = None;
    let _ = ctx.run(
        egui::RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 640.))),
            ..Default::default()
        },
        |ctx| {
            egui::TopBottomPanel::top("bar").show(ctx, |ui| chosen = app.menu_bar(ui, true).0);
        },
    );
    chosen
}

#[test]
fn a_click_on_a_label_of_the_bar_opens_its_menu_resolved_then() {
    let mut app = app();
    expression(
        &app.core,
        r#"(progn (menu-bar-add '("File" "Open") 'find-file)
                  (menu-bar-add '("Help" "Keys") 'describe-key))"#,
    );
    app.bar_labels = vec!["File".into(), "Help".into()];
    let ctx = egui::Context::default();
    bar_frame(&mut app, &ctx, Vec::new());
    assert_eq!(app.bar_rects.len(), 2, "each label drawn");
    assert!(app.bar_menu.is_none(), "nothing resolved while closed");

    let at = app.bar_rects[0].center();
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    bar_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(at), press(true)],
    );
    bar_frame(&mut app, &ctx, vec![press(false)]);
    bar_frame(&mut app, &ctx, Vec::new());
    let bar = app.bar_menu.as_ref().expect("resolved as the menu opened");
    assert_eq!(bar.labels(), ["File", "Help"]);
}

/// Runs one frame of APP in CTX with EVENTS, as `update` does for the menus:
/// the input, the frame's menus taken, the popup shown.
fn menu_frame(
    app: &mut EditorApp<'static, GapBuffer>,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) {
    let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 640.));
    let grid = Grid::new(area, Vec2::new(8., 20.));
    let _ = ctx.run(
        egui::RawInput {
            events,
            screen_rect: Some(area),
            ..Default::default()
        },
        |ctx| {
            app.input(ctx, area, grid);
            let snapshot = app
                .core
                .state
                .snapshot(&app.core.env, grid.columns, grid.rows);
            app.take_menus(&snapshot, grid, area);
            app.show_popup(ctx);
        },
    );
}

#[test]
fn a_context_menu_stays_open_once_the_right_button_is_released() {
    let mut app = app();
    expression(
        &app.core,
        r#"(progn (setq mouse-mode t) (context-menu-add nil "Here" 'undo))"#,
    );
    let ctx = egui::Context::default();
    let at = Pos2::new(20., 30.);
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    menu_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(at), button(true)],
    );
    menu_frame(&mut app, &ctx, vec![button(false)]);
    menu_frame(&mut app, &ctx, Vec::new());
    assert!(
        app.popup.as_ref().is_some_and(|popup| popup.open),
        "the release of the click that opened it must not close it"
    );
    // A click elsewhere closes it, and reaches nothing under it.
    let elsewhere = Pos2::new(600., 500.);
    let primary = |pressed| egui::Event::PointerButton {
        pos: elsewhere,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    menu_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(elsewhere), primary(true)],
    );
    menu_frame(&mut app, &ctx, vec![primary(false)]);
    assert!(app.popup.is_none(), "closed by the click elsewhere");
    assert_eq!(expression(&app.core, "(point)"), ELispExp::number(0.));
}

#[test]
fn ctrl_and_the_wheel_size_the_text_rather_than_scroll() {
    let mut app = app();
    expression(&app.core, "(setq mouse-mode t)");
    let before = app.font_size;
    let wheel = |y, ctrl| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0., y),
        modifiers: egui::Modifiers {
            ctrl,
            ..egui::Modifiers::NONE
        },
    };
    let ctx = egui::Context::default();
    let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 640.));
    let grid = Grid::new(area, Vec2::new(8., 20.));
    let _ = ctx.run(
        egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(Pos2::new(50., 50.)),
                wheel(2., true),
            ],
            screen_rect: Some(area),
            ..Default::default()
        },
        |ctx| app.input(ctx, area, grid),
    );
    assert_eq!(app.font_size, before + 2.);
    app.zoom(-100.);
    assert_eq!(app.font_size, 8., "never too small to read");
}
