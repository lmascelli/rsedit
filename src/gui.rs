use rsedit_core::{
    BufferTrait, ELispExp, EditorState, Env, KeyCode, KeyEvent, KeyModifiers, eval,
};
use std::{path::PathBuf, sync::Arc};

fn shifted(c: char) -> char {
    // Fallback for logical symbols absent from egui::Key (e.g. '_' or '<').
    // Already translated punctuation and letters pass through naturally.
    match c {
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '`' => '~',
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        c => c.to_ascii_uppercase(),
    }
}

// A Text event is authoritative for layout-dependent punctuation. Some keys
// (notably '<'/'>' on an ISO keyboard) have no egui::Key at all.
fn character_shortcut(
    fallback: Option<char>,
    mut modifiers: KeyModifiers,
    text: Option<&str>,
) -> Option<KeyEvent> {
    if !modifiers.ctrl && !modifiers.alt {
        return None;
    }
    let c = if let Some(text) = text {
        let mut chars = text.chars();
        let c = chars.next()?;
        if chars.next().is_some() || c.is_control() {
            return None;
        }
        // Preserve AltGr and non-ASCII Option/composed text as text input.
        if modifiers.alt && (modifiers.ctrl || !c.is_ascii()) {
            return None;
        }
        c
    } else {
        let c = fallback?;
        if modifiers.shift { shifted(c) } else { c }
    };
    modifiers.shift = false; // Shift is already represented by the character.
    Some(KeyEvent {
        code: KeyCode::Char(c),
        modifiers,
    })
}

struct Core<'editor, B: BufferTrait> {
    state: &'editor EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
}

impl<'editor, B: BufferTrait> Core<'editor, B> {
    fn new(
        state: &'editor EditorState<B>,
        env: Arc<Env<EditorState<B>>>,
        _font: Option<PathBuf>,
    ) -> Result<Self, String> {
        // The core loads init.lisp itself, from the same configuration as the TUI.
        let core = Self { state, env };
        core.state.resize(core.env.clone(), 100, 32);
        core.enable_unicode_typing()?;
        Ok(core)
    }

    fn quote(name: &str) -> ELispExp<B> {
        ELispExp::form(vec![
            ELispExp::symbol("quote".into()),
            ELispExp::symbol(name.into()),
        ])
    }

    fn enable_unicode_typing(&self) -> Result<(), String> {
        // The core's built-in global map binds printable ASCII. Install its
        // generic character fallback only if non-ASCII has no existing binding.
        if self
            .value("key-binding", vec![ELispExp::string("é".into())])?
            .is_nil()
        {
            let installed = self.value(
                "define-key",
                vec![
                    ELispExp::nil(),
                    ELispExp::string("<char>".into()),
                    Self::quote("self-insert"),
                ],
            )?;
            if !installed.is_truthy() {
                return Err(
                    "This frontend requires the core patch supporting <char> keymap fallbacks"
                        .into(),
                );
            }
        }
        Ok(())
    }

    fn value(
        &self,
        name: &str,
        args: Vec<ELispExp<B>>,
    ) -> Result<ELispExp<B>, String> {
        let mut form = vec![ELispExp::symbol(name.into())];
        form.extend(args);
        eval(&ELispExp::form(form), self.env.clone(), &self.state)
            .map_err(|e| format!("{e:?}{}", self.state.take_backtrace_suffix()))
    }

    fn open(&self, path: &std::path::Path) -> Result<(), String> {
        if self.prompt_open() {
            return Err("Finish the current prompt before opening another file".into());
        }
        let path = path.to_str().ok_or("rsedit requires a UTF-8 file path")?;
        if self
            .value("find-file", vec![ELispExp::string(path.into())])?
            .is_nil()
        {
            return Err(format!("Could not open {path}"));
        }
        Ok(())
    }

    fn key(&self, code: KeyCode, modifiers: KeyModifiers) {
        self.state
            .handle_key_event(KeyEvent { code, modifiers }, &self.env);
    }

    fn control(&self, c: char) {
        self.key(
            KeyCode::Char(c),
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
    }

    fn text(&self, text: &str) {
        // Typing goes through keymaps, including Vim, macros and language hooks.
        for c in text.chars() {
            let code = match c {
                '\r' | '\n' => KeyCode::Enter,
                '\t' => KeyCode::Tab,
                c if c.is_control() => continue,
                c => KeyCode::Char(c),
            };
            self.key(code, KeyModifiers::default());
        }
    }

    fn text_input(&self, text: &str, modifiers: KeyModifiers) {
        if let Some(event) = character_shortcut(None, modifiers, Some(text)) {
            self.state.handle_key_event(event, &self.env);
        } else {
            self.text(text);
        }
    }

    fn prompt_open(&self) -> bool {
        self.value("major-mode", vec![ELispExp::string("*Minibuffer*".into())])
            .is_ok_and(|mode| !mode.is_nil())
    }

    fn vim_state(&self) -> String {
        match self.value("buffer-get", vec![Self::quote("vim-state")]) {
            Ok(ELispExp::Symbol(name)) if name.as_str() != "nil" => name.to_string(),
            _ => String::new(),
        }
    }

    fn command(&self, name: &str) {
        if self.prompt_open() {
            self.state
                .set_echo_message("Finish the current prompt, or cancel it with Escape");
            return;
        }
        // Execute exactly as M-x does: command collection, pre/post hooks, undo
        // epochs and macro recording stay inside the core's command dispatcher.
        self.key(
            KeyCode::Char('x'),
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        );
        self.text(name);
        self.key(KeyCode::Enter, KeyModifiers::default());
    }

    fn request_quit(&self) {
        if self.prompt_open() {
            self.key(KeyCode::Esc, KeyModifiers::default());
        }
        self.control('g');
        self.command("quit"); // Core prompts about modified files, asynchronously.
    }
}

mod gui {
    use super::*;
    use eframe::egui::{self, Color32, FontId, Pos2, Rect, Stroke, Vec2};
    use rsedit_core::{
        Face, FrameSnapshot, MouseButton, MouseEvent, MouseKind, RenderableWindowView, Style,
    };
    use std::time::Duration;

    const BACKGROUND_POLL: Duration = Duration::from_millis(250);

    #[derive(Clone, Copy)]
    struct Grid {
        origin: Pos2,
        cell: Vec2,
        columns: usize,
        rows: usize,
    }

    impl Grid {
        fn new(area: Rect, cell: Vec2) -> Self {
            Self {
                origin: area.min,
                cell,
                columns: ((area.width() / cell.x).floor() as usize).clamp(1, 8192),
                rows: ((area.height() / cell.y).floor() as usize).clamp(2, 4096),
            }
        }

        fn at(self, x: isize, y: isize) -> Pos2 {
            self.origin + Vec2::new(x as f32 * self.cell.x, y as f32 * self.cell.y)
        }

        fn rect(self, x: isize, y: isize, width: usize, height: usize) -> Rect {
            Rect::from_min_size(
                self.at(x, y),
                self.cell * Vec2::new(width as f32, height as f32),
            )
        }

        fn mouse(self, at: Pos2, kind: MouseKind, modifiers: egui::Modifiers) -> MouseEvent {
            let offset = (at - self.origin) / self.cell;
            MouseEvent {
                column: offset.x.floor().clamp(0., u16::MAX as f32) as u16,
                row: offset.y.floor().clamp(0., u16::MAX as f32) as u16,
                kind,
                modifiers: mods(modifiers),
            }
        }
    }

    fn mods(modifiers: egui::Modifiers) -> KeyModifiers {
        KeyModifiers {
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
            caps_lock_as_ctrl: false,
        }
    }

    fn character(key: egui::Key) -> Option<char> {
        use egui::Key;
        match key {
            Key::Space => Some(' '),
            Key::Minus => Some('-'),
            Key::Plus => Some('+'),
            Key::Equals => Some('='),
            Key::OpenBracket => Some('['),
            Key::CloseBracket => Some(']'),
            Key::Backslash => Some('\\'),
            Key::Slash => Some('/'),
            Key::Comma => Some(','),
            Key::Period => Some('.'),
            Key::Semicolon => Some(';'),
            Key::Quote => Some('\''),
            Key::Backtick => Some('`'),
            _ => {
                let name = key.symbol_or_name();
                let mut chars = name.chars();
                let first = chars.next()?;
                (chars.next().is_none() && first.is_ascii_graphic())
                    .then_some(first.to_ascii_lowercase())
            }
        }
    }

    fn special_key(key: egui::Key) -> Option<KeyCode> {
        use egui::Key;
        Some(match key {
            Key::ArrowLeft => KeyCode::Left,
            Key::ArrowRight => KeyCode::Right,
            Key::ArrowUp => KeyCode::Up,
            Key::ArrowDown => KeyCode::Down,
            Key::Escape => KeyCode::Esc,
            Key::Enter => KeyCode::Enter,
            Key::Backspace => KeyCode::Backspace,
            Key::Tab => KeyCode::Tab,
            _ => return None,
        })
    }

    pub struct EditorApp<'editor, B: BufferTrait> {
        core: Core<'editor, B>,
        font_size: f32,
        size: (usize, usize),
        preedit: String,
        dragging: bool,
        wheel: f32,
        pending: bool,
        clipboard_v_seen: bool,
        last_input_frame: Option<u64>,
    }

    impl<'editor, B: BufferTrait> EditorApp<'editor, B> {
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

        fn input(&mut self, ctx: &egui::Context, area: Rect, grid: Grid) {
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
                            // egui consumes Ctrl-C/X on desktop. Restore the core's keys:
                            // C-c cancels Vim; C-x begins Emacs file/window commands.
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
                            self.core.control('v'); // Vim block selection / Emacs paging.
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
                        // egui-winit omits the press entirely when clipboard
                        // lookup is empty/unavailable. Recover C-v on release.
                        if !self.clipboard_v_seen
                            && modifiers.ctrl
                            && !modifiers.mac_cmd
                            && !modifiers.shift
                        {
                            self.core.control('v');
                        }
                        self.clipboard_v_seen = false;
                    }
                    egui::Event::Ime(egui::ImeEvent::Preedit(text)) => {
                        self.preedit.clone_from(text)
                    }
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

    fn rgb(color: rsedit_core::Color) -> Color32 {
        Color32::from_rgb(color.r, color.g, color.b)
    }

    fn colors(style: Style, default: Style) -> (Color32, Color32) {
        let mut foreground = style
            .fg
            .or(default.fg)
            .map(rgb)
            .unwrap_or(Color32::from_rgb(215, 220, 229));
        let mut background = style
            .bg
            .or(default.bg)
            .map(rgb)
            .unwrap_or(Color32::from_rgb(23, 26, 33));
        if style.reverse {
            std::mem::swap(&mut foreground, &mut background);
        }
        (foreground, background)
    }

    fn merge(base: &mut Style, overlay: Style) {
        if overlay.fg.is_some() {
            base.fg = overlay.fg;
        }
        if overlay.bg.is_some() {
            base.bg = overlay.bg;
        }
        base.bold |= overlay.bold;
        base.italic |= overlay.italic;
        base.underline |= overlay.underline;
        base.reverse |= overlay.reverse;
    }

    fn label(
        painter: &egui::Painter,
        at: Pos2,
        text: &str,
        font: &FontId,
        style: Style,
        default: Style,
    ) {
        let (color, _) = colors(style, default);
        let mut job = egui::text::LayoutJob::default();
        job.append(
            text,
            0.,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                italics: style.italic,
                underline: if style.underline {
                    Stroke::new(1_f32, color)
                } else {
                    Stroke::NONE
                },
                ..Default::default()
            },
        );
        let galley = painter.layout_job(job);
        painter.galley(at, galley.clone(), color);
        if style.bold {
            painter.galley(at + Vec2::new(0.35, 0.), galley, color);
        }
    }

    fn row(
        painter: &egui::Painter,
        grid: Grid,
        x: isize,
        y: isize,
        width: usize,
        text: &str,
        styles: &[Style],
        font: &FontId,
        default: Style,
    ) {
        // Each core column is one Unicode scalar. Paint on that same grid so
        // mouse positions, tabs, overlays and the cursor use identical columns.
        let (_, base_background) = colors(default, default);
        painter.rect_filled(grid.rect(x, y, width, 1), 0., base_background);
        let mut chars = text.chars();
        for column in 0..width {
            let c = chars.next().unwrap_or(' ');
            let style = styles.get(column).copied().unwrap_or(default);
            let (_, background) = colors(style, default);
            let cell = grid.rect(x + column as isize, y, 1, 1);
            if background != base_background {
                painter.rect_filled(cell, 0., background);
            }
            if c != ' ' && !c.is_control() {
                let mut encoded = [0; 4];
                label(
                    painter,
                    cell.min,
                    c.encode_utf8(&mut encoded),
                    font,
                    style,
                    default,
                );
            }
        }
    }

    fn paint_view(
        painter: &egui::Painter,
        grid: Grid,
        view: &RenderableWindowView,
        frame: &FrameSnapshot,
        font: &FontId,
        block: bool,
        preedit: &str,
    ) -> Option<Rect> {
        let rect = view.rect;
        let default = frame.theme.style(Face::DEFAULT);
        let (_, background) = colors(default, default);
        if view.has_border {
            let outer = grid.rect(rect.x - 1, rect.y - 1, rect.width + 2, rect.height + 2);
            painter.rect_filled(outer, 3., background);
            let (color, _) = colors(frame.theme.style(Face::WINDOW_SEPARATOR), default);
            painter.rect_stroke(
                outer.shrink(0.5),
                3.,
                Stroke::new(1_f32, color),
                egui::StrokeKind::Inside,
            );
            let title = view.title.as_deref().unwrap_or(&view.buffer_name);
            let title: String = title.chars().take(rect.width).collect();
            label(
                painter,
                grid.at(rect.x, rect.y - 1),
                &title,
                font,
                default,
                default,
            );
        }
        let text_area = grid
            .rect(rect.x, rect.y, rect.width, rect.height)
            .intersect(painter.clip_rect());
        let clipped = painter.with_clip_rect(text_area);
        clipped.rect_filled(text_area, 0., background);
        for line in 0..rect.height {
            let mut styles = vec![default; rect.width];
            for highlight in view
                .highlights
                .iter()
                .filter(|highlight| highlight.row == line)
            {
                let end = highlight.end_col.min(styles.len());
                let start = highlight.start_col.min(end);
                for style in &mut styles[start..end] {
                    merge(style, frame.theme.style(highlight.face));
                }
            }
            row(
                &clipped,
                grid,
                rect.x,
                rect.y + line as isize,
                rect.width,
                view.lines.get(line).map(String::as_str).unwrap_or(""),
                &styles,
                font,
                default,
            );
        }
        for (line, cell) in view.gutter.iter().enumerate() {
            let style = frame.theme.style(cell.face);
            row(
                painter,
                grid,
                rect.x - view.gutter_width as isize,
                rect.y + line as isize,
                view.gutter_width,
                &cell.text,
                &vec![style; view.gutter_width],
                font,
                default,
            );
        }
        if let Some(text) = &view.mode_line {
            let style = frame.theme.style(if view.is_focused {
                Face::MODE_LINE
            } else {
                Face::MODE_LINE_INACTIVE
            });
            let width = rect.width + view.gutter_width;
            row(
                painter,
                grid,
                rect.x - view.gutter_width as isize,
                rect.y + rect.height as isize,
                width,
                text,
                &vec![style; width],
                font,
                default,
            );
        }
        if view.is_focused {
            if let Some((column, line)) = view.cursor_rel_pos {
                let caret = grid.rect(rect.x + column as isize, rect.y + line as isize, 1, 1);
                let (color, _) = colors(default, default);
                if block {
                    clipped.rect_stroke(
                        caret.shrink(0.5),
                        0.,
                        Stroke::new(1.5_f32, color),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    clipped.line_segment(
                        [caret.left_top(), caret.left_bottom()],
                        Stroke::new(1.5_f32, color),
                    );
                }
                if !preedit.is_empty() {
                    let composition = Rect::from_min_size(
                        caret.min,
                        Vec2::new(
                            (preedit.chars().count() + 1) as f32 * grid.cell.x,
                            grid.cell.y,
                        ),
                    );
                    clipped.rect_filled(composition, 0., background);
                    let style = Style {
                        underline: true,
                        ..default
                    };
                    label(&clipped, caret.min, preedit, font, style, default);
                }
                return Some(caret);
            }
        }
        None
    }
    

    impl<'editor, B: BufferTrait> eframe::App for EditorApp<'editor, B> {
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
                    response.request_focus(); // The editor owns keyboard focus; prompts are core buffers.
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
                    let snapshot =
                        self.core
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
                    let block =
                        !state.is_empty() && !matches!(state.as_str(), "insert" | "replace");
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
                    // The core currently has no OS-event-loop waker for file watcher/spawn.
                    // Poll slowly when idle; active jobs can request an earlier update.
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
}

pub fn gui_main<B: BufferTrait>(
    state: &EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
    font: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let core = Core::new(state, env, font.clone())?;
    let native = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1000., 720.])
            .with_min_inner_size([360., 240.]),
        renderer: eframe::Renderer::Glow,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "rsedit",
        native,
        Box::new(move |cc| Ok(Box::new(gui::EditorApp::new(cc, core, font)?))),
    )
    .map_err(|error| error.to_string().into())
}
