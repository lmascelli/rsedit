use crate::buffer::{Buffer, BufferTrait};
use crate::input::{KeyCode, KeyEvent, fill_self_insert_keymaps};
use crate::lisp::{Env, EvalError, LispContext, LispExp, Parser, bootstrap_vm, eval};
use crate::ui::{
    FloatingWindow, LayoutNode, Rect, RenderableWindowView, Window, extract_buffer_lines,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    },
};
pub type ELispExp<B> = LispExp<EditorState<B>>;

/// A major mode is a collection of rules that apply to a specific
/// kind of buffers like specific programming language, special text
/// files or special buffers like the minibuffer or the repl buffer.
/// It provides custom keymap, syntax highlighting rules and hook
/// that will be called befor or after some events.
#[derive(Clone, Debug)]
pub struct MajorMode<B: BufferTrait> {
    pub name: String,
    pub keymap: HashMap<KeyEvent, ELispExp<B>>,
    pub syntax_highlighing: (), // TODO! make it a SyntaxRules
    pub hook_functions: Vec<ELispExp<B>>,
}

impl<B: BufferTrait> MajorMode<B> {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            keymap: HashMap::new(),
            syntax_highlighing: (),
            hook_functions: vec![],
        }
    }
}

/// This is the container for all the editor informations.
/// The whole editor memory should live in an instance of this
/// struct. It is generic behiond the implementation of the
/// buffer.
/// It also provides instruction for an UI provider of what to
/// render and where.
#[derive(Clone)]
pub struct EditorState<B: BufferTrait> {
    pub running: Arc<AtomicBool>,

    pub buffers: Arc<RwLock<HashMap<String, Arc<RwLock<Buffer<B>>>>>>,
    pub echo_message: Arc<RwLock<String>>,
    pub current_buffer_name: Arc<RwLock<String>>,

    /// A keymap is an association between a KeyEvent and the name of a
    /// function that have to be executed (i.e. self-insert)
    pub keymaps: Arc<RwLock<HashMap<KeyEvent, String>>>,
    pub mode_registry: Arc<RwLock<HashMap<String, MajorMode<B>>>>,
    /// This is the root of the window tree that the UI should visualize
    pub layout_root: Arc<RwLock<LayoutNode>>,
    /// This is a list of floating window that will be renderered above the
    /// others
    pub floating_windows: Arc<RwLock<Vec<FloatingWindow>>>,
    pub focused_window_id: Arc<RwLock<usize>>,
    /// A value only used to fastly create a new window id
    pub next_window_id: Arc<AtomicUsize>,

    /// The fuel of the lisp machine, if somehow it will start to use too much
    /// cpu power, it will run out of fuel
    fuel: Arc<AtomicU32>,
    /// Here the lisp VM will output its logs
    logs: Arc<RwLock<Vec<String>>>,
}

impl<B: BufferTrait> LispContext for EditorState<B> {
    fn consume_fuel(&self, amount: u32) -> Result<(), EvalError> {
        if self.fuel.load(Ordering::Relaxed) > amount {
            self.fuel.fetch_sub(amount, Ordering::Relaxed);
            Ok(())
        } else {
            self.fuel.store(0, Ordering::Relaxed);
            Err(EvalError::OutOfFuel)
        }
    }

    fn log_diagnostic(&self, msg: &str) {
        let mut lock = self
            .logs
            .write()
            .expect("Failed to get the write lock on logs");
        lock.push(msg.into());
    }
}

impl<B: BufferTrait> std::fmt::Debug for EditorState<B> {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        todo!()
    }
}
impl<B: BufferTrait> std::cmp::PartialEq for EditorState<B> {
    fn eq(&self, _: &EditorState<B>) -> bool {
        unreachable!()
    }
}

impl<B: BufferTrait> EditorState<B> {
    /// Create a new EditorState environment. Install the default keymaps,
    /// provides a default *scratch* buffer in a base window.
    fn new() -> Self {
        let mut buffers = HashMap::new();
        let scratch_name = "*scratch*".to_string();
        buffers.insert(
            scratch_name.clone(),
            Arc::new(RwLock::new(Buffer::new(&scratch_name))),
        );

        let mut keymaps = HashMap::new();
        fill_self_insert_keymaps(&mut keymaps);

        Self {
            running: Arc::new(AtomicBool::new(true)),
            buffers: Arc::new(RwLock::new(buffers)),
            echo_message: Arc::new(RwLock::new("Welcome to rsedit".to_string())),
            current_buffer_name: Arc::new(RwLock::new(scratch_name)),
            keymaps: Arc::new(RwLock::new(keymaps)),
            mode_registry: Arc::new(RwLock::new(HashMap::new())),
            layout_root: Arc::new(RwLock::new(LayoutNode::Leaf(Window {
                id: 0,
                buffer_name: String::from("*scratch*"),
                scroll_x: 0,
                scroll_y: 0,
            }))),
            floating_windows: Arc::new(RwLock::new(Vec::new())),
            focused_window_id: Arc::new(RwLock::new(0)),
            next_window_id: Arc::new(AtomicUsize::new(1)),
            fuel: Arc::new(AtomicU32::new(10_000)),
            logs: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Quit the editor
    pub fn quit(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Open a new empty buffer or load a file into a new buffer if a path is
    /// provided.
    pub fn new_buffer(&self, name: &str, path: Option<&str>) -> Option<String> {
        if let Some(file_path) = path {
            match std::fs::read_to_string(file_path) {
                Ok(content) => {
                    let mut new_buf = Buffer::from_text(name, &content);
                    new_buf.file_path = Some(file_path.to_string());

                    let mut buffers_lock = self
                        .buffers
                        .write()
                        .expect("Failed to get write lock on buffers");
                    buffers_lock.insert(name.to_string(), Arc::new(RwLock::new(new_buf)));

                    self.set_current_buffer_name(name);

                    self.set_echo_message(&format!("Loaded {}", file_path));
                    Some(name.to_string())
                }
                Err(e) => {
                    self.set_echo_message(&format!("Error reading file: {}", e));
                    None
                }
            }
        } else {
            let mut buffers_lock = self
                .buffers
                .write()
                .expect("Failed to get write lock on buffers");
            buffers_lock.insert(name.to_string(), Arc::new(RwLock::new(Buffer::new(name))));
            Some(name.to_string())
        }
    }

    /// Handle a key event. An UI provider is responsible to call this function
    /// every time it want to make the editor react to an user input.
    pub fn handle_key_event(&self, event: KeyEvent, env: &Arc<Env<EditorState<B>>>) {
        let symbol_name = if let Some(symbol_name) = self
            .keymaps
            .read()
            .expect("Failed to acquire read lock on keymaps")
            .get(&event)
        {
            symbol_name.to_string()
        } else {
            self.set_echo_message(&format!("Keymap not bound {:?}", event));
            return;
        };
        let mut ast = vec![ELispExp::symbol(symbol_name)];
        if let KeyCode::Char(c) = event.code {
            ast.push(ELispExp::string(c.to_string()));
        }
        let ast = ELispExp::list(ast);
        if let Err(e) = eval(&ast, env.clone(), self) {
            self.set_echo_message(&format!("Eval Error: {:?} {:?}", ast, e));
        } else {
            self.set_echo_message("");
        }
    }

    /// Ask the editor for a list of window to be rendered. Those are composed of a rect that tells
    /// where the window is placed and its size, the name of the buffer it represents, if it's
    /// focused, the relative cursor position in it, if it has a border and of course the line
    /// that it contains and that have to be drawn.
    pub fn compose_layout(
        &self,
        screen_width: usize,
        screen_height: usize,
    ) -> Vec<RenderableWindowView> {
        let mut views = Vec::new();
        let tiled_space = Rect {
            x: 0,
            y: 0,
            width: screen_width,
            height: screen_height,
        };
        self.layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root")
            .compute_tiled_views(
                tiled_space,
                self.get_focused_window_id(),
                self.buffers
                    .read()
                    .as_ref()
                    .expect("Failed to acquire a read lock on buffers"),
                &mut views,
            );

        for float in self
            .floating_windows
            .as_ref()
            .read()
            .expect("Failed to acquire a read lock on floating_windows")
            .iter()
        {
            let lines = extract_buffer_lines(
                &float.window,
                &float.rect,
                self.buffers
                    .read()
                    .as_ref()
                    .expect("Failed to acquire a read lock on buffers"),
            );
            let is_focused = float.window.id == self.get_focused_window_id();

            let cursor_rel_pos = if is_focused {
                if let Some(buf) = self
                    .buffers
                    .read()
                    .expect("Failed to acquire read lock on buffers")
                    .get(&float.window.buffer_name)
                {
                    let (c_line, c_col) = buf
                        .read()
                        .expect("Failed to acquire read lock on buffer")
                        .text
                        .cursor_pos();
                    Some((
                        c_col.saturating_sub(float.window.scroll_x),
                        c_line.saturating_sub(float.window.scroll_y),
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            views.push(RenderableWindowView {
                rect: float.rect.clone(),
                buffer_name: float.window.buffer_name.clone(),
                is_focused,
                cursor_rel_pos,
                lines,
                has_border: float.has_border,
            });
        }

        views
    }

    //--------------------------------------------------------------------------
    //                         GETTERS AND SETTERS
    //--------------------------------------------------------------------------

    /// Return the editor echo string
    pub fn get_echo_message(&self) -> String {
        let lock = self
            .echo_message
            .read()
            .expect("Failed to acquire read lock on echo_message");
        lock.clone()
    }

    /// Set the echo message to be MSG
    pub fn set_echo_message(&self, msg: &str) {
        *self
            .echo_message
            .write()
            .expect("Failed to acquire write lock on echo_message") = msg.to_string();
    }

    /// Return the next valid ID for a new window
    pub fn get_next_window_id(&self) -> usize {
        self.next_window_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Get the ID of the current focused window
    pub fn get_focused_window_id(&self) -> usize {
        let lock = self
            .focused_window_id
            .read()
            .expect("Failed to acquire read lock on focused_window_id");
        lock.clone()
    }

    /// Get the name of the current buffer
    pub fn get_current_buffer_name(&self) -> String {
        self.current_buffer_name
            .read()
            .expect("Failed to acquire read lock on current_buffer_name")
            .to_string()
    }

    /// Set the name of the current buffer
    pub fn set_current_buffer_name(&self, name: &str) {
        *self
            .current_buffer_name
            .write()
            .expect("Failed to acquire write lock on current_buffer_name") = name.to_string();
    }

    /// Returns an Arc reference to the current buffer
    fn get_current_buffer(&self) -> Arc<RwLock<Buffer<B>>> {
        self.buffers
            .read()
            .expect("Failed to acquire read lock on buffers")
            .get(&self.get_current_buffer_name())
            .expect("Corruption in the hashmap of buffers")
            .clone()
    }

    pub fn get_buffer(&self, name: &str) -> Arc<RwLock<Buffer<B>>> {
        self.buffers
            .read()
            .expect("Failed to acquire read lock on buffers")
            .get(name)
            .expect("Corruption in the hashmap of buffers")
            .clone()
    }

    /// Apply the operation OP to the buffer BUF
    pub fn mutate_buffer<F, R>(&self, buffer: Arc<RwLock<Buffer<B>>>, op: F) -> R
    where
        F: FnOnce(&mut Buffer<B>) -> R,
    {
        let mut guard = buffer
            .write()
            .expect("Failed to acquire write lock on current buffer");
        op(&mut *guard)
    }
}

/// Create a global EditorState environment and a Lisp environment associated to it.
/// It installs in the lisp environment all the primitive functions to use the editor.
/// It is mandatory that the lisp environment does not outlive the EditorState struct.
pub fn create_global_env<B: BufferTrait>()
-> Result<(EditorState<B>, Arc<Env<EditorState<B>>>), EvalError> {
    let editor_state = EditorState::new();
    let env = bootstrap_vm(&editor_state)?;

    // ---------------------- FILLING PRIMITIVE FUNCTIONS ----------------------
    macro_rules! insert_fn {
        ($name:literal, $func:ident) => {
            env.set_function($name.into(), LispExp::Primitive(primitives::$func));
        };
    }
    
    insert_fn!("quit", quit);
    insert_fn!("load-file", load_file);
    insert_fn!("define-key", define_key);
    insert_fn!("make-mode", make_mode);
    insert_fn!("self-insert", self_insert);
    insert_fn!("insert-newline", insert_newline);
    insert_fn!("delete-backward-char", delete_backward_char);
    insert_fn!("backward-char", backward_char);
    insert_fn!("forward-char", forward_char);
    insert_fn!("previous-line", previous_line);
    insert_fn!("next-line", next_line);
    insert_fn!("find-file", find_file);
    insert_fn!("save-buffer", save_buffer);

    // --------------------- LOADING LISP STDLIB -------------------------------

    let mut stdlib_src = format!("(progn {})", include_str!(
        concat!(env!("CARGO_MANIFEST_DIR"), "/lisp/stdlib.lisp")
    ));

    let mut parser = Parser::new(&stdlib_src);

    if let Ok(ast) = parser.next() {
        if let Err(e) = eval(&ast, env.clone(), &editor_state) {
            editor_state.log_diagnostic(&format!("Stdlib Eval Error: {:?}", e));
        }
    } else {
        editor_state.log_diagnostic("CRITICAL: Failed to parse the standard library");
    }
    
    Ok((editor_state, env))
}

// ---------------------------------------------------------------------------//
//                                                                            //
//                                  PRIMITIVES                                //
//                                                                            //
// ---------------------------------------------------------------------------//

mod primitives {
    use super::{BufferTrait, ELispExp, EditorState, MajorMode};
    use crate::{
        input::{KeyCode, KeyEvent, KeyModifiers},
        lisp::{EvalError, LispExp},
    };

    fn is_nil<B: BufferTrait>(args: &[ELispExp<B>]) -> bool {
        args.len() == 0
            || (args.len() == 1 && (args[0] == ELispExp::list(vec![]))
                || args[0] == ELispExp::symbol("nil".into()))
    }

    fn parse_key_sequence(seq: &str) -> Option<KeyEvent> {
        let mut modifiers = KeyModifiers::default();
        let mut chars = seq.chars().peekable();

        if seq.starts_with("C-") {
            modifiers.ctrl = true;
            chars.nth(0);
            chars.nth(0);
        } else if seq.starts_with("M-") {
            modifiers.alt = true;
            chars.nth(0);
            chars.nth(0);
        } else if seq.starts_with("C-M-") {
            modifiers.ctrl = true;
            modifiers.alt = true;
            chars.nth(0);
            chars.nth(0);
            chars.nth(0);
            chars.nth(0);
        }

        let key_code = match chars.collect::<String>().as_str() {
            "<ret>" | "<Return>" => KeyCode::Enter,
            "<backspace>" => KeyCode::Backspace,
            "<up>" => KeyCode::Up,
            "<down>" => KeyCode::Down,
            "<left>" => KeyCode::Left,
            "<right>" => KeyCode::Right,
            s if s.len() == 1 => KeyCode::Char(
                s.chars()
                    .next()
                    .expect(&format!("Failed to interpret the sequence {seq}")),
            ),
            _ => return None,
        };

        Some(KeyEvent {
            code: key_code,
            modifiers,
        })
    }

    macro_rules! nil {
        () => {
            ELispExp::symbol("nil".into())
        };
    }

    macro_rules! primitive {
        ($func_name:ident, $args:ident, $ctx:ident, $body:block) => {
            pub fn $func_name<B: BufferTrait>(
                $args: &[ELispExp<B>],
                $ctx: &EditorState<B>,
            ) -> Result<ELispExp<B>, EvalError> {
                $body
            }
        };
    }

    //------------------------------------------------------------//
    //                                                            //
    //                          EDITOR                            //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(quit, _args, ctx, {
        ctx.quit();
        Ok(nil!())
    });

    primitive!(load_file, args, ctx, {
        if let Some(ELispExp::String(path_str)) = args.first() {
            match std::fs::read_to_string(path_str.to_string()) {
                Ok(content) => {
                    let wrapped_content = format!("(progn {})", content);
                    let mut parser = crate::lisp::Parser::new(&wrapped_content);
                    match parser.next() {
                        Ok(ast) => Ok(ast),
                        Err(e) => {
                            ctx.set_echo_message(&format!("Parse Error in {}: {:?}", path_str, e));
                            Ok(nil!())
                        }
                    }
                }
                Err(e) => {
                    ctx.set_echo_message(&format!("Could not load {}: {}", path_str, e));
                    Ok(nil!())
                }
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: format!("{:?}", args.first()),
            })
        }
    });

    primitive!(define_key, args, ctx, {
        if args.len() != 2 {
            Err(EvalError::WrongNumberOfArguments {
                expected: 2,
                got: args.len(),
            })
        } else {
            if let (ELispExp::String(key_str), ELispExp::Symbol(func_name)) = (&args[0], &args[1]) {
                if let Some(key_event) = parse_key_sequence(key_str) {
                    let mut keymaps = ctx
                        .keymaps
                        .write()
                        .expect("Failed to acquire write lock on keymaps");
                    keymaps.insert(key_event, func_name.to_string());
                    Ok(ELispExp::symbol("t".into()))
                } else {
                    ctx.set_echo_message(&format!("Invalid key sequence: {}", key_str));
                    Ok(nil!())
                }
            } else {
                Err(EvalError::WrongArgumentType {
                    expected: "String, Symbol".into(),
                    got: format!("{:?}, {:?}", &args[0], &args[1]),
                })
            }
        }
    });

    //------------------------------------------------------------//
    //                                                            //
    //                      MAJOR MODES                           //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(make_mode, args, ctx, {
        if args.len() != 1 {
            Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: args.len(),
            })
        } else {
            if let Some(LispExp::String(mode_name)) = args.get(0) {
                ctx.mode_registry
                    .write()
                    .expect("Failed to acquire write lock on mode_registry")
                    .insert(mode_name.to_string(), MajorMode::new(mode_name));
                Ok(ELispExp::symbol("t".into()))
            } else {
                Err(EvalError::WrongArgumentType {
                    expected: "String".into(),
                    got: format!("{:?}", args.get(0)),
                })
            }
        }
    });

    //------------------------------------------------------------//
    //                                                            //
    //                          EDIT                              //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(self_insert, args, ctx, {
        if let Some(LispExp::String(s)) = args.first() {
            if let Some(c) = s.chars().next() {
                ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
                    buf.text.insert(c);
                    buf.is_modified = true;
                });
            }
            Ok(LispExp::symbol("nil".into()))
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: format!("{:?}", args.first()),
            })
        }
    });

    primitive!(insert_newline, _args, ctx, {
        ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
            buf.text.insert('\n');
            buf.is_modified = true;
        });
        Ok(LispExp::symbol("nil".into()))
    });

    primitive!(delete_backward_char, _args, ctx, {
        let buf = ctx.get_current_buffer();
        let mut buf = buf
            .write()
            .expect("Failed to acquire a write lock on buffer");
        buf.text.delete();
        buf.is_modified = true;
        Ok(nil!())
    });

    //------------------------------------------------------------//
    //                                                            //
    //                     CURSOR MOVEMENT                        //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(forward_char, args, ctx, {
        let buf = ctx.get_current_buffer();
        let mut buf = buf
            .write()
            .expect("Failed to acquire a write lock on buffer");
        let step = if is_nil(args) {
            1
        } else {
            if let ELispExp::Number(n) = args[0] {
                n.floor() as usize
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: format!("{:?}", args[0]),
                });
            }
        };
        let (line, col) = buf.text.cursor_pos();
        buf.text.cursor_move(line, col + step);

        Ok(nil!())
    });

    primitive!(backward_char, args, ctx, {
        let buf = ctx.get_current_buffer();
        let mut buf = buf
            .write()
            .expect("Failed to acquire a write lock on buffer");
        let step = if is_nil(args) {
            1
        } else {
            if let ELispExp::Number(n) = args[0] {
                n.floor() as usize
            } else {
                return Err(EvalError::WrongArgumentType {
                    expected: "Number".into(),
                    got: format!("{:?}", args[0]),
                });
            }
        };
        let (line, col) = buf.text.cursor_pos();
        if col >= step {
            buf.text.cursor_move(line, col - step);
        } else {
            buf.text.cursor_move(line, 0);
        }
        Ok(nil!())
    });

    primitive!(previous_line, _args, ctx, {
        let buf = ctx.get_current_buffer();
        let mut buf = buf
            .write()
            .expect("Failed to acquire a write lock on buffer");
        let (line, col) = buf.text.cursor_pos();
        if line > 0 {
            buf.text.cursor_move(line - 1, col);
        } else {
            buf.text.cursor_move(0, 0);
        }
        Ok(nil!())
    });

    primitive!(next_line, _args, ctx, {
        let buf = ctx.get_current_buffer();
        let mut buf = buf.write().expect("Failed to acquire write lock on buffer");
        let (line, col) = buf.text.cursor_pos();
        buf.text.cursor_move(line + 1, col);
        Ok(nil!())
    });

    //------------------------------------------------------------//
    //                                                            //
    //                          I/O                               //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(find_file, args, ctx, {
        if let Some(ELispExp::String(path_str)) = args.first() {
            let path_str = path_str.to_string();
            let path = std::path::Path::new(&path_str);
            let file_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let buf_name = if file_name.is_empty() {
                path_str.to_string()
            } else {
                file_name
            };
            match ctx.new_buffer(&buf_name, Some(&path_str)) {
                Some(buf_name) => Ok(ELispExp::string(buf_name)),
                None => Ok(nil!()),
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: format!("{:?}", args.first()),
            })
        }
    });

    primitive!(save_buffer, _args, ctx, {
        let buf = ctx.get_current_buffer();
        let mut buf = buf.write().expect("Failed to acquire write lock on buffer");
        let path = if let Some(path) = &buf.file_path {
            path.to_string()
        } else {
            ctx.set_echo_message("No file associated with this buffer");
            return Ok(nil!());
        };
        let content = buf.text.to_string();
        match std::fs::write(&path, content) {
            Ok(_) => {
                buf.is_modified = false;
                ctx.set_echo_message(&format!("Wrote {}", path));
                Ok(nil!())
            }
            Err(e) => {
                ctx.set_echo_message(&format!("Failed to save: {}", e));
                Ok(nil!())
            }
        }
    });
}
