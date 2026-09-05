use crate::{
    ELispExp,
    buffer::{Buffer, BufferTrait},
    input::{KeyEvent, fill_default_keymaps},
    lisp::{
        DEFAULT_FUEL, Env, EvalError, FuelMeter, FuelScope, LispContext, Parser, bootstrap_vm, eval,
    },
    minibuffer::install_minibuffer,
    modes::MajorMode,
    primitives::install_primitives,
    commands::{ArgSpec, CommandRegistry, PendingCommand},
    task::{BackgroundScheduler, WorkerMessage},
    ui::{
        FloatingWindow, FrameSnapshot, LayoutNode, Rect, RenderableWindowView, Window,
        extract_buffer_lines,
    },
};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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

    /// Which named functions the user may invoke by name, and what arguments
    /// the editor collects for each. See `crate::commands`.
    commands: Arc<RwLock<CommandRegistry>>,

    /// Commands whose arguments are still being collected, innermost last.
    /// See `crate::commands::PendingCommand`.
    pending_commands: Arc<RwLock<Vec<PendingCommand<ELispExp<B>>>>>,

    /// Execution budget for Lisp evaluation.
    fuel: Arc<FuelMeter>,
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
    fn consume_fuel(&self, amount: u32) -> Result<(), EvalError<EditorState<B>>> {
        // The meter reports a host-agnostic `Exhausted`; naming it as a Lisp
        // error is the host's job, which is the point of the split.
        self.fuel.consume(amount).map_err(|_| EvalError::OutOfFuel)
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

    fn begin_thread_evaluation(&self) {
        self.fuel.arm_thread();
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
            commands: Arc::new(RwLock::new(CommandRegistry::new())),
            pending_commands: Arc::new(RwLock::new(Vec::new())),
            fuel: Arc::new(FuelMeter::new(DEFAULT_FUEL)),
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
    pub fn eval_file(
        &self,
        file: &str,
        env: Arc<Env<Self>>,
    ) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
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
                            return Ok(ELispExp::nil());
                        } else {
                            // search for file.lisp in every lisp-path folder
                            // 1. get the lisp-path lists, and check it is a list of strings
                            let mut lisp_path = vec![];
                            if let Some(lisp_path_list) = env.get_variable("lisp-path") {
                                if matches!(lisp_path_list, ELispExp::Cons(_)) {
                                    for ipath in lisp_path_list.iter() {
                                        if let ELispExp::String(path) = &ipath {
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
                                return Ok(ELispExp::nil());
                            }
                        }
                    } else {
                        self.log_diagnostic(&format!("[ERROR] Failed eval file {file} {:?}", err));
                        return Ok(ELispExp::nil());
                    }
                }
            }
        );

        let ast = if let Ok(ast) = Parser::new(&content).next() {
            ast
        } else {
            return Ok(ELispExp::nil());
        };

        let _command = self.begin_command();
        Ok(ELispExp::proper_list(vec![eval(&ast, env.clone(), self)?]))
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
        let mut ast = ELispExp::form(vec![]);
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
                ast = ELispExp::form(vec![ELispExp::symbol("funcall".into()), ast]);
            }
        }

        // If no symbol has been found
        if !keymap_found {
            self.log_diagnostic(&format!("[INFO] Keymap not bound {:?}", event));
            return;
        };

        // A key bound to a bare command invocation -- `(next-line)`, as
        // `define-key` stores a symbol -- is routed through
        // `call-interactively` so that the editor collects whatever arguments
        // the command declared. Without this a command taking arguments simply
        // cannot be bound to a key: `find-file` needs a path, and a keystroke
        // has none to give it, which is why it had no binding at all.
        //
        // Routing every such binding through one Lisp entry point, rather than
        // only those that need arguments, is deliberate: it is the single
        // place to observe or advise command execution, which is what a macro
        // recorder or a `repeat` command would later hook.
        //
        // A binding that already supplies its arguments -- `(self-insert "a")`,
        // which is every ordinary keystroke -- is left exactly as it was, so
        // the typing path is untouched and still costs what it always did.
        // A binding may name its command either way: `define-key` wraps a
        // symbol into a one-element form, while the keymaps built in Rust
        // (`install_minibuffer`) store the bare symbol. Both mean "run this
        // command".
        //
        // The bare-symbol case was previously unreachable code: evaluating a
        // symbol looks up a *variable*, so Enter, Escape and Tab in the
        // minibuffer all failed with `UnboundVariable` in the real editor. No
        // test caught it because they all call `(minibuffer-confirm)` through
        // `eval` rather than pressing the key.
        let bound_command = match &ast {
            ELispExp::Symbol(name) => Some(name.to_string()),
            ELispExp::Form(items) if items.len() == 1 => match &items[0] {
                ELispExp::Symbol(name) => Some(name.to_string()),
                _ => None,
            },
            _ => None,
        };
        if let Some(name) = bound_command {
            // The name is passed as a string rather than a quoted symbol:
            // a string literal is self-evaluating, so this needs no `quote`
            // and cannot be mistaken for a variable reference.
            ast = ELispExp::form(vec![
                ELispExp::symbol("call-interactively".into()),
                ELispExp::string(name),
            ]);
        }

        let outcome = {
            let _command = self.begin_command();
            eval(&ast, env.clone(), self)
        };
        if let Err(e) = outcome {
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
        // Copy the hook list out and release the registry *before* evaluating
        // any of it.
        //
        // Holding the lock across `eval` deadlocks the editor outright: a hook
        // that calls `add-hook`, `define-key` with a mode, `make-mode` or
        // `add-syntax-rule` needs a write lock on the same registry this thread
        // is already reading, and `RwLock` is not reentrant. It is not a race --
        // it hangs every time, on one thread, from ordinary user Lisp.
        //
        // The general rule this is an instance of: never hold a lock across a
        // callback into the interpreter. Lisp can re-enter the editor through
        // any primitive, so a lock held across `eval` is a lock offered to
        // arbitrary code.
        let hooks: Vec<ELispExp<B>> = self
            .mode_registry
            .read()
            .expect("Failed to acquire read lock on mode registry")
            .get(mode_name)
            .and_then(|mode| mode.hooks.get(hook_name))
            .cloned()
            .unwrap_or_default();

        for hook in hooks {
            let hook_call = ELispExp::form(vec![hook.clone()]);
            let _command = self.begin_command();
            if let Err(e) = eval(&hook_call, env.clone(), self) {
                self.log_diagnostic(&format!(
                    "Hook {hook_name} ({:?}) execution failed: {:?}",
                    hook, e
                ));
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
    /// Capture everything the UI needs to draw one frame.
    ///
    /// # Lock order
    ///
    /// This is the **canonical order** for the whole editor. It is currently
    /// the only code path that holds more than one of these at a time, so it
    /// gets to define the order; anything added later that needs two or more
    /// must take them in this sequence or risk a deadlock the moment a second
    /// thread writes:
    ///
    /// `focused_window_id` -> `echo_message` -> `layout_root` ->
    /// `floating_windows` -> `buffers` -> an individual `Buffer`
    ///
    /// Cheap scalars first so the structural locks are held for as short a
    /// time as possible. Every lock is acquired exactly once and released
    /// before the caller sees the result, so no terminal I/O ever happens with
    /// a lock held.
    ///
    /// # Why one capture rather than field-by-field reads
    ///
    /// See [`FrameSnapshot`]. The short version: `BackgroundScheduler` and
    /// `(spawn ...)` already mutate this state from other threads, so a
    /// renderer that reads six locks at six different instants can compose a
    /// frame that never existed.
    ///
    /// # Note on `layout_root`
    ///
    /// A *write* lock, because `compute_tiled_views` adjusts each window's
    /// `scroll_x`/`scroll_y` to keep the cursor in view -- rendering mutates
    /// editor state. That works while one thread renders, and is the thing to
    /// untangle before the UI loop moves off the command thread: hoisting the
    /// scroll reconciliation into an explicit post-command step would let this
    /// take a read lock and let renders run concurrently.
    pub fn snapshot(&self, screen_width: usize, screen_height: usize) -> FrameSnapshot {
        let focused_window_id = *self
            .focused_window_id
            .read()
            .expect("Failed to acquire read lock on focused_window_id");
        let echo_message = self
            .echo_message
            .read()
            .expect("Failed to acquire read lock on echo_message")
            .clone();
        let mut layout_root = self
            .layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root");
        let floating_windows = self
            .floating_windows
            .read()
            .expect("Failed to acquire read lock on floating_windows");
        let buffers = self
            .buffers
            .read()
            .expect("Failed to acquire read lock on buffers");

        let mut views = Vec::new();
        layout_root.compute_tiled_views(
            Rect {
                x: 0,
                y: 0,
                width: screen_width,
                height: screen_height,
            },
            focused_window_id,
            &buffers,
            &mut views,
        );

        for float in floating_windows.iter() {
            let is_focused = float.window.id == focused_window_id;
            // Unlike a tiled window, a float is not auto-scrolled to follow the
            // cursor; its scroll offsets are whatever whoever opened it set.
            let cursor_rel_pos = is_focused
                .then(|| buffers.get(&float.window.buffer_name))
                .flatten()
                .map(|buf| {
                    let (c_line, c_col) = buf
                        .read()
                        .expect("Failed to acquire read lock on buffer")
                        .text
                        .cursor_pos();
                    (
                        c_col.saturating_sub(float.window.scroll_x),
                        c_line.saturating_sub(float.window.scroll_y),
                    )
                });

            views.push(RenderableWindowView {
                rect: float.rect.clone(),
                buffer_name: float.window.buffer_name.clone(),
                is_focused,
                cursor_rel_pos,
                lines: extract_buffer_lines(&float.window, &float.rect, &buffers),
                has_border: float.has_border,
            });
        }

        FrameSnapshot {
            views,
            echo_message,
            focused_window_id,
            width: screen_width,
            height: screen_height,
        }
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
        if let Some(callback_list) = env.get_variable("after-resize-hook") {
            for el in callback_list.iter() {
                let _command = self.begin_command();
                match &el {
                    ELispExp::Lambda(_) | ELispExp::Symbol(_) => {
                        if let Err(err) = eval(
                            &ELispExp::form(vec![
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
            let call_ast = ELispExp::form(vec![
                ELispExp::symbol("report-error".into()),
                ELispExp::string(message.to_string()),
                ELispExp::form(vec![
                    ELispExp::symbol("quote".into()),
                    ELispExp::proper_list(frames.into_iter().map(ELispExp::string).collect()),
                ]),
            ]);
            let _command = self.begin_command();
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

    /// Open a fresh Lisp execution budget for one top-level command -- a
    /// keystroke, a hook run, a config file being loaded.
    ///
    /// What counts as "one command" is editor *policy*, which is why this lives
    /// here rather than in `FuelMeter`: only the editor knows a keystroke is one
    /// unit of work. Nesting is safe -- the meter tracks depth and only the
    /// outermost scope refills -- so a command that re-enters the evaluator, via
    /// the Lisp-callable `eval-file` primitive for instance, keeps spending the
    /// budget it already has instead of quietly being handed a new one.
    pub(crate) fn begin_command(&self) -> FuelScope<'_> {
        self.fuel.begin()
    }

    /// The execution meter behind [`Self::begin_command`].
    ///
    /// Exposed for `lisp::measure`, which needs the meter to hold a scope of
    /// its own for the duration of a measurement.
    pub(crate) fn fuel_meter(&self) -> &FuelMeter {
        &self.fuel
    }

    // ---------------------------------------------------------------
    // Argument collection for a command in flight
    // ---------------------------------------------------------------

    /// Begin collecting arguments for NAME.
    pub(crate) fn push_pending_command(&self, name: String, remaining: Vec<ArgSpec>) {
        self.pending_commands
            .write()
            .expect("Failed to acquire write lock on pending_commands")
            .push(PendingCommand::new(name, remaining));
    }

    /// The argument the innermost pending command is waiting on.
    pub(crate) fn pending_current_spec(&self) -> Option<ArgSpec> {
        self.pending_commands
            .read()
            .expect("Failed to acquire read lock on pending_commands")
            .last()
            .and_then(|pending| pending.current().cloned())
    }

    /// Record VALUE as the innermost pending command's next argument, and
    /// report what it still needs: `Some(spec)` to prompt for, or `None` when
    /// it is complete -- in which case the entry is removed and its name and
    /// arguments are returned by [`Self::take_pending_command`].
    pub(crate) fn accept_pending_arg(&self, value: ELispExp<B>) -> Option<ArgSpec> {
        let mut stack = self
            .pending_commands
            .write()
            .expect("Failed to acquire write lock on pending_commands");
        let pending = stack.last_mut()?;
        if !pending.remaining.is_empty() {
            pending.remaining.remove(0);
        }
        pending.collected.push(value);
        pending.current().cloned()
    }

    /// Remove and return the innermost pending command.
    pub(crate) fn take_pending_command(&self) -> Option<(String, Vec<ELispExp<B>>)> {
        self.pending_commands
            .write()
            .expect("Failed to acquire write lock on pending_commands")
            .pop()
            .map(|pending| (pending.name, pending.collected))
    }

    /// Drop every pending command.
    ///
    /// Called when a fresh command starts with no minibuffer open, which means
    /// any entry still on the stack belongs to a prompt that was closed by some
    /// path other than confirm or cancel. Without this, that orphan would be
    /// fed the *next* command's input.
    pub(crate) fn clear_pending_commands(&self) {
        self.pending_commands
            .write()
            .expect("Failed to acquire write lock on pending_commands")
            .clear();
    }

    /// Whether a minibuffer prompt is currently open.
    pub(crate) fn minibuffer_is_open(&self) -> bool {
        self.buffers
            .read()
            .expect("Failed to acquire read lock on buffers")
            .contains_key("*Minibuffer*")
    }

    /// Every live buffer's name, sorted. Used for buffer-name completion.
    pub(crate) fn buffer_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .buffers
            .read()
            .expect("Failed to acquire read lock on buffers")
            .keys()
            .cloned()
            .collect();
        names.sort_unstable();
        names
    }

    /// Register NAME as a command taking SPECS.
    ///
    /// Idempotent by name: re-registering replaces the previous specs, so a
    /// user can change how an existing command prompts without restarting.
    pub(crate) fn register_command(&self, name: &str, specs: Vec<ArgSpec>) {
        self.commands
            .write()
            .expect("Failed to acquire write lock on commands")
            .insert(name.to_string(), specs);
    }

    /// The arguments to collect for NAME, or `None` if it is not a command.
    ///
    /// Returns owned data, and every other accessor here does too. That is not
    /// incidental: `call-interactively` looks a command up and then evaluates
    /// Lisp, and Lisp can call `register-command`. Handing back a guard would
    /// mean holding this lock across `eval` -- exactly the reentrancy that
    /// deadlocked `run_hook`.
    pub(crate) fn command_specs(&self, name: &str) -> Option<Vec<ArgSpec>> {
        self.commands
            .read()
            .expect("Failed to acquire read lock on commands")
            .get(name)
            .cloned()
    }

    pub(crate) fn is_command(&self, name: &str) -> bool {
        self.commands
            .read()
            .expect("Failed to acquire read lock on commands")
            .contains_key(name)
    }

    /// Every command name, sorted, for M-x completion.
    pub(crate) fn command_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .commands
            .read()
            .expect("Failed to acquire read lock on commands")
            .keys()
            .cloned()
            .collect();
        names.sort_unstable();
        names
    }

    /// Set how much fuel a fresh command receives, and top the current thread's
    /// remaining fuel up to it. Exposed so the `set-command-fuel` primitive --
    /// and tests that want a deliberately tiny budget -- can reach it.
    pub(crate) fn set_fuel_budget(&self, budget: u32) {
        self.fuel.set_budget(budget);
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
-> Result<(EditorState<B>, Arc<Env<EditorState<B>>>), EvalError<EditorState<B>>> {
    let editor_state = EditorState::new();
    let env = bootstrap_vm(&editor_state)?;

    // ---------------------- EDITOR ENVIRONMENT CONFIGURATION ----------------------

    // Add the rsedit std lisp sources to *lisp-path*
    match std::env::current_exe() {
        Ok(exe_path) => {
            env.set_variable(
                "lisp-path".into(),
                ELispExp::proper_list(vec![ELispExp::string(format!(
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
    env.set_variable("after-resize-hook".into(), ELispExp::nil());

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
    install_primitives(&editor_state, &env);
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
