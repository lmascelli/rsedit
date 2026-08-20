use crate::{
    ELispExp,
    buffer::{Buffer, BufferTrait},
    input::{KeyEvent, fill_default_keymaps},
    lisp::{Env, EvalError, LispContext, LispExp, Parser, bootstrap_vm, eval},
    minibuffer::MinibufferState,
    modes::MajorMode,
    primitives::{buffers, edits, general, io, modes, ui},
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
    pub minibuffer: Arc<RwLock<MinibufferState>>,
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
            minibuffer: Arc::new(RwLock::new(MinibufferState::default())),
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
    pub(crate) fn quit(&self) {
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
    pub(crate) fn get_next_window_id(&self) -> usize {
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
    pub(crate) fn get_current_buffer_name(&self) -> String {
        self.current_buffer_name
            .read()
            .expect("Failed to acquire read lock on current_buffer_name")
            .to_string()
    }

    /// Set the name of the current buffer
    pub(crate) fn set_current_buffer_name(&self, name: &str) {
        *self
            .current_buffer_name
            .write()
            .expect("Failed to acquire write lock on current_buffer_name") = name.to_string();
    }

    /// Returns an Arc reference to the current buffer
    pub(crate) fn get_current_buffer(&self) -> Arc<RwLock<Buffer<B>>> {
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
        ($name:literal, $func:path) => {
            env.set_function($name.into(), LispExp::primitive($func, None));
        };
        ($name:literal, $func:path, $doc:literal) => {
            env.set_function(
                $name.into(),
                LispExp::primitive($func, Some($doc.into())),
            );
        };
    }

    insert_fn!(
        "quit",
        general::quit,
        "(quit): Stop the editor's main loop.\n\n\
         Example:\n\
         (define-key nil \"C-x C-c\" 'quit)"
    );
    insert_fn!(
        "eval-file",
        general::eval_file,
        "(eval-file FILE): Evaluate FILE as Lisp. FILE is first looked up as \
         an absolute or relative path; if that fails and FILE has no \
         directory separator or extension, each directory in `lisp-path` is \
         tried in order for FILE.lisp. Returns a one-element list holding the \
         result of evaluating the file's contents (wrapped in an implicit \
         `progn`), or nil (logging a diagnostic) if no matching file was \
         found.\n\n\
         Example:\n\
         (eval-file \"common-keymaps\") ; loads <lisp-path>/common-keymaps.lisp\n\
         (eval-file \"/home/me/.config/rsedit/extra.lisp\")"
    );
    insert_fn!(
        "define-key",
        general::define_key,
        "(define-key MODE KEY COMMAND): Bind KEY (an Emacs-style key sequence \
         string, e.g. \"C-x\" or \"<ret>\") to COMMAND. MODE is either nil, \
         binding KEY globally, or a mode name symbol/string, binding KEY only \
         in that major mode. COMMAND may be a symbol naming a function (which \
         is wrapped so it is called with no arguments) or an arbitrary \
         expression to evaluate. Returns t on success, nil (logging a \
         diagnostic) if MODE names an unknown mode or KEY isn't a recognized \
         key sequence.\n\n\
         Example:\n\
         (define-key nil \"C-n\" 'next-line)          ; global binding\n\
         (define-key 'lisp-mode \"<ret>\" 'insert-newline) ; mode-local binding\n\
         (define-key nil \"C-x C-s\" '(save-buffer))  ; arbitrary expression"
    );
    insert_fn!(
        "log",
        general::log,
        "(log MESSAGE): Append the string MESSAGE to the editor's diagnostic \
         log (and to the log file, if one is enabled).\n\n\
         Example:\n\
         (log \"buffer saved\")"
    );
    insert_fn!(
        "make-mode",
        modes::make_mode,
        "(make-mode NAME): Register a new, empty major mode named NAME (a \
         symbol) with no keymaps, hooks, or syntax rules of its own. Returns \
         t. rsedit's own simplified alternative to Emacs's \
         `define-derived-mode`.\n\n\
         Example:\n\
         (make-mode 'lisp-mode)\n\
         (add-syntax-rule 'lisp-mode \"defun\" 'keyword)"
    );
    insert_fn!(
        "add-hook",
        modes::add_hook,
        "(add-hook MODE HOOK FUNCTION): Append the function named FUNCTION to \
         the list of functions run for HOOK (a string, e.g. \
         \"post-command-hook\") in major mode MODE. Returns t, or nil \
         (logging a diagnostic) if MODE names an unknown mode. Unlike real \
         Emacs Lisp's `add-hook`, hooks here are scoped to a single major \
         mode rather than being global variables.\n\n\
         Example:\n\
         (defun my-mode-hook () (log \"entered my-mode\"))\n\
         (add-hook 'my-mode \"post-command-hook\" 'my-mode-hook)"
    );
    insert_fn!(
        "add-syntax-rule",
        modes::add_syntax_rule,
        "(add-syntax-rule MODE REGEX FACE): Add a syntax-highlighting rule to \
         major mode MODE: any text matching the regular expression REGEX (a \
         string) is rendered with FACE (a symbol: keyword, type, string, \
         comment, function, or builtin -- anything else falls back to the \
         default face). Returns t, or nil (logging a diagnostic) if MODE is \
         unknown or REGEX fails to compile. Not a standard Elisp primitive.\n\n\
         Example:\n\
         (add-syntax-rule 'lisp-mode \"\\\\bdefun\\\\b\" 'keyword)\n\
         (add-syntax-rule 'lisp-mode \"\\\";.*\\\"\" 'comment)"
    );
    insert_fn!(
        "self-insert",
        edits::self_insert,
        "(self-insert STRING): Insert the first character of STRING at point \
         in the current buffer. Unlike Emacs's `self-insert-command`, which \
         reads the character to insert from `last-command-event`, this takes \
         the character explicitly as an argument.\n\n\
         Example:\n\
         (self-insert \"a\") ; inserts the character a at point"
    );
    insert_fn!(
        "insert-newline",
        edits::insert_newline,
        "(insert-newline): Insert a newline character at point in the current \
         buffer.\n\n\
         Example:\n\
         (define-key nil \"<ret>\" 'insert-newline)"
    );
    insert_fn!(
        "delete-backward-char",
        edits::delete_backward_char,
        "(delete-backward-char): Delete the character before point in the \
         current buffer. Unlike Emacs's command of the same name, this takes \
         no count argument -- it always deletes exactly one character.\n\n\
         Example:\n\
         (define-key nil \"<backspace>\" 'delete-backward-char)"
    );
    insert_fn!(
        "backward-char",
        edits::backward_char,
        "(backward-char &optional N): Move point backward N characters \
         (default 1) in the current buffer, stopping at the beginning of the \
         line.\n\n\
         Example:\n\
         (backward-char)   ; move back 1 character\n\
         (backward-char 4) ; move back 4 characters"
    );
    insert_fn!(
        "forward-char",
        edits::forward_char,
        "(forward-char &optional N): Move point forward N characters (default \
         1) in the current buffer.\n\n\
         Example:\n\
         (forward-char)   ; move forward 1 character\n\
         (forward-char 4) ; move forward 4 characters"
    );
    insert_fn!(
        "previous-line",
        edits::previous_line,
        "(previous-line): Move point up one line in the current buffer, \
         keeping the same column (clamped to that line's length), stopping at \
         the first line.\n\n\
         Example:\n\
         (define-key nil \"<up>\" 'previous-line)"
    );
    insert_fn!(
        "next-line",
        edits::next_line,
        "(next-line): Move point down one line in the current buffer, keeping \
         the same column.\n\n\
         Example:\n\
         (define-key nil \"<down>\" 'next-line)"
    );
    insert_fn!(
        "find-file",
        io::find_file,
        "(find-file PATH): Open the file at PATH into a new buffer named \
         after PATH's file name (or PATH itself if it has none), make it the \
         current buffer, and return its buffer name. Returns nil (logging a \
         diagnostic) if the file can't be read.\n\n\
         Example:\n\
         (find-file \"/home/me/notes.txt\") => \"notes.txt\""
    );
    insert_fn!(
        "save-buffer",
        io::save_buffer,
        "(save-buffer): Write the current buffer's contents to the file it \
         was visiting. Returns nil in every case (logging a diagnostic \
         either way); if the buffer has no associated file, or the write \
         fails, nothing is written.\n\n\
         Example:\n\
         (define-key nil \"C-x C-s\" 'save-buffer)"
    );
    insert_fn!(
        "make-floating-window",
        ui::make_floating_window,
        "(make-floating-window BUFFER-NAME X Y WIDTH HEIGHT &optional TITLE): \
         Create a new buffer named BUFFER-NAME, open it in a new bordered \
         floating window positioned at (X, Y) with the given WIDTH and \
         HEIGHT (and optional TITLE string), give that window focus, and \
         return t. Not a standard Elisp primitive.\n\n\
         Example:\n\
         (make-floating-window \"*Minibuffer*\" 0 20 80 1 \"Find file\")"
    );
    insert_fn!("close-floating-window", ui::close_floating_window);
    insert_fn!(
        "switch-to-buffer",
        buffers::switch_to_buffer,
        "(switch-to-buffer BUFFER-NAME): Make the buffer named BUFFER-NAME \
         (a string or symbol) the one shown in the focused window, and \
         return BUFFER-NAME. Returns nil (logging a diagnostic) if no buffer \
         with that name exists -- unlike real Emacs Lisp's \
         `switch-to-buffer`, this does not create one.\n\n\
         Example:\n\
         (switch-to-buffer \"*scratch*\") => \"*scratch*\""
    );
    insert_fn!(
        "current-buffer",
        buffers::current_buffer,
        "(current-buffer): Return the name of the current buffer, as a \
         string. Unlike real Emacs Lisp's `current-buffer`, which returns a \
         buffer object, this returns the buffer's name.\n\n\
         Example:\n\
         (current-buffer) => \"*scratch*\""
    );
    insert_fn!("close-buffer", buffers::close_buffer);
    insert_fn!(
        "buffer-string",
        buffers::buffer_string,
        "(buffer-string): Return the entire contents of the current buffer as \
         a string.\n\n\
         Example:\n\
         (buffer-string) => \"line one\\nline two\\n\""
    );
    insert_fn!(
        "clear-buffer",
        buffers::clear_buffer,
        "(clear-buffer): Delete the entire contents of the current buffer. \
         Not a standard Elisp primitive -- comparable to Emacs's \
         `erase-buffer`.\n\n\
         Example:\n\
         (clear-buffer)\n\
         (buffer-string) => \"\""
    );

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
