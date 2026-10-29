//! The window front end: egui's keys, text and IME turned into the
//! editor's keys and text, the shortcuts the platform takes for itself, and
//! the toolbar's commands run through the core.
//!
//! Design: `doc/architettura-editor.typ, <gui>`.
use super::Core;
use super::app::EditorApp;
use super::keys::character_shortcut;
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
    let exe = std::env::current_exe().unwrap();
    let data = exe.parent().unwrap().parent().unwrap().join("data/lisp");
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
        core.value("require", vec![Core::<GapBuffer>::quote(module)])
            .expect("load module");
    }
    assert!(core.env.get_function("vim-mode").is_some());
    core.enable_unicode_typing().unwrap();
    core
}

/// Returns the value of SOURCE, evaluated in the editor.
fn expression(core: &Core<'static, GapBuffer>, source: &str) -> ELispExp<GapBuffer> {
    let ast = Parser::new(source).next().expect("parse");
    eval(&ast, core.env.clone(), core.state).expect("eval")
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
fn command_prompt_blocks_unrelated_toolbar_commands() {
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
    assert_eq!(
        expression(&core, "(buffer-string)"),
        ELispExp::string("file text".into())
    );
    core.text("x");
    core.command("save-buffer");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "xfile text");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn window_close_uses_core_save_confirmation() {
    let core = editor();
    let path = std::env::temp_dir().join(format!("rsedit-gui-{}-unsaved.txt", std::process::id()));
    core.open(&path).unwrap();
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
