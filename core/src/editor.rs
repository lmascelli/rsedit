use crate::{
    ELispExp,
    buffer::{Buffer, BufferTrait},
    input::{KeyEvent, fill_default_keymaps},
    lisp::{Env, EvalError, LispContext, LispExp, Parser, bootstrap_vm, eval},
    modes::MajorMode,
    task::{BackgroundScheduler, WorkerMessage},
    ui::{FloatingWindow, LayoutNode, Rect, RenderableWindowView, Window, extract_buffer_lines},
};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::Sender,
    },
};

/// This is the container for all the editor informations.
/// The whole editor memory should live in an instance of this
/// struct. It is generic behiond the implementation of the
/// buffer.
/// It also provides instruction for an UI provider of what to
/// render and where.
#[derive(Clone)]
pub struct EditorState<B: BufferTrait> {
    pub running: Arc<AtomicBool>,
    /// A channel that is used to send work to a worker thread like
    /// the syntax highlighting computation
    pub worker_mailbox: Sender<WorkerMessage<B>>,

    pub buffers: Arc<RwLock<HashMap<String, Arc<RwLock<Buffer<B>>>>>>,
    pub echo_message: Arc<RwLock<String>>,
    pub current_buffer_name: Arc<RwLock<String>>,

    /// A keymap is an association between a KeyEvent and the name of a
    /// function that have to be executed (i.e. self-insert)
    pub keymaps: Arc<RwLock<HashMap<KeyEvent, ELispExp<B>>>>,
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
    /// If some, is the file where the logs will be written into
    log_file: Option<Arc<RwLock<File>>>,
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

        if let Some(log_file) = &self.log_file {
            log_file
                .write()
                .expect("Failed to acquire write lock on log_file")
                .write_all(&format!("{msg}\n").into_bytes())
                .expect("Failed to write into log file");
        }
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
        fill_default_keymaps(&mut keymaps);

        // Create a secondary worker thread and the communication channels with it.
        let (sender, receiver) = std::sync::mpsc::channel();

        let editor_state = Self {
            running: Arc::new(AtomicBool::new(true)),
            worker_mailbox: sender,
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
            log_file: None,
        };
        BackgroundScheduler::spawn(receiver, editor_state.clone());

        editor_state
    }

    /// Enable writing logs to the specified file.
    pub fn enable_log_file<P: AsRef<std::path::Path>>(&mut self, path: P) -> std::io::Result<()> {
        let mut log_file = File::create(path)?;
        // TODO(uncertain) maybe this is an unwanted change, i don't know if it's better to be
        // able to enable the writing of the logs only at specific times and maybe disable it
        // to get only some logs.

        // Write the previous unwritten log messages
        for msg in self
            .logs
            .read()
            .expect("Failed to acquire read lock on logs")
            .iter()
        {
            log_file.write_all(&format!("[LOG] {msg}\n").into_bytes())?;
        }
        // _TODO

        self.log_file.replace(Arc::new(RwLock::new(log_file)));
        Ok(())
    }

    /// Eval a lisp file in the editor context. First it look for the file as an
    /// absolute or relative path and if exists evalues it. If not and if the path
    /// is instead the name of a file without extension, it looks for
    /// it in the `lisp_path` paths in order and if it exists evalues it. If the
    /// file doesn't exists in any of this paths returns nil and simply reports
    /// it on the logs and doesn't evaluate anything otherwise evaluate it and if
    /// the evaluation succeeds return a list with the result of the evaluation or
    /// the error of the evaluation.
    pub fn eval_file(&self, file: &str, env: Arc<Env<Self>>) -> Result<ELispExp<B>, EvalError> {
        let content = format!(
            "(progn {})",
            // first search the file as a relative path
            match std::fs::read_to_string(file) {
                Ok(content) => content,
                Err(err) => {
                    // if file wasn't a path to an existing file
                    if let std::io::ErrorKind::NotFound = err.kind() {
                        // if file is instead a name of a lisp file without extension
                        if file.contains('\\') || file.contains('/') || file.contains('.') {
                            self.log_diagnostic(&format!(
                                "[ERROR] eval_file {file} is not a valid script name"
                            ));
                            return Ok(ELispExp::list(vec![]));
                        } else {
                            // search for file.lisp in every lisp-path folder
                            // 1. get the lisp-path lists, and check it is a list of strings
                            let mut lisp_path = vec![];
                            if let Some(lisp_path_list) = env.get_variable("lisp-path") {
                                if let ELispExp::List(paths) = lisp_path_list {
                                    for ipath in paths.iter() {
                                        if let ELispExp::String(path) = ipath {
                                            lisp_path.push(path.clone());
                                        } else {
                                            self.log_diagnostic(&format!(
                                                "Element in lisp-path is not a path {:?}",
                                                ipath
                                            ));
                                        }
                                    }
                                } else {
                                    self.log_diagnostic(
                                        "Variable lisp-path is not a list of paths",
                                    );
                                    return Ok(ELispExp::symbol("nil".into()));
                                }
                            };
                            // 2. check for each path in lisp-path if there is a file in *path*/*file*.lisp
                            let mut script_content = None;
                            for path in lisp_path {
                                if let Ok(content) =
                                    std::fs::read_to_string(&format!("{path}/{file}.lisp"))
                                {
                                    script_content = Some(content);
                                    break;
                                }
                            }
                            if let Some(content) = script_content {
                                content
                            } else {
                                self.log_diagnostic(&format!(
                                    "[ERROR] eval_file {file}.lisp was not found in lisp_path"
                                ));
                                return Ok(ELispExp::list(vec![]));
                            }
                        }
                    } else {
                        self.log_diagnostic(&format!("[ERROR] Failed eval file {file} {:?}", err));
                        return Ok(ELispExp::list(vec![]));
                    }
                }
            }
        );

        let ast = if let Ok(ast) = Parser::new(&content).next() {
            ast
        } else {
            return Ok(ELispExp::list(vec![]));
        };

        Ok(ELispExp::list(vec![eval(&ast, env.clone(), self)?]))
    }

    /// Quit the editor
    fn quit(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Open a new empty buffer or load a file into a new buffer if a path is
    /// provided.
    pub(crate) fn new_buffer(
        &self,
        name: &str,
        path: Option<&str>,
        start_mode: Option<String>,
    ) -> Option<String> {
        if let Some(file_path) = path {
            match std::fs::read_to_string(file_path) {
                Ok(content) => {
                    let mut new_buf = Buffer::from_text(name, &content);
                    new_buf.current_mode = if let Some(mode_name) = start_mode {
                        mode_name
                    } else {
                        "fundamental".into()
                    };
                    new_buf.file_path = Some(file_path.to_string());

                    let mut buffers_lock = self
                        .buffers
                        .write()
                        .expect("Failed to get write lock on buffers");
                    buffers_lock.insert(name.to_string(), Arc::new(RwLock::new(new_buf)));

                    self.set_current_buffer_name(name);
                    if let Some(window) = self
                        .layout_root
                        .write()
                        .expect("Failed to acquire write lock on layout_root")
                        .get_window_by_id(self.get_focused_window_id())
                    {
                        window.buffer_name = name.to_string();
                    }

                    Some(name.to_string())
                }
                Err(e) => {
                    self.log_diagnostic(&format!("Error reading file: {}", e));
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
        let mut ast = ELispExp::list(vec![]);
        let mut keymap_found = false;

        // Look for the keymap in the major mode of the current buffer
        if let Some(current_mode) = self
            .mode_registry
            .read()
            .expect("Failed to acquire read lock on mode_registry")
            .get(
                &self
                    .get_current_buffer()
                    .read()
                    .expect("Failed to acquire read lock on current_buffer")
                    .current_mode,
            )
        {
            if let Some(mode_ast) = current_mode.keymaps.get(&event) {
                ast = mode_ast.clone();
                keymap_found = true;
            }
        };

        // If no keymap was found in the major mode look for it in the global keymaps
        if !keymap_found
            && let Some(global_ast) = self
                .keymaps
                .read()
                .expect("Failed to acquire read lock on keymaps")
                .get(&event)
        {
            ast = global_ast.clone();
            keymap_found = true;
        };

        if let ELispExp::Lambda(ref lambda) = ast {
            if lambda.params.len() != 0 {
                self.log_diagnostic(&format!("Keymap {event:?} associated to a lambda with some parameters. Associate it to a lambda with 0 parameters."));
                return;
            } else {
                ast = ELispExp::list(vec![ELispExp::symbol("funcall".into()), ast]);
            }
        }

        // If no symbol has been found
        if !keymap_found {
            self.log_diagnostic(&format!("[INFO] Keymap not bound {:?}", event));
            return;
        };

        if let Err(e) = eval(&ast, env.clone(), self) {
            self.log_diagnostic(&format!("Eval Error: {:?} {:?}", ast, e));
            return;
        }

        // Handle the post command hooks
        let current_mode_name = {
            let buf_arc = self.get_current_buffer();
            let buf_lock = buf_arc
                .read()
                .expect("Failed to acquire read lock on current buffer");
            buf_lock.current_mode.clone()
        };

        let registry = self
            .mode_registry
            .read()
            .expect("Failed to acquire read lock on mode registry");
        if let Some(mode) = registry.get(&current_mode_name) {
            if let Some(hook_vec) = mode.hooks.get("post-command-hook") {
                for hook in hook_vec {
                    let hook_call = ELispExp::list(vec![hook.clone()]);
                    if let Err(e) = eval(&hook_call, env.clone(), self) {
                        self.log_diagnostic(&format!("Hook {:?} execution failed: {:?}", hook, e));
                    }
                }
            }
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

    pub fn resize(&self, env: Arc<Env<Self>>, new_screen_width: usize, new_screen_height: usize) {
        env.set_variable(
            "frame-width".into(),
            ELispExp::number(new_screen_width as f64),
        );
        env.set_variable(
            "frame-height".into(),
            ELispExp::number(new_screen_height as f64),
        );
        if let Some(ELispExp::List(callback_list)) = env.get_variable("after-resize-hook") {
            for el in callback_list.iter() {
                match el {
                    ELispExp::Lambda(_) | ELispExp::Symbol(_) => {
                        if let Err(err) = eval(
                            &ELispExp::list(vec![
                                el.clone(),
                                ELispExp::number(new_screen_width as f64),
                                ELispExp::number(new_screen_height as f64),
                            ]),
                            env.clone(),
                            self,
                        ) {
                            self.log_diagnostic(&format!("[ERROR] resize: {:?}", err));
                        }
                    }
                    _ => {
                        self.log_diagnostic(&format!(
                            "[WARNING] not a valid lambda for after-resize-hook {:?}",
                            el
                        ));
                    }
                }
            }
        } else {
            self.log_diagnostic(
                "[WARNING] there is not after-resize-hook variable bound or it's not a list",
            );
        }
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
    fn get_next_window_id(&self) -> usize {
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

    pub(crate) fn set_focused_window_id(&self, id: usize) {
        *self
            .focused_window_id
            .write()
            .expect("Failed to acquire write lock on focused_window_id") = id;
    }

    /// Get the name of the current buffer
    fn get_current_buffer_name(&self) -> String {
        self.current_buffer_name
            .read()
            .expect("Failed to acquire read lock on current_buffer_name")
            .to_string()
    }

    /// Set the name of the current buffer
    fn set_current_buffer_name(&self, name: &str) {
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

    pub(crate) fn get_buffer(&self, name: &str) -> Option<Arc<RwLock<Buffer<B>>>> {
        if let Some(buffer_arc) = self
            .buffers
            .read()
            .expect("Failed to acquire read lock on buffers")
            .get(name)
        {
            Some(buffer_arc.clone())
        } else {
            None
        }
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

    // ---------------------- EDITOR ENVIRONMENT CONFIGURATION ----------------------

    // Add the rsedit std lisp sources to *lisp-path*
    match std::env::current_exe() {
        Ok(exe_path) => {
            env.set_variable(
                "lisp-path".into(),
                ELispExp::list(vec![ELispExp::string(format!(
                    "{}/data/lisp",
                    exe_path
                        .parent()
                        .expect("Failed to get the parent directory of rsedit")
                        .display()
                ))]),
            );
        }
        Err(e) => {
            editor_state.log_diagnostic(&format!(
                "[ERROR] Failed to find the path of rsedit executable. {:?}",
                e
            ));
        }
    }

    // Add a list of callbacks that will be called after a resize event.
    // The list will contain lambdas with arguments (new_width, new_height)
    env.set_variable("after-resize-hook".into(), ELispExp::list(vec![]));

    // Create the fundamental modes:
    // - fundamental-mode to edit base files
    editor_state
        .mode_registry
        .write()
        .expect("Failed to acquire write lock on mode_registry")
        .insert(
            "fundamental-mode".into(),
            MajorMode::new("fundamental-mode".into()),
        );

    // ---------------------- FILLING PRIMITIVE FUNCTIONS ----------------------
    macro_rules! insert_fn {
        ($name:literal, $func:ident) => {
            env.set_function($name.into(), LispExp::Primitive(primitives::$func));
        };
    }

    insert_fn!("quit", quit);
    insert_fn!("eval-file", eval_file);
    insert_fn!("define-key", define_key);
    insert_fn!("log", log);
    insert_fn!("make-mode", make_mode);
    insert_fn!("add-hook", add_hook);
    insert_fn!("add-syntax-rule", add_syntax_rule);
    insert_fn!("self-insert", self_insert);
    insert_fn!("insert-newline", insert_newline);
    insert_fn!("delete-backward-char", delete_backward_char);
    insert_fn!("backward-char", backward_char);
    insert_fn!("forward-char", forward_char);
    insert_fn!("previous-line", previous_line);
    insert_fn!("next-line", next_line);
    insert_fn!("find-file", find_file);
    insert_fn!("save-buffer", save_buffer);
    insert_fn!("make-floating-window", make_floating_window);
    insert_fn!("close-floating-window", close_floating_window);
    insert_fn!("switch-to-buffer", switch_to_buffer);
    insert_fn!("buffer-string", buffer_string);
    insert_fn!("clear-buffer", clear_buffer);

    // --------------------- LOADING LISP CONFIGURATION -------------------------------

    // Set the `rsedit-path' env variable to the path of rsedit
    let current_exe_path =
        std::env::current_exe().expect("Failed to locate the path of rsedit executable");
    env.set_variable(
        "rsedit-path".into(),
        ELispExp::string(format!("{}", current_exe_path.display())),
    );

    // Look if there is a init.lisp file in
    // - LINUX: ~/.config/rsedit/init.lisp
    // - WINDOW: ~/AppData/Roaming/rsedit/init.lisp
    // and if found not found create it and the path
    // then evaluate it

    let mut user_config_path = PathBuf::new();

    #[cfg(target_os = "windows")]
    if let Ok(appdata) = std::env::var("APPDATA") {
        user_config_path.push(appdata);
        user_config_path.push("rsedit");
        user_config_path.push("init.lisp");
    }

    #[cfg(not(target_os = "windows"))]
    if let Ok(appdata) = std::env::var("HOME") {
        user_config_path.push(appdata);
        user_config_path.push(".config");
        user_config_path.push("rsedit");
        user_config_path.push("init.lisp");
    }

    if !user_config_path.as_os_str().is_empty() && !user_config_path.exists() {
        if let Some(parent_dir) = user_config_path.parent() {
            if let Err(err) = fs::create_dir_all(parent_dir) {
                editor_state.log_diagnostic(&format!(
                    "[ERROR] Failed to create the user configuration dir {}",
                    err
                ));
            } else {
                if let Err(err) = fs::write(
                    &user_config_path,
                    r#";; rsedit init.lisp
;; Add your configuration here
(eval-file "common-keymaps")
(eval-file "minibuffer")


"#,
                ) {
                    editor_state.log_diagnostic(&format!(
                        "[ERROR] Failed to write default user configuration {}",
                        err
                    ));
                }
            }
        }
    }

    editor_state.eval_file(
        user_config_path
            .to_str()
            .expect("Failed to retrieve a valid String from user_config_path"),
        env.clone(),
    )?;

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
        lisp::{Env, EvalError, LispContext, LispExp},
        modes::SyntaxRule,
        ui::{Face, FloatingWindow, Rect, Window},
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
            "<esc>" | "<Escape>" => KeyCode::Esc,
            "tab" | "<Tab>" => KeyCode::Tab,
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

    macro_rules! t {
        () => {
            ELispExp::symbol("t".into())
        };
    }

    macro_rules! primitive {
        ($func_name:ident, $args:ident, $env:ident, $ctx:ident, $body:block) => {
            pub fn $func_name<B: BufferTrait>(
                $args: &[ELispExp<B>],
                $env: std::sync::Arc<Env<EditorState<B>>>,
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

    primitive!(quit, _args, _env, ctx, {
        ctx.quit();
        Ok(nil!())
    });

    primitive!(eval_file, args, env, ctx, {
        if args.len() != 1 {
            Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: args.len(),
            })
        } else if let Some(ELispExp::String(file)) = args.first() {
            Ok(ctx.eval_file(file, env.clone())?)
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "String".into(),
                got: format!("{:?}", args.first()),
            })
        }
    });

    primitive!(define_key, args, env, ctx, {
        if args.len() != 3 {
            Err(EvalError::WrongNumberOfArguments {
                expected: 3,
                got: args.len(),
            })
        } else {
            if let (mode, ELispExp::String(key_str), ast) = (&args[0], &args[1], &args[2]) {
                let mode_name: Option<String> = match mode {
                    ELispExp::Symbol(mode_name) | ELispExp::String(mode_name) => {
                        if mode_name.as_str() == "nil" {
                            None
                        } else {
                            Some(mode_name.to_string())
                        }
                    }
                    ELispExp::List(list) => {
                        if *list == std::sync::Arc::new(vec![]) {
                            None
                        } else {
                            return Err(EvalError::WrongArgumentType {
                                expected: "Symbol, String, Symbol".into(),
                                got: format!("{:?}, {:?}, {:?}", &args[0], &args[1], &args[2]),
                            });
                        }
                    }
                    _ => {
                        return Err(EvalError::WrongArgumentType {
                            expected: "Symbol, String, Symbol".into(),
                            got: format!("{:?}, {:?}, {:?}", &args[0], &args[1], &args[2]),
                        });
                    }
                };
                let actual_ast = if let ELispExp::Symbol(symbol_name) = ast {
                    if let Some(_) = env.get_function(symbol_name) {
                        ELispExp::list(vec![ast.clone()])
                    } else {
                        ast.clone()
                    }
                } else {
                    ast.clone()
                };
                if let Some(key_event) = parse_key_sequence(key_str) {
                    if let Some(mode_name) = mode_name {
                        let mut mode_registry_lock = ctx
                            .mode_registry
                            .write()
                            .expect("Failed to acquire write lock on mode_registry");
                        if let Some(mode) = mode_registry_lock.get_mut(&mode_name) {
                            mode.keymaps.insert(key_event, actual_ast);
                            Ok(t!())
                        } else {
                            ctx.log_diagnostic(&format!(
                                "[ERROR]: define-key no mode named {mode_name}"
                            ));
                            Ok(nil!())
                        }
                    } else {
                        let mut keymaps = ctx
                            .keymaps
                            .write()
                            .expect("Failed to acquire write lock on keymaps");
                        keymaps.insert(key_event, actual_ast);
                        Ok(ELispExp::symbol("t".into()))
                    }
                } else {
                    ctx.log_diagnostic(&format!("Invalid key sequence: {}", key_str));
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

    primitive!(log, args, _env, ctx, {
        if args.len() != 1 {
            Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: args.len(),
            })
        } else {
            if let LispExp::String(message) = &args[0] {
                ctx.log_diagnostic(&message);
                Ok(nil!())
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
    //                      MAJOR MODES                           //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(make_mode, args, _env, ctx, {
        if args.len() != 1 {
            Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: args.len(),
            })
        } else {
            if let Some(LispExp::Symbol(mode_name)) = args.get(0) {
                ctx.mode_registry
                    .write()
                    .expect("Failed to acquire write lock on mode_registry")
                    .insert(mode_name.to_string(), MajorMode::new(mode_name));
                Ok(ELispExp::symbol("t".into()))
            } else {
                Err(EvalError::WrongArgumentType {
                    expected: "Symbol".into(),
                    got: format!("{:?}", args.get(0)),
                })
            }
        }
    });

    primitive!(add_hook, args, _env, ctx, {
        if args.len() != 3 {
            return Err(EvalError::WrongNumberOfArguments {
                expected: 3,
                got: args.len(),
            });
        }

        if let (
            ELispExp::Symbol(mode_name),
            ELispExp::String(hook_name),
            ELispExp::Symbol(func_name),
        ) = (&args[0], &args[1], &args[2])
        {
            let mut registry = ctx
                .mode_registry
                .write()
                .expect("Failed to acquire write lock on mode_registry");

            if let Some(mode) = registry.get_mut(mode_name.as_str()) {
                let hook_list = mode
                    .hooks
                    .entry(hook_name.to_string())
                    .or_insert_with(Vec::new);
                hook_list.push(ELispExp::symbol(func_name.to_string()));

                Ok(ELispExp::symbol("t".into()))
            } else {
                ctx.log_diagnostic(&format!("Mode {} does not exist", mode_name));
                Ok(nil!())
            }
        } else {
            Err(EvalError::WrongArgumentType {
                expected: "Symbol, String, Symbol".into(),
                got: "other".into(),
            })
        }
    });

    primitive!(add_syntax_rule, args, _env, ctx, {
        if args.len() != 3 {
            Err(EvalError::WrongNumberOfArguments {
                expected: 3,
                got: args.len(),
            })
        } else {
            if let (
                ELispExp::Symbol(mode_name),
                ELispExp::String(regex_str),
                ELispExp::Symbol(face_sym),
            ) = (&args[0], &args[1], &args[2])
            {
                let face = match face_sym.as_str() {
                    "keyword" => Face::Keyword,
                    "type" => Face::Type,
                    "string" => Face::String,
                    "comment" => Face::Comment,
                    "function" => Face::Function,
                    "builtin" => Face::Builtin,
                    face_sym_str => {
                        ctx.log_diagnostic(&format!(
                            "Unknown face: {}. Used Face::Default",
                            face_sym_str
                        ));
                        Face::Default
                    }
                };

                match regex::Regex::new(regex_str) {
                    Ok(pattern) => {
                        let mut registry = ctx
                            .mode_registry
                            .write()
                            .expect("Failed to acquire read lock on mode_registry");
                        if let Some(mode) = registry.get_mut(mode_name.as_str()) {
                            mode.syntax_rules.push(SyntaxRule { pattern, face });

                            Ok(t!())
                        } else {
                            Ok(nil!())
                        }
                    }
                    Err(e) => {
                        ctx.log_diagnostic(&format!("Invalid Regex {}: {}", regex_str, e));
                        Ok(nil!())
                    }
                }
            } else {
                Err(EvalError::WrongArgumentType {
                    expected: "Symbol, String, Symbol".into(),
                    got: format!("{:?}", args),
                })
            }
        }
    });

    //------------------------------------------------------------//
    //                                                            //
    //                          EDIT                              //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(self_insert, args, _env, ctx, {
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

    primitive!(insert_newline, _args, _env, ctx, {
        ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
            buf.text.insert('\n');
            buf.is_modified = true;
        });
        Ok(LispExp::symbol("nil".into()))
    });

    primitive!(delete_backward_char, _args, _env, ctx, {
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

    primitive!(forward_char, args, _env, ctx, {
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

    primitive!(backward_char, args, _env, ctx, {
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

    primitive!(previous_line, _args, _env, ctx, {
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

    primitive!(next_line, _args, _env, ctx, {
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

    primitive!(find_file, args, _env, ctx, {
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
            match ctx.new_buffer(&buf_name, Some(&path_str), None) {
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

    primitive!(save_buffer, _args, _env, ctx, {
        let buf = ctx.get_current_buffer();
        let mut buf = buf.write().expect("Failed to acquire write lock on buffer");
        let path = if let Some(path) = &buf.file_path {
            path.to_string()
        } else {
            ctx.log_diagnostic("No file associated with this buffer");
            return Ok(nil!());
        };
        let content = buf.text.to_string();
        match std::fs::write(&path, content) {
            Ok(_) => {
                buf.is_modified = false;
                ctx.log_diagnostic(&format!("Wrote {}", path));
                Ok(nil!())
            }
            Err(e) => {
                ctx.log_diagnostic(&format!("Failed to save: {}", e));
                Ok(nil!())
            }
        }
    });

    //------------------------------------------------------------//
    //                                                            //
    //                           UI                               //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(make_floating_window, args, _env, ctx, {
        if args.len() < 5 {
            Err(EvalError::WrongNumberOfArguments {
                expected: 5,
                got: args.len(),
            })
        } else {
            if let (
                ELispExp::String(buf_name),
                ELispExp::Number(x),
                ELispExp::Number(y),
                ELispExp::Number(w),
                ELispExp::Number(h),
            ) = (&args[0], &args[1], &args[2], &args[3], &args[4])
            {
                let title = args.get(5).and_then(|exp| {
                    if let ELispExp::String(t) = exp {
                        Some(t.to_string())
                    } else {
                        None
                    }
                });

                ctx.new_buffer(buf_name, None, None);

                let new_id = ctx.get_next_window_id();
                let window = Window {
                    id: new_id,
                    buffer_name: buf_name.to_string(),
                    scroll_x: 0,
                    scroll_y: 0,
                };

                let rect = Rect {
                    x: *x as usize,
                    y: *y as usize,
                    width: *w as usize,
                    height: *h as usize,
                };

                let floating_win = FloatingWindow {
                    window,
                    rect,
                    has_border: true,
                    title,
                };

                ctx.floating_windows
                    .write()
                    .expect("Failed to acquire write lock on floating_windows")
                    .push(floating_win);

                ctx.set_focused_window_id(new_id);
                ctx.set_current_buffer_name(buf_name);

                Ok(t!())
            } else {
                Err(EvalError::WrongArgumentType {
                    expected: "String, Number, Number, Number, Number".into(),
                    got: format!("{:?}", args),
                })
            }
        }
    });

    primitive!(close_floating_window, _args, _env, ctx, {
        // TODO(improve) this is a mess at the moment. It close the last floating opened and close
        // it not just toggle it so it has to be reopened. It must be improved to get the name of
        // the buffer or the id of the window to close or toggle. Moreover the last not floating
        // window id must be stored so when a window is closed the focus can be passed where it was.
        let mut floats = ctx
            .floating_windows
            .write()
            .expect("Failed to acquire write lock for floating_windows");
        floats.pop();
        ctx.set_focused_window_id(0);
        Ok(nil!())
    });
    //------------------------------------------------------------//
    //                                                            //
    //                        BUFFERS                             //
    //                                                            //
    //------------------------------------------------------------//

    primitive!(buffer_string, _args, _env, ctx, {
        let buf = ctx.get_current_buffer();
        let content = buf
            .read()
            .expect("Failed to acquire read lock on current buffer")
            .text
            .to_string();
        Ok(ELispExp::string(content))
    });

    primitive!(clear_buffer, _args, _env, ctx, {
        // TODO(improve) this is highly inefficent. A clear function of BufferTrait must be added
        // to clear the buffer.
        ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
            while buf.text.cursor_pos() != (0, 0) {
                buf.text.delete();
            }
        });

        Ok(nil!())
    });

    primitive!(switch_to_buffer, args, _env, ctx, {
        if args.len() != 1 {
            Err(EvalError::WrongNumberOfArguments {
                expected: 1,
                got: args.len(),
            })
        } else {
            match &args[0] {
                ELispExp::String(buffer_name) => {
                    if let Some(_) = ctx.get_buffer(buffer_name) {
                        if let Some(window) = ctx
                            .layout_root
                            .write()
                            .expect("Failed to acquire write lock on layout_root")
                            .get_window_by_id(ctx.get_focused_window_id())
                        {
                            window.buffer_name = buffer_name.to_string();
                        }
                        Ok(args[0].clone())
                    } else {
                        ctx.log_diagnostic(&format!(
                            "[LOG] buffer {} does not exist.",
                            buffer_name
                        ));
                        Ok(nil!())
                    }
                }
                ELispExp::Symbol(buffer_name) => {
                    if let Some(_) = ctx.get_buffer(buffer_name) {
                        if let Some(window) = ctx
                            .layout_root
                            .write()
                            .expect("Failed to acquire write lock on layout_root")
                            .get_window_by_id(ctx.get_focused_window_id())
                        {
                            window.buffer_name = buffer_name.to_string();
                        }
                        Ok(args[0].clone())
                    } else {
                        ctx.log_diagnostic(&format!(
                            "[LOG] buffer {} does not exist.",
                            buffer_name
                        ));
                        Ok(nil!())
                    }
                }
                _ => Err(EvalError::WrongArgumentType {
                    expected: "String".into(),
                    got: format!("{:?}", args.get(0)),
                }),
            }
        }
    });
}
