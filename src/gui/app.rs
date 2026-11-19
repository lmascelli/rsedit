//! The window's application: egui's events turned into the editor's keys,
//! clicks and pastes, and a frame drawn at every update, with the editor
//! saying how soon the next one is due.
//!
//! Design: `doc/architettura-editor.typ, <gui>`.
use super::Core;
use super::keys::{character, character_shortcut, mods, special_key};
use super::menus::{PopupMenu, entries};
use super::paint::{Grid, colors, paint_view, row};
use eframe::egui::{self, FontId, Rect, Stroke, Vec2};
use rsedit_core::{
    BufferTrait, Face, FrameSnapshot, Menu, MenuItem, MenuItemId, MenuKind, MenuRequest,
    MouseButton, MouseKind,
};
use std::path::PathBuf;

/// The editor in a window: what the window remembers between two updates.
pub(super) struct EditorApp<'editor, B: BufferTrait> {
    pub(super) core: Core<'editor, B>,
    /// The size of the monospace font, which the − and + buttons of the menu
    /// bar change, and Ctrl with the wheel.
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
    /// the menu bar.
    pub(super) pending: bool,
    /// Whether this press of Ctrl-V arrived as a paste: if not, the release
    /// stands for it.
    pub(super) clipboard_v_seen: bool,
    /// The frame whose input was read, so that an update is not read twice.
    pub(super) last_input_frame: Option<u64>,
    /// The labels of the menu bar, as the last frame named them: empty while
    /// `menu-bar-mode` is off.
    pub(super) bar_labels: Vec<String>,
    /// The menu bar resolved when one of its menus opened, kept while one is
    /// open: what the editor answers holds for as long as the menu is seen.
    pub(super) bar_menu: Option<Menu>,
    /// Where each label of the bar was drawn, for a menu of the bar the keys
    /// open.
    pub(super) bar_rects: Vec<Rect>,
    /// A menu the editor asked for, open as a popup.
    pub(super) popup: Option<PopupMenu>,
    /// How many popups have been opened, which names the next.
    pub(super) popups: u64,
    /// Where the right button went down in the grid, while it is down: the
    /// context menu opens there on its release.
    pub(super) right_press: Option<egui::Pos2>,
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
        // What a job changes wakes the window, as it wakes the terminal's loop:
        // the update that follows asks the editor what is due. Without it the
        // window polled four times a second, still or not.
        let window = cc.egui_ctx.clone();
        core.state
            .on_background_change(move || window.request_repaint());
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
            bar_labels: Vec::new(),
            bar_menu: None,
            bar_rects: Vec::new(),
            popup: None,
            popups: 0,
            right_press: None,
        })
    }

    /// Opens the menu REQUEST asks for as a popup: a context menu at its
    /// cell of GRID, or at point when it names none; a menu of the bar under
    /// its label, or at the top of AREA while the bar is hidden.
    pub(super) fn open_requested(
        &mut self,
        request: MenuRequest,
        grid: Grid,
        area: Rect,
        cursor: Option<Rect>,
    ) {
        let (menu, beside) = match request.kind {
            MenuKind::Context => {
                let beside = match request.at {
                    Some((x, y)) => grid.rect(x as isize, y as isize, 1, 1),
                    None => cursor.unwrap_or(Rect::from_min_size(area.min, grid.cell)),
                };
                (request.menu, beside)
            }
            MenuKind::Bar { open } => {
                let beside = self
                    .bar_rects
                    .get(open)
                    .copied()
                    .unwrap_or(Rect::from_min_size(area.min, Vec2::ZERO));
                match request.menu.items.into_iter().nth(open) {
                    Some(MenuItem::Submenu { menu, .. }) => (menu, beside),
                    Some(action @ MenuItem::Action { .. }) => (
                        Menu {
                            items: vec![action],
                        },
                        beside,
                    ),
                    _ => return,
                }
            }
        };
        self.popups += 1;
        self.popup = Some(PopupMenu {
            menu,
            beside,
            serial: self.popups,
            open: true,
        });
    }

    /// Takes from SNAPSHOT, a frame of GRID in AREA, what it says of the
    /// menus: the labels of the bar, and a menu to open.
    pub(super) fn take_menus(&mut self, snapshot: &FrameSnapshot, grid: Grid, area: Rect) {
        self.bar_labels.clone_from(&snapshot.menu_bar);
        if let Some(request) = snapshot.menu.clone() {
            let cursor = snapshot.focused_view().and_then(|view| {
                let (x, y) = view.cursor_rel_pos?;
                Some(grid.rect(view.rect.x + x as isize, view.rect.y + y as isize, 1, 1))
            });
            self.open_requested(request, grid, area, cursor);
        }
    }

    /// Shows the popup in CTX, while it is open, and runs what is chosen in
    /// it.
    pub(super) fn show_popup(&mut self, ctx: &egui::Context) {
        let Some(popup) = self.popup.as_mut() else {
            return;
        };
        let chosen = popup.show(ctx);
        if !popup.open {
            self.popup = None;
        }
        if let Some(id) = chosen {
            self.core.state.run_menu_item(id, &self.core.env);
        }
    }

    /// Makes the text STEPS sizes larger, or smaller for a negative STEPS,
    /// within what can be read.
    pub(super) fn zoom(&mut self, steps: f32) {
        self.font_size = (self.font_size + steps).clamp(8., 40.);
    }

    /// Draws the menu bar in UI, its menus resolved when one opens and
    /// greyed out unless ENABLED, with the − and + buttons at its right end;
    /// returns the action chosen in it, if any, and by how many sizes the
    /// buttons asked for the text to change.
    pub(super) fn menu_bar(
        &mut self,
        ui: &mut egui::Ui,
        enabled: bool,
    ) -> (Option<MenuItemId>, f32) {
        let mut chosen = None;
        let mut zoom = 0.;
        let mut any_open = false;
        let mut rects = Vec::with_capacity(self.bar_labels.len());
        let core = &self.core;
        let cache = &mut self.bar_menu;
        let labels = &self.bar_labels;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                for (index, label) in labels.iter().enumerate() {
                    let response = ui.menu_button(label.as_str(), |ui| {
                        any_open = true;
                        let bar = cache.get_or_insert_with(|| core.state.menu_bar(&core.env));
                        match bar.items.get(index) {
                            Some(MenuItem::Submenu { menu, .. }) => entries(ui, menu, &mut chosen),
                            Some(action @ MenuItem::Action { .. }) => entries(
                                ui,
                                &Menu {
                                    items: vec![action.clone()],
                                },
                                &mut chosen,
                            ),
                            _ => {}
                        }
                    });
                    rects.push(response.response.rect);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+").clicked() {
                    zoom += 1.;
                }
                if ui.small_button("−").clicked() {
                    zoom -= 1.;
                }
            });
        });
        if !any_open {
            // Resolved again when a menu opens next: what it holds may have
            // changed since.
            self.bar_menu = None;
        }
        self.bar_rects = rects;
        (chosen, zoom)
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
            // An open menu has the keyboard and the buttons: egui closes it on
            // Escape or a click elsewhere, and neither reaches the buffer
            // under it.
            if self.popup.is_some()
                && matches!(
                    event,
                    egui::Event::Key { .. }
                        | egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::PointerButton { .. }
                )
            {
                continue;
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
                // The menu opens on the release, as egui's own do: opened on
                // the press, the release that ends the same click is a click
                // outside it, which closes it the frame after.
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    ..
                } => self.right_press = area.contains(*pos).then_some(*pos),
                egui::Event::PointerButton {
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers,
                    ..
                } => {
                    if let Some(at) = self.right_press.take() {
                        self.core.state.handle_mouse_event(
                            grid.mouse(at, MouseKind::Down(MouseButton::Right), *modifiers),
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
                    // Ctrl and the wheel size the text, as in Emacs and in
                    // most programs, rather than scroll.
                    if modifiers.ctrl || modifiers.mac_cmd {
                        self.zoom(steps as f32);
                        continue;
                    }
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
    /// Reads the events, once per frame, draws the menu bar and the frame the
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
        let mut chosen = None;
        if self.bar_labels.is_empty() {
            self.bar_rects.clear();
            self.bar_menu = None;
        } else {
            let mut zoom = 0.;
            egui::TopBottomPanel::top("menu-bar").show(ctx, |ui| {
                let enabled = !self.core.prompt_open() && !self.pending;
                (chosen, zoom) = self.menu_bar(ui, enabled);
            });
            if accept_input {
                self.zoom(zoom);
            }
        }
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
                if let Some(id) = chosen
                    && accept_input
                {
                    self.core.state.run_menu_item(id, &self.core.env);
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
                self.take_menus(&snapshot, grid, area);
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
                // An update when the editor says one is due -- a message
                // expiring, colour arriving, a drag past a window's edge --
                // and none otherwise: a key, the mouse or a job's change
                // wakes the window on its own.
                if let Some(wake) = [
                    self.core.state.next_redraw_in(&self.core.env, &snapshot),
                    self.core.state.drag_scroll_in(),
                ]
                .into_iter()
                .flatten()
                .min()
                {
                    ctx.request_repaint_after(wake);
                }
            });
        self.show_popup(ctx);
    }
}
