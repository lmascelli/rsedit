use crate::{
    ELispExp,
    buffer::{Buffer, BufferTrait},
    input::{KeyEvent, fill_default_keymaps},
    lisp::{Env, EvalError, LispContext, Parser, bootstrap_vm, eval},
    minibuffer::install_minibuffer,
    modes::MajorMode,
    primitives::install_primitives,
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
    /// The call stack, as maintained by `LispContext::push_call_frame` /
    /// `pop_call_frame` (see their docs for the exact protocol). Frozen at
    /// its state at the moment of the most recent uncaught error until
    /// something calls `clear_backtrace` -- typically whoever caught that
    /// error, once it's done reporting it.
    call_stack: Arc<RwLock<Vec<String>>>,
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

    fn push_call_frame(&self, frame: &str) {
        self.call_stack
            .write()
            .expect("Failed to acquire write lock on call_stack")
            .push(frame.to_string());
    }

    fn pop_call_frame(&self) {
        self.call_stack
            .write()
            .expect("Failed to acquire write lock on call_stack")
            .pop();
    }

    fn call_frame_depth(&self) -> usize {
        self.call_stack
            .read()
            .expect("Failed to acquire read lock on call_stack")
            .len()
    }

    fn truncate_call_frames(&self, depth: usize) {
        self.call_stack
            .write()
            .expect("Failed to acquire write lock on call_stack")
            .truncate(depth);
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
            call_stack: Arc::new(RwLock::new(Vec::new())),
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
            let mut new_buf = Buffer::new(name);
            if let Some(mode_name) = start_mode {
                new_buf.current_mode = mode_name;
            }
            let mut buffers_lock = self
                .buffers
                .write()
                .expect("Failed to get write lock on buffers");
            buffers_lock.insert(name.to_string(), Arc::new(RwLock::new(new_buf)));
            Some(name.to_string())
        }
    }

    /// Make the buffer named NAME the one shown in the focused window and
    /// the current buffer. Returns `false` (logging a diagnostic) if no
    /// buffer named NAME exists, `true` otherwise. Shared by the
    /// `switch-to-buffer` primitive and the built-in minibuffer's cleanup.
    pub(crate) fn switch_to_buffer(&self, name: &str) -> bool {
        if self.get_buffer(name).is_none() {
            self.log_diagnostic(&format!("[LOG] buffer {} does not exist.", name));
            return false;
        }
        if let Some(window) = self
            .layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root")
            .get_window_by_id(self.get_focused_window_id())
        {
            window.buffer_name = name.to_string();
        }
        self.set_current_buffer_name(name);
        true
    }

    /// Create a new buffer named BUF_NAME (in major mode MODE, defaulting
    /// to fundamental-mode) and open it in a new bordered floating window
    /// at (X, Y) with the given WIDTH/HEIGHT and optional TITLE, giving
    /// that window focus. Closing the floating window (`close-buffer` or
    /// `close-floating-window`) restores focus to whatever window was
    /// focused before this call. Shared by the `make-floating-window`
    /// primitive and the built-in minibuffer, so both open a floating
    /// window exactly the same way.
    pub(crate) fn open_floating_window(
        &self,
        buf_name: &str,
        x: isize,
        y: isize,
        width: usize,
        height: usize,
        title: Option<String>,
        mode: Option<String>,
    ) {
        let previous_focused_window_id = self.get_focused_window_id();
        self.new_buffer(buf_name, None, mode);

        let new_id = self.get_next_window_id();
        let window = Window {
            id: new_id,
            buffer_name: buf_name.to_string(),
            scroll_x: 0,
            scroll_y: 0,
        };

        let rect = Rect {
            x,
            y,
            width,
            height,
        };

        let floating_win = FloatingWindow {
            window,
            rect,
            has_border: true,
            title,
            previous_focused_window_id,
        };

        self.floating_windows
            .write()
            .expect("Failed to acquire write lock on floating_windows")
            .push(floating_win);

        self.set_focused_window_id(new_id);
        self.set_current_buffer_name(buf_name);
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
            self.report_error(&format!("{:?} {:?}", ast, e), env);
            return;
        }

        // Handle the post-command hooks
        let current_mode_name = {
            let buf_arc = self.get_current_buffer();
            let buf_lock = buf_arc
                .read()
                .expect("Failed to acquire read lock on current buffer");
            buf_lock.current_mode.clone()
        };
        self.run_hook(&current_mode_name, "post-command-hook", env);
    }

    /// Run every function registered under HOOK_NAME in the major mode
    /// named MODE_NAME, in registration order, each called with zero
    /// arguments. Errors from an individual hook are logged and
    /// otherwise swallowed, so one broken hook can't block the rest.
    pub(crate) fn run_hook(
        &self,
        mode_name: &str,
        hook_name: &str,
        env: &Arc<Env<EditorState<B>>>,
    ) {
        let registry = self
            .mode_registry
            .read()
            .expect("Failed to acquire read lock on mode registry");
        if let Some(mode) = registry.get(mode_name) {
            if let Some(hook_vec) = mode.hooks.get(hook_name) {
                for hook in hook_vec {
                    let hook_call = ELispExp::list(vec![hook.clone()]);
                    if let Err(e) = eval(&hook_call, env.clone(), self) {
                        self.log_diagnostic(&format!(
                            "Hook {hook_name} ({:?}) execution failed: {:?}",
                            hook, e
                        ));
                    }
                }
            }
        }
    }

    /// Close the buffer named NAME: detach it from whatever window is
    /// showing it (a floating window is removed outright and focus
    /// returns to whatever had it before that floating window opened;
    /// a tiled window falls back to `*scratch*`, since there's no
    /// per-window buffer history to fall back to yet), run that
    /// buffer's major mode's `after-close-hook`, then remove it from
    /// the buffer table. If NAME was the last remaining buffer, a fresh
    /// empty `*scratch*` is created so the editor is never left with
    /// none. Returns `false` if no buffer named NAME exists, `true`
    /// otherwise.
    pub fn close_buffer(&self, name: &str, env: &Arc<Env<EditorState<B>>>) -> bool {
        let Some(buffer) = self.get_buffer(name) else {
            return false;
        };
        let closing_mode = buffer
            .read()
            .expect("Failed to acquire read lock on buffer")
            .current_mode
            .clone();

        // Detach NAME from wherever it's currently displayed.
        let floating_match = {
            let floats = self
                .floating_windows
                .read()
                .expect("Failed to acquire read lock on floating_windows");
            floats.iter().position(|f| f.window.buffer_name == name)
        };
        if let Some(idx) = floating_match {
            let restore_id = self
                .floating_windows
                .write()
                .expect("Failed to acquire write lock on floating_windows")
                .remove(idx)
                .previous_focused_window_id;
            self.set_focused_window_id(restore_id);
        } else if let Some(window) = self
            .layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root")
            .get_window_by_id(self.get_focused_window_id())
        {
            if window.buffer_name == name {
                window.buffer_name = "*scratch*".into();
            }
        }

        {
            let mut buffers = self
                .buffers
                .write()
                .expect("Failed to acquire write lock on buffers");
            buffers.remove(name);
            if buffers.is_empty() {
                buffers.insert(
                    "*scratch*".to_string(),
                    Arc::new(RwLock::new(Buffer::new("*scratch*"))),
                );
            }
        }

        if self.get_current_buffer_name() == name
            || self.get_buffer(&self.get_current_buffer_name()).is_none()
        {
            self.set_current_buffer_name("*scratch*");
        }

        self.run_hook(&closing_mode, "after-close-hook", env);
        true
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

    pub fn set_mode(&self, mode_name: &str, mode: MajorMode<B>) {
        self.mode_registry
            .write()
            .expect("Failed to acquire write lock on mode_registry")
            .insert(mode_name.to_string(), mode);
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

    /// Return every diagnostic logged so far via `log_diagnostic`, oldest
    /// first.
    pub fn get_logs(&self) -> Vec<String> {
        self.logs
            .read()
            .expect("Failed to acquire read lock on logs")
            .clone()
    }

    /// Return the call stack captured at the point of the most recent
    /// uncaught error, innermost (deepest) call first -- or an empty list
    /// if nothing has errored since the last `clear_backtrace`. See
    /// `LispContext::push_call_frame` for the capture protocol and its
    /// tail-call caveat.
    pub fn backtrace(&self) -> Vec<String> {
        let mut frames = self
            .call_stack
            .read()
            .expect("Failed to acquire read lock on call_stack")
            .clone();
        frames.reverse();
        frames
    }

    /// Discard the captured backtrace, so the next error starts from a
    /// clean stack instead of stacking on top of a stale one. Callers that
    /// catch and report an error (a key handler, `eval_file`, ...) should
    /// call this once they're done reading `backtrace()`.
    pub fn clear_backtrace(&self) {
        self.call_stack
            .write()
            .expect("Failed to acquire write lock on call_stack")
            .clear();
    }

    /// Convenience for error-reporting call sites: returns a
    /// `" | backtrace: a -> b -> c"` suffix (innermost call first)
    /// describing the frames captured at the point of the most recent
    /// uncaught error, or an empty string if there's nothing to report --
    /// and clears the captured backtrace either way, so the next error
    /// starts from a clean stack.
    pub fn take_backtrace_suffix(&self) -> String {
        let frames = self.backtrace();
        self.clear_backtrace();
        if frames.is_empty() {
            String::new()
        } else {
            format!(" | backtrace: {}", frames.join(" -> "))
        }
    }

    /// Report an uncaught evaluation error to the user. Always logs it. If
    /// the user's Lisp configuration defines a `report-error` function
    /// (see `core/lisp/debug.lisp`), hands it MESSAGE and the call stack
    /// captured at the point of failure (see `backtrace`) as `(report-error
    /// MESSAGE FRAMES)`, so Lisp decides how to present it -- the default
    /// implementation echoes it, and additionally opens a *Backtrace*
    /// popup if `debug-on-error` is set. Falls back to plain logging (the
    /// same shape `take_backtrace_suffix` produces) if no such hook is
    /// defined yet, e.g. during early boot before `debug.lisp` has loaded.
    pub fn report_error(&self, message: &str, env: &Arc<Env<Self>>) {
        let frames = self.backtrace();
        self.clear_backtrace();

        if env.get_function("report-error").is_some() {
            let call_ast = ELispExp::list(vec![
                ELispExp::symbol("report-error".into()),
                ELispExp::string(message.to_string()),
                ELispExp::list(vec![
                    ELispExp::symbol("quote".into()),
                    ELispExp::list(frames.into_iter().map(ELispExp::string).collect()),
                ]),
            ]);
            if let Err(e) = eval(&call_ast, env.clone(), self) {
                self.log_diagnostic(&format!("[ERROR] report-error hook itself failed: {:?}", e));
            }
        } else {
            let suffix = if frames.is_empty() {
                String::new()
            } else {
                format!(" | backtrace: {}", frames.join(" -> "))
            };
            self.log_diagnostic(&format!("Eval Error: {message}{suffix}"));
        }
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

    // ---------------------- FILLING PRIMITIVE FUNCTIONS -----------------------------
    install_primitives(&env);
    install_minibuffer(&editor_state, env.clone());

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
(eval-file "debug")
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
