//! The window front end, on egui: the editor reached through [`Core`], the
//! keys translated in `keys`, the events and the frame loop in `app`, and the
//! drawing in `paint`.
//!
//! Design: `doc/architettura-editor.typ, <gui>`.
mod app;
mod keys;
mod paint;
#[cfg(test)]
mod tests;

use keys::character_shortcut;
use rsedit_core::{BufferTrait, ELispExp, EditorState, Env, KeyCode, KeyEvent, KeyModifiers, eval};
use std::{path::PathBuf, sync::Arc};

/// The editor, as the window reaches it: everything goes through Lisp and the
/// key path, as it would from the terminal.
struct Core<'editor, B: BufferTrait> {
    state: &'editor EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
}

impl<'editor, B: BufferTrait> Core<'editor, B> {
    /// Returns the editor STATE, with ENV, sized for a first frame and able to
    /// type characters past ASCII; init.lisp has been read already.
    fn new(state: &'editor EditorState<B>, env: Arc<Env<EditorState<B>>>) -> Result<Self, String> {
        let core = Self { state, env };
        core.state.resize(core.env.clone(), 100, 32);
        core.enable_unicode_typing()?;
        Ok(core)
    }

    /// Returns the form `'NAME`.
    fn quote(name: &str) -> ELispExp<B> {
        ELispExp::form(vec![
            ELispExp::symbol("quote".into()),
            ELispExp::symbol(name.into()),
        ])
    }

    /// Binds `<char>` to `self-insert` when nothing binds `é`: the keymap
    /// written in Rust binds printable ASCII only.
    fn enable_unicode_typing(&self) -> Result<(), String> {
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

    /// Returns the value of `(NAME ARGS...)`, or the error with its backtrace.
    fn value(&self, name: &str, args: Vec<ELispExp<B>>) -> Result<ELispExp<B>, String> {
        let mut form = vec![ELispExp::symbol(name.into())];
        form.extend(args);
        eval(&ELispExp::form(form), self.env.clone(), &self.state)
            .map_err(|e| format!("{e:?}{}", self.state.take_backtrace_suffix()))
    }

    /// Visits PATH with `find-file`; refused while a prompt is open.
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

    /// Presses CODE with MODIFIERS.
    fn key(&self, code: KeyCode, modifiers: KeyModifiers) {
        self.state
            .handle_key_event(KeyEvent { code, modifiers }, &self.env);
    }

    /// Presses Ctrl and C.
    fn control(&self, c: char) {
        self.key(
            KeyCode::Char(c),
            KeyModifiers {
                ctrl: true,
                ..Default::default()
            },
        );
    }

    /// Types TEXT, a key for each character, through the keymaps, so that
    /// Vim, macros and the modes see it; control characters but Return and
    /// Tab are dropped.
    fn text(&self, text: &str) {
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

    /// Types TEXT, or presses it as a shortcut when MODIFIERS hold Ctrl or
    /// Alt.
    fn text_input(&self, text: &str, modifiers: KeyModifiers) {
        if let Some(event) = character_shortcut(None, modifiers, Some(text)) {
            self.state.handle_key_event(event, &self.env);
        } else {
            self.text(text);
        }
    }

    /// Returns whether a prompt is open.
    fn prompt_open(&self) -> bool {
        self.value("major-mode", vec![ELispExp::string("*Minibuffer*".into())])
            .is_ok_and(|mode| !mode.is_nil())
    }

    /// Returns the current buffer's Vim state, or empty when it has none.
    fn vim_state(&self) -> String {
        match self.value("buffer-get", vec![Self::quote("vim-state")]) {
            Ok(ELispExp::Symbol(name)) if name.as_str() != "nil" => name.to_string(),
            _ => String::new(),
        }
    }

    /// Runs command NAME as `M-x` does, with its hooks, its undo step and
    /// the macro being recorded; refused while a prompt is open.
    fn command(&self, name: &str) {
        if self.prompt_open() {
            self.state
                .set_echo_message("Finish the current prompt, or cancel it with Escape");
            return;
        }
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

    /// Asks the editor to quit, closing a prompt first: modified buffers are
    /// asked about as `C-x C-c` asks.
    fn request_quit(&self) {
        if self.prompt_open() {
            self.key(KeyCode::Esc, KeyModifiers::default());
        }
        self.control('g');
        self.command("quit");
    }
}

/// Runs the editor STATE, with ENV, in a window until it quits, drawing with
/// FONT when one is given.
pub fn gui_main<B: BufferTrait>(
    state: &EditorState<B>,
    env: Arc<Env<EditorState<B>>>,
    font: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let core = Core::new(state, env)?;
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
        Box::new(move |cc| Ok(Box::new(app::EditorApp::new(cc, core, font)?))),
    )
    .map_err(|error| error.to_string().into())
}
