//! The window's application: egui's events turned into the editor's keys,
//! clicks and pastes, and a frame drawn at every update, with the editor
//! saying how soon the next one is due.
//!
//! Design: `doc/architettura-editor.typ, <gui>`.
use super::Core;
use super::keys::{character, character_shortcut, mods, special_key};
use super::paint::{Grid, colors, paint_view, row};
use eframe::egui::{self, FontId, Rect, Stroke, Vec2};
use rsedit_core::{BufferTrait, Face, KeyCode, KeyModifiers, MouseButton, MouseKind};
use std::{path::PathBuf, time::Duration};

/// The longest the window goes without an update, when nothing asks for one
/// sooner.
const BACKGROUND_POLL: Duration = Duration::from_millis(250);

/// The editor in a window: what the window remembers between two updates.
pub(super) struct EditorApp<'editor, B: BufferTrait> {
    pub(super) core: Core<'editor, B>,
    /// The size of the monospace font, which the − and + buttons change.
    pub(super) font_size: f32,
    /// The frame's size in cells, as the editor was last told.
    pub(super) size: (usize, usize),
    /// What an input method is composing, drawn at the cursor until it is
    /// committed.
    pub(super) preedit: String,
    /// Whether the left button went down in the editor and is still down.
    pub(super) dragging: bool,
    /// The wheel's motion, in lines, not yet turned into a scroll.
    pub(super) wheel: f32,
    /// Whether the editor is part-way through a key sequence, which disables
    /// the toolbar.
    pub(super) pending: bool,
    /// Whether this press of Ctrl-V arrived as a paste: if not, the release
    /// stands for it.
    pub(super) clipboard_v_seen: bool,
    /// The frame whose input was read, so that an update is not read twice.
    pub(super) last_input_frame: Option<u64>,
}

impl<'editor, B: BufferTrait> EditorApp<'editor, B> {
    /// Returns the application for CORE, its window dark and drawn with FONT
    /// when one is given.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        core: Core<'editor, B>,
        font: Option<PathBuf>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        if let Some(path) = font {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "user-mono".into(),
                egui::FontData::from_owned(std::fs::read(path)?).into(),
            );
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "user-mono".into());
            cc.egui_ctx.set_fonts(fonts);
        }
        Ok(Self {
            core,
            font_size: 16.,
            size: (0, 0),
            preedit: String::new(),
            dragging: false,
            wheel: 0.,
            pending: false,
            clipboard_v_seen: false,
            last_input_frame: None,
        })
    }

    /// Presses what KEY, which the editor has no name for, stands for: Home
    /// and End, Delete, Page Up and Page Down, as Emacs or, in a Vim state
    /// other than insert, as Vim binds them. Any other key does nothing; its
    /// text arrives as a text event.
    fn native_key(&self, key: egui::Key, modifiers: egui::Modifiers) {
        use egui::Key;
        let state = self.core.vim_state();
        let normal = !state.is_empty() && !matches!(state.as_str(), "insert" | "replace");
        match key {
            Key::Home | Key::End if modifiers.ctrl => self.core.key(
                KeyCode::Char(if key == Key::Home { '<' } else { '>' }),
                KeyModifiers {
                    alt: true,
                    ..Default::default()
                },
            ),
            Key::Home if normal => self.core.text("0"),
            Key::End if normal => self.core.text("$"),
            Key::Home => self.core.control('a'),
            Key::End => self.core.control('e'),
            Key::Delete if normal => self.core.text("x"),
            Key::Delete => self.core.control('d'),
            Key::PageUp if normal => self.core.control('b'),
            Key::PageDown if normal => self.core.control('f'),
            Key::PageUp => self.core.key(
                KeyCode::Char('v'),
                KeyModifiers {
                    alt: true,
                    ..Default::default()
                },
            ),
            Key::PageDown => self.core.control('v'),
            _ => (),
        }
    }

    /// Hands the editor this update's events from CTX: keys and text, the
    /// clipboard keys egui takes for itself, the input method, the mouse in
    /// AREA, laid out as GRID, and the files dropped on the window.
    pub(super) fn input(&mut self, ctx: &egui::Context, area: Rect, grid: Grid) {
        let (events, modifiers, pointer) =
            ctx.input(|i| (i.events.clone(), i.modifiers, i.pointer.hover_pos()));
        let mut suppress_text: Option<char> = None;
        let mut text_modifiers = modifiers;
        let mut clipboard_keys = Vec::new();
        for event in &events {
            match event {
                egui::Event::Copy => clipboard_keys.push(egui::Key::C),
                egui::Event::Cut => clipboard_keys.push(egui::Key::X),
                egui::Event::Paste(_) => clipboard_keys.push(egui::Key::V),
                _ => (),
            }
        }
        for (at, event) in events.iter().enumerate() {
            if !self.core.state.is_running() {
                break;
            }
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    suppress_text = None;
                    text_modifiers = *modifiers;
                    if clipboard_keys.contains(key) && modifiers.command {
                        continue;
                    }
                    if modifiers.mac_cmd {
                        match key {
                            egui::Key::O => self.core.command("find-file"),
                            egui::Key::S if modifiers.shift => self.core.command("write-file"),
                            egui::Key::S => self.core.command("save-buffer"),
                            egui::Key::Q => self.core.request_quit(),
                            egui::Key::A => self.core.command("mark-whole-buffer"),
                            egui::Key::Z if modifiers.shift => self.core.command("redo"),
                            egui::Key::Z => self.core.command("undo"),
                            _ => (),
                        }
                    } else if let Some(code) = special_key(*key) {
                        self.core.key(code, mods(*modifiers));
                    } else if matches!(
                        key,
                        egui::Key::Home
                            | egui::Key::End
                            | egui::Key::Delete
                            | egui::Key::PageUp
                            | egui::Key::PageDown
                    ) {
                        self.native_key(*key, *modifiers);
                    } else if modifiers.ctrl || modifiers.alt {
                        let text = events[at + 1..]
                            .iter()
                            .take_while(|event| !matches!(event, egui::Event::Key { .. }))
                            .find_map(|event| match event {
                                egui::Event::Text(text) => Some(text.as_str()),
                                _ => None,
                            });
                        if let Some(event) =
                            character_shortcut(character(*key), mods(*modifiers), text)
                        {
                            self.core.state.handle_key_event(event, &self.core.env);
                            suppress_text = text.and_then(|text| text.chars().next());
                        }
                    } else {
                        self.native_key(*key, *modifiers);
                    }
                }
                egui::Event::Text(text) => {
                    let duplicated = suppress_text.take().is_some_and(|c| {
                        text.chars().count() == 1
                            && text
                                .chars()
                                .next()
                                .is_some_and(|t| t.eq_ignore_ascii_case(&c))
                    });
                    if !duplicated && self.preedit.is_empty() && !text_modifiers.mac_cmd {
                        self.core.text_input(text, mods(text_modifiers));
                    }
                    text_modifiers = modifiers;
                }
                egui::Event::Copy | egui::Event::Cut => {
                    if cfg!(target_os = "macos") || modifiers.mac_cmd || modifiers.shift {
                        self.core.command(if matches!(event, egui::Event::Copy) {
                            "kill-ring-save"
                        } else {
                            "kill-region"
                        });
                    } else {
                        // egui takes Ctrl-C and Ctrl-X: they are the editor's
                        // C-c and C-x again.
                        self.core.control(if matches!(event, egui::Event::Copy) {
                            'c'
                        } else {
                            'x'
                        });
                    }
                }
                egui::Event::Paste(text) => {
                    self.clipboard_v_seen = true;
                    if modifiers.ctrl && !modifiers.mac_cmd && !modifiers.shift {
                        self.core.control('v');
                    } else {
                        self.core.state.handle_paste(text.clone(), &self.core.env);
                    }
                }
                egui::Event::Key {
                    key: egui::Key::V,
                    pressed: false,
                    modifiers,
                    ..
                } => {
                    // With the clipboard empty or out of reach, egui-winit
                    // sends no press: the release stands for C-v.
                    if !self.clipboard_v_seen
                        && modifiers.ctrl
                        && !modifiers.mac_cmd
                        && !modifiers.shift
                    {
                        self.core.control('v');
                    }
                    self.clipboard_v_seen = false;
                }
                egui::Event::Ime(egui::ImeEvent::Preedit(text)) => self.preedit.clone_from(text),
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    self.preedit.clear();
                    let state = self.core.vim_state();
                    if !state.is_empty() && state != "insert" {
                        self.core.text(text);
                    } else {
                        self.core.state.handle_paste(text.clone(), &self.core.env);
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Disabled) => self.preedit.clear(),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                } => {
                    if *pressed && area.contains(*pos) {
                        self.dragging = true;
                        self.core.state.handle_mouse_event(
                            grid.mouse(*pos, MouseKind::Down(MouseButton::Left), *modifiers),
                            &self.core.env,
                        );
                    } else if !pressed && self.dragging {
                        self.dragging = false;
                        self.core.state.handle_mouse_event(
                            grid.mouse(*pos, MouseKind::Up(MouseButton::Left), *modifiers),
                            &self.core.env,
                        );
                    }
                }
                egui::Event::PointerMoved(pos) if self.dragging => {
                    self.core.state.handle_mouse_event(
                        grid.mouse(*pos, MouseKind::Drag(MouseButton::Left), modifiers),
                        &self.core.env,
                    );
                }
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                } if pointer.is_some_and(|pos| area.contains(pos)) => {
                    let lines = match unit {
                        egui::MouseWheelUnit::Point => delta.y / grid.cell.y,
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * grid.rows as f32,
                    };
                    self.wheel = (self.wheel + lines).clamp(-32., 32.);
                    let steps = (self.wheel.trunc() as i32).clamp(-32, 32);
                    self.wheel -= steps as f32;
                    let kind = if steps > 0 {
                        MouseKind::ScrollUp
                    } else {
                        MouseKind::ScrollDown
                    };
                    for _ in 0..steps.unsigned_abs() {
                        self.core.state.handle_mouse_event(
                            grid.mouse(pointer.unwrap_or(area.min), kind, *modifiers),
                            &self.core.env,
                        );
                    }
                }
                egui::Event::WindowFocused(false) => {
                    self.preedit.clear();
                    self.clipboard_v_seen = false;
                    if self.dragging {
                        self.dragging = false;
                        self.core.state.handle_mouse_event(
                            grid.mouse(area.min, MouseKind::Up(MouseButton::Left), modifiers),
                            &self.core.env,
                        );
                    }
                }
                _ => (),
            }
        }
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        for file in files {
            if let Some(path) = file.path {
                if let Err(error) = self.core.open(&path) {
                    self.core.state.set_echo_message(&error);
                }
            }
        }
    }
}

impl<'editor, B: BufferTrait> eframe::App for EditorApp<'editor, B> {
    /// Reads the events, once per frame, draws the toolbar and the frame the
    /// editor composes, and asks for the next update when the editor says.
    /// Closing the window asks the editor to quit, as `C-x C-c` does.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let frame_number = ctx.cumulative_frame_nr();
        let accept_input = self.last_input_frame != Some(frame_number);
        if accept_input
            && ctx.input(|i| i.viewport().close_requested())
            && self.core.state.is_running()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.core.request_quit();
        }
        if accept_input {
            self.core.state.tick(&self.core.env);
        }
        if !self.core.state.is_running() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let mut action = None;
        egui::TopBottomPanel::top("commands").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let enabled = !self.core.prompt_open() && !self.pending;
                ui.add_enabled_ui(enabled, |ui| {
                    for (title, command) in [
                        ("Open", "find-file"),
                        ("Save", "save-buffer"),
                        ("Save as", "write-file"),
                        ("Copy", "kill-ring-save"),
                        ("Cut", "kill-region"),
                        ("Undo", "undo"),
                        ("Vim", "vim-mode"),
                    ] {
                        let available = self.core.env.get_function(command).is_some();
                        if ui
                            .add_enabled(available, egui::Button::new(title))
                            .clicked()
                            && accept_input
                        {
                            action = Some(command);
                        }
                    }
                });
                ui.separator();
                if ui.small_button("−").clicked() && accept_input {
                    self.font_size = (self.font_size - 1.).max(8.);
                }
                if ui.small_button("+").clicked() && accept_input {
                    self.font_size = (self.font_size + 1.).min(40.);
                }
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let area = ui.available_rect_before_wrap();
                let (_, response) =
                    ui.allocate_exact_size(area.size(), egui::Sense::click_and_drag());
                // The keyboard is the editor's: its prompts are its buffers.
                response.request_focus();
                let font = FontId::monospace(self.font_size);
                let cell = ctx.fonts_mut(|fonts| {
                    Vec2::new(
                        fonts.glyph_width(&font, 'M').ceil().max(1.),
                        fonts.row_height(&font).ceil() + 2.,
                    )
                });
                let grid = Grid::new(area, cell);
                if self.size != (grid.columns, grid.rows) {
                    self.size = (grid.columns, grid.rows);
                    self.core
                        .state
                        .resize(self.core.env.clone(), grid.columns, grid.rows);
                }
                if let Some(command) = action {
                    self.core.command(command);
                }
                if accept_input {
                    self.input(ctx, area, grid);
                    self.last_input_frame = Some(frame_number);
                }
                let snapshot = self
                    .core
                    .state
                    .snapshot(&self.core.env, grid.columns, grid.rows);
                if let Some(text) = &snapshot.clipboard {
                    ctx.copy_text(text.clone());
                }
                self.pending = !snapshot.pending_input.is_empty();
                let default = snapshot.theme.style(Face::DEFAULT);
                let (_, background) = colors(default, default);
                let painter = ui.painter().with_clip_rect(area);
                painter.rect_filled(area, 0., background);
                for separator in &snapshot.separators {
                    let style = snapshot.theme.style(separator.face);
                    let rect = separator.rect;
                    let (_, bg) = colors(style, default);
                    let (fg, _) = colors(style, default);
                    let area = grid.rect(rect.x, rect.y, rect.width, rect.height);
                    painter.rect_filled(area, 0., bg);
                    let ends = if rect.width > rect.height {
                        [area.left_center(), area.right_center()]
                    } else {
                        [area.center_top(), area.center_bottom()]
                    };
                    painter.line_segment(ends, Stroke::new(1_f32, fg));
                }
                let state = self.core.vim_state();
                let block = !state.is_empty() && !matches!(state.as_str(), "insert" | "replace");
                for view in &snapshot.views {
                    if let Some(caret) =
                        paint_view(&painter, grid, view, &snapshot, &font, block, &self.preedit)
                    {
                        ctx.output_mut(|output| {
                            output.ime = Some(egui::output::IMEOutput {
                                rect: area,
                                cursor_rect: caret,
                            })
                        });
                    }
                }
                let echo = if !snapshot.prompt.is_empty() {
                    &snapshot.prompt
                } else {
                    &snapshot.echo_message
                };
                let mut text = echo.to_owned();
                if !snapshot.pending_input.is_empty() {
                    if !text.is_empty() {
                        text.push_str("   ");
                    }
                    text.push_str(&snapshot.pending_input);
                }
                row(
                    &painter,
                    grid,
                    0,
                    grid.rows as isize - 1,
                    grid.columns,
                    &text,
                    &[],
                    &font,
                    default,
                );
                // TODO(GUI-1): the background work's changes are not asked to
                // wake the window (`on_background_change`), which updates every
                // `BACKGROUND_POLL` even when nothing changes.
                let wake = [
                    Some(BACKGROUND_POLL),
                    self.core.state.next_redraw_in(&self.core.env, &snapshot),
                    self.core.state.drag_scroll_in(),
                ]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(BACKGROUND_POLL);
                ctx.request_repaint_after(wake);
            });
    }
}
