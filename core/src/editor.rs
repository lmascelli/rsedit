use crate::{
    ELispExp,
    buffer::{Buffer, BufferTrait},
    input::{KeyCode, KeyEvent, Keymap, describe_keys, fill_default_keymaps},
    kill_ring::{Direction, KillRing},
    lisp::{
        DEFAULT_FUEL, Env, EvalError, FuelMeter, FuelScope, LispContext, Parser, bootstrap_vm, eval,
    },
    minibuffer::install_minibuffer,
    modes::MajorMode,
    primitives::install_primitives,
    commands::{ArgSpec, CommandRegistry, Invocation, PendingCommand, PrefixArg},
    task::{BackgroundScheduler, WorkerMessage},
    ui::{
        Face, FloatingWindow, FrameSnapshot, LayoutNode, Rect, RenderableWindowView, Style, Theme,
        Window, extract_buffer_lines, region_highlights,
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
    time::{Duration, Instant},
};

/// The name of the Lisp variable that arms the echo area's timeout.
pub const ECHO_MESSAGE_TIMEOUT: &str = "echo-message-timeout";

/// How long an echo message stays on screen when nothing sets
/// [`ECHO_MESSAGE_TIMEOUT`] to something else.
pub const DEFAULT_ECHO_MESSAGE_TIMEOUT: f64 = 5.0;

/// What the echo area is showing, and since when.
///
/// The timestamp lives beside the text rather than in a lock of its own so
/// that a reader cannot catch a new message paired with the previous one's
/// clock and hide it a moment after it appeared.
#[derive(Debug, Clone, PartialEq)]
pub struct EchoMessage {
    pub text: String,
    /// When [`EditorState::set_echo_message`] last wrote `text`. Each new
    /// message restarts the clock, so a message is always shown for its full
    /// timeout however soon it followed the one before.
    pub set_at: Instant,
}

impl EchoMessage {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            set_at: Instant::now(),
        }
    }

    /// How long this message has been on screen, or `None` once TIMEOUT has
    /// run out -- and `None` too when there is no message to show, so that an
    /// empty echo area never counts as something waiting to expire.
    fn visible_for(&self, timeout: Option<Duration>) -> Option<Duration> {
        if self.text.is_empty() {
            return None;
        }
        let elapsed = self.set_at.elapsed();
        match timeout {
            Some(timeout) if elapsed >= timeout => None,
            _ => Some(elapsed),
        }
    }
}

/// The name of the Lisp variable holding the mode-line format.
pub const MODE_LINE_FORMAT: &str = "mode-line-format";

/// What a window's status line says when nothing sets [`MODE_LINE_FORMAT`].
pub const DEFAULT_MODE_LINE_FORMAT: &str = " %* %b   %m   L%l C%c   %p ";

/// The mode-line format as Lisp currently defines it.
///
/// Anything that is not a string falls back to the default rather than
/// blanking every status line in the editor: a mistyped format should look
/// wrong, not make the editor look broken.
fn mode_line_format<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> String {
    match env.get_variable(MODE_LINE_FORMAT) {
        Some(ELispExp::String(format)) => format.to_string(),
        _ => DEFAULT_MODE_LINE_FORMAT.to_string(),
    }
}

/// The echo timeout as Lisp currently defines it, or `None` for "never
/// expires".
///
/// Only a finite, non-negative number arms the timeout. `nil` means the
/// message stays until something replaces it, and so does anything else --
/// an unbound variable, a string, a list, or an infinity. Refusing to guess
/// at a nonsensical value is the safe direction: the failure mode is a
/// message that outstays its welcome, not one that disappears before it is
/// read.
fn echo_timeout<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
    match env.get_variable(ECHO_MESSAGE_TIMEOUT) {
        Some(ELispExp::Number(seconds)) if seconds.is_finite() => {
            Some(Duration::from_secs_f64(seconds.max(0.0)))
        }
        _ => None,
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
    /// A channel that is used to send work to a worker thread like
    /// the syntax highlighting computation
    pub worker_mailbox: Sender<WorkerMessage<B>>,

    pub buffers: Arc<RwLock<HashMap<String, Arc<RwLock<Buffer<B>>>>>>,
    /// The echo area's text together with when it was set, in one lock so a
    /// reader can never pair a new message with an old timestamp.
    pub echo_message: Arc<RwLock<EchoMessage>>,
    /// An `Arc<str>` rather than a `String` because every buffer access starts
    /// by reading this name, and reading a `String` out of a lock means
    /// copying it. Sharing it instead makes `get_current_buffer` allocation
    /// free, on a path that runs several times per keystroke.
    pub current_buffer_name: Arc<RwLock<Arc<str>>>,

    /// A keymap is an association between a KeyEvent and the name of a
    /// function that have to be executed (i.e. self-insert)
    pub keymaps: Arc<RwLock<Keymap<B>>>,
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

    /// Name of the command that ran immediately before the current one.
    ///
    /// Emacs' `last-command`. A command that wants to know whether it is a
    /// repeat of itself needs this: vertical movement uses it to decide
    /// whether a goal column is still in play, and appending kills (#20) will
    /// want it too.
    /// Held as the symbol's own `Arc` rather than a fresh `String`: this is
    /// written on every keystroke, and a name that is already interned in the
    /// keymap does not need copying to be remembered.
    last_command: Arc<RwLock<Option<Arc<String>>>>,

    /// Killed and copied text, shared by every buffer so that a kill in one
    /// can be yanked into another.
    kill_ring: Arc<RwLock<KillRing>>,

    /// How each face is drawn. One theme for the whole editor -- a per-buffer
    /// theme would mean two windows on the same file disagreeing about what a
    /// keyword looks like.
    theme: Arc<RwLock<Theme>>,

    /// The argument being built for the next command, and whether the digit
    /// keys are still being read into it.
    ///
    /// Two fields because "there is an argument" and "digits still extend it"
    /// are different: after `C-u 4 C-x`, the four is still the next command's
    /// argument, but the `4` in a following `C-x 4 f` belongs to the key
    /// sequence, not to the number.
    prefix_arg: Arc<RwLock<(Option<PrefixArg>, bool)>>,

    /// Keys pressed so far that do not yet make a complete binding.
    ///
    /// On the editor rather than on a mode, because a mode keymap is consulted
    /// before the global one and a sequence begun under one must not be
    /// half-remembered by another. One place, whatever mode is active.
    ///
    /// Kept as one `Vec` that is pushed to and cleared rather than rebuilt, so
    /// the common case -- a single key that is a whole binding -- costs a push
    /// and a clear rather than an allocation on the keystroke path.
    pending_keys: Arc<RwLock<Vec<KeyEvent>>>,

    /// Whether the command *before* this one killed, and whether this one has.
    ///
    /// A pair rather than one flag because the question -- "is this kill
    /// continuing a run?" -- is asked during a command about the one before
    /// it, so the answer has to be settled before the command runs and
    /// recorded while it does. `handle_key_event` rolls the second into the
    /// first between commands, exactly as it does for `last_command`.
    ///
    /// Deliberately *not* rolled over inside a single Lisp function: two
    /// `(kill-line)` calls in one `defun` append into one entry, which is what
    /// Emacs does too and what makes a Lisp-driven kill loop useful.
    last_command_killed: Arc<AtomicBool>,
    this_command_killed: Arc<AtomicBool>,

    /// Where the last yank put its text, so `yank-pop` knows what to replace,
    /// and whether the previous command was that yank.
    last_yank: Arc<RwLock<Option<(usize, usize)>>>,
    last_command_yanked: Arc<AtomicBool>,
    this_command_yanked: Arc<AtomicBool>,

    /// Column that repeated vertical movement is aiming for.
    ///
    /// Moving down through a short line and back up must return to the
    /// column you started from, so the target column is remembered rather
    /// than re-read from the cursor -- which a short line would have clamped.
    goal_column: Arc<RwLock<Option<usize>>>,

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

    fn begin_unwind(&self) {
        // Roughly a hundredth of a command's budget: ample for closing a file
        // or restoring a variable, far too little to hide a runaway loop.
        self.fuel.grant(100_000);
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

        let mut keymaps = Keymap::new();
        fill_default_keymaps(&mut keymaps);

        // Create a secondary worker thread and the communication channels with it.
        let (sender, receiver) = std::sync::mpsc::channel();

        let editor_state = Self {
            running: Arc::new(AtomicBool::new(true)),
            worker_mailbox: sender,
            buffers: Arc::new(RwLock::new(buffers)),
            echo_message: Arc::new(RwLock::new(EchoMessage::new("Welcome to rsedit"))),
            current_buffer_name: Arc::new(RwLock::new(Arc::from(scratch_name.as_str()))),
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
            last_command: Arc::new(RwLock::new(None)),
            kill_ring: Arc::new(RwLock::new(KillRing::default())),
            theme: Arc::new(RwLock::new(Theme::default())),
            prefix_arg: Arc::new(RwLock::new((None, false))),
            pending_keys: Arc::new(RwLock::new(Vec::new())),
            last_command_killed: Arc::new(AtomicBool::new(false)),
            this_command_killed: Arc::new(AtomicBool::new(false)),
            last_yank: Arc::new(RwLock::new(None)),
            last_command_yanked: Arc::new(AtomicBool::new(false)),
            this_command_yanked: Arc::new(AtomicBool::new(false)),
            goal_column: Arc::new(RwLock::new(None)),
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
    /// Offer EVENT to the prefix-argument reader, and say whether it was taken.
    ///
    /// Called before the key sequence gets a look, because `C-u` and the digits
    /// that follow it are not keys the keymaps should ever see. Anything the
    /// reader does not want falls straight through -- including a digit typed
    /// when no argument is being built, which is how you can still type the
    /// number four into a buffer.
    fn read_prefix_argument(&self, event: &KeyEvent) -> bool {
        let mut state = self
            .prefix_arg
            .write()
            .expect("Failed to acquire write lock on prefix_arg");
        let (arg, reading) = &mut *state;

        // C-u: start an argument, or multiply the one being built by four.
        if event.modifiers.ctrl && !event.modifiers.alt && event.code == KeyCode::Char('u') {
            *arg = Some(match (*arg, *reading) {
                (Some(PrefixArg::Raw(times)), true) => PrefixArg::Raw(times + 1),
                _ => PrefixArg::Raw(1),
            });
            *reading = true;
            return true;
        }

        if *reading
            && let KeyCode::Char(c) = event.code
            && !event.modifiers.ctrl
            && !event.modifiers.alt
        {
            if let Some(digit) = c.to_digit(10) {
                arg.get_or_insert(PrefixArg::Raw(1)).push_digit(digit);
                return true;
            }
            // A minus is only a sign, and only before any digits.
            if c == '-' && matches!(*arg, Some(PrefixArg::Raw(_))) {
                *arg = Some(PrefixArg::Negative);
                return true;
            }
        }

        // Whatever this key is, it is not part of the argument -- so the
        // argument is finished, even though it has not been used yet.
        *reading = false;
        false
    }

    /// The argument waiting for the next command, if any.
    pub(crate) fn prefix_argument(&self) -> Option<PrefixArg> {
        self.prefix_arg
            .read()
            .expect("Failed to acquire read lock on prefix_arg")
            .0
    }

    /// Abandon whatever is half-finished: a key sequence, a prefix argument.
    ///
    /// The text-level half of `keyboard-quit`. Interrupting a *running* command
    /// is roadmap #24 and needs more than this; abandoning something not yet
    /// started needs only this, and is what a mistyped `C-x` calls for.
    pub(crate) fn abandon_pending_input(&self) {
        self.pending_keys
            .write()
            .expect("Failed to acquire write lock on pending_keys")
            .clear();
        self.clear_prefix_argument();
    }

    /// What the editor is part-way through reading, spelt the way a binding
    /// is written and ending in a dash for the key still to come.
    ///
    /// The argument and the key sequence appear together -- `C-u 4 C-x-` --
    /// because they compose, and showing only one of them would misreport
    /// what pressing the next key will do.
    pub(crate) fn pending_input(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(arg) = self.prefix_argument() {
            parts.push(arg.describe());
        }
        let keys = self
            .pending_keys
            .read()
            .expect("Failed to acquire read lock on pending_keys");
        if !keys.is_empty() {
            parts.push(describe_keys(&keys));
        }
        if parts.is_empty() {
            return String::new();
        }
        format!("{}-", parts.join(" "))
    }

    /// Forget the argument. Called after every command, whether or not
    /// anything consumed it: an argument belongs to exactly one command, and
    /// one that errors must not leave its argument for the next.
    pub(crate) fn clear_prefix_argument(&self) {
        *self
            .prefix_arg
            .write()
            .expect("Failed to acquire write lock on prefix_arg") = (None, false);
    }

    /// Add EVENT to the sequence being typed, and say what to run.
    ///
    /// `Some(ast)` means the sequence is now a complete binding and has been
    /// cleared ready for the next one. `None` means there is nothing to run --
    /// either because more keys are expected, or because the sequence is bound
    /// to nothing.
    ///
    /// # Why this returns rather than dispatching
    ///
    /// A key that only lengthens a sequence is **not a command**. It must not
    /// reach `begin_command`, `roll_over_command_flags` or `set_last_command`,
    /// all of which live in the caller below the point this returns `None`.
    /// Letting `C-x` through any of them would silently end the undo group
    /// being typed into, and split a run of kills into two ring entries --
    /// neither of which looks like a key-handling bug when you go looking.
    fn resolve_key_sequence(&self, event: KeyEvent) -> Option<ELispExp<B>> {
        let mut pending = self
            .pending_keys
            .write()
            .expect("Failed to acquire write lock on pending_keys");
        pending.push(event);

        let current_mode = {
            let buffer = self.get_current_buffer();
            let buffer = buffer
                .read()
                .expect("Failed to acquire read lock on current_buffer");
            buffer.current_mode.clone()
        };

        // The mode's own keymap wins, then the global one -- and a mode that
        // binds a prefix keeps the sequence alive even when only the global
        // map completes it.
        let registry = self
            .mode_registry
            .read()
            .expect("Failed to acquire read lock on mode_registry");
        let mode_keymap = registry.get(&current_mode).map(|mode| &mode.keymaps);
        let global_keymap = self
            .keymaps
            .read()
            .expect("Failed to acquire read lock on keymaps");

        let bound = mode_keymap
            .and_then(|keymap| keymap.get(&pending))
            .or_else(|| global_keymap.get(&pending))
            .cloned();
        let is_prefix = mode_keymap.is_some_and(|keymap| keymap.is_prefix(&pending))
            || global_keymap.is_prefix(&pending);

        // Everything the keymaps had to say, said. Released here so that
        // reporting -- which writes the echo area, a lock of its own -- happens
        // under no keymap lock at all.
        drop(global_keymap);
        drop(registry);

        let described = describe_keys(&pending);
        if bound.is_some() || !is_prefix {
            pending.clear();
        }
        drop(pending);

        match (bound, is_prefix) {
            (Some(ast), _) => Some(ast),
            // Nothing to say: the sequence is in `pending_input`, which the
            // frame carries and which does not expire the way a message does.
            (None, true) => None,
            (None, false) => {
                self.set_echo_message(&format!("{described} is undefined"));
                self.log_diagnostic(&format!("[INFO] Keymap not bound {described}"));
                None
            }
        }
    }

    pub fn handle_key_event(&self, event: KeyEvent, env: &Arc<Env<EditorState<B>>>) {
        // The argument reader gets first refusal. A key it takes is not a
        // command and never reaches a keymap.
        if self.read_prefix_argument(&event) {
            return;
        }

        // A key may be the whole of a binding, the start of a longer one, or
        // neither. Deciding which comes first, and two of the three answers
        // return before anything below runs -- see `resolve_key_sequence`.
        let Some(mut ast) = self.resolve_key_sequence(event.clone()) else {
            return;
        };

        if let ELispExp::Lambda(ref lambda) = ast {
            if lambda.params.len() != 0 {
                self.log_diagnostic(&format!("Keymap {event:?} associated to a lambda with some parameters. Associate it to a lambda with 0 parameters."));
                return;
            } else {
                ast = ELispExp::form(vec![ELispExp::symbol("funcall".into()), ast]);
            }
        }

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
        // The command this key runs, whether or not the binding supplies its
        // arguments: `(self-insert "a")` names `self-insert` just as much as a
        // bare `next-line` does. `bound_command` above deliberately matches
        // only the argument-less shapes, because those are the ones to route
        // through `call-interactively`; grouping and `last-command` want the
        // wider answer, since typing is exactly the case they care about.
        //
        // Remembered for the *next* command to consult, so that during this
        // one `last_command` still names its predecessor -- which is what
        // makes "am I a repeat of myself?" answerable.
        let this_command = match &ast {
            ELispExp::Symbol(name) => Some(name.clone()),
            ELispExp::Form(items) => items.first().and_then(|head| match head {
                ELispExp::Symbol(name) => Some(name.clone()),
                _ => None,
            }),
            _ => None,
        };
        // Told to the undo history before the command runs, so that its edits
        // -- however many it makes -- land in one group. This is a thread-local
        // stamp and an atomic increment: it costs nothing on the keys that
        // edit nothing, which is most of them.
        crate::buffer::undo::begin_command(
            this_command.as_deref().map(String::as_str) == Some("self-insert"),
        );
        // Before the command runs, not after: a command asks these about its
        // predecessor, so the answer has to be in place by the time it starts.
        // Rolling over here rather than at the end also means a command that
        // fails partway cannot leave its flag set for the next one to read.
        self.roll_over_command_flags();
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
        // Before the error check, so a command that fails still consumes its
        // argument rather than leaving it for whatever runs next.
        self.clear_prefix_argument();
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
        self.set_last_command(this_command);
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

        let current = self.current_buffer_name_shared();
        if &*current == name || self.get_buffer(&current).is_none() {
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
    /// The Lisp environment is read first of all, before any of these, so that
    /// no editor lock is ever held while touching it.
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
    pub fn snapshot(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        screen_width: usize,
        screen_height: usize,
    ) -> FrameSnapshot {
        // Read *before* the first editor lock is taken. The environment has
        // locks of its own, and taking one while holding an editor lock would
        // add an edge to the ordering below that nothing else respects.
        let echo_timeout = echo_timeout(env);
        // The theme is read before the structural locks and never alongside
        // them, so it stays outside the ordering below rather than becoming
        // another link in it. It is a small `Copy` value, so this is a memcpy
        // and the lock is released immediately.
        let theme = self.theme();
        let pending_input = self.pending_input();
        let mode_line_format = mode_line_format(env);

        let focused_window_id = *self
            .focused_window_id
            .read()
            .expect("Failed to acquire read lock on focused_window_id");
        // An expired message is simply not reported. The state keeps it -- the
        // view is what forgets, so nothing has to run on a timer to tidy up.
        let echo_message = {
            let echo = self
                .echo_message
                .read()
                .expect("Failed to acquire read lock on echo_message");
            match echo.visible_for(echo_timeout) {
                Some(_) => echo.text.clone(),
                None => String::new(),
            }
        };
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
                // The bottom row belongs to the echo area, which is drawn over
                // whatever is under it. Tiling into it would put a window's
                // status line on the same row as a message, and one of the two
                // would win at random.
                height: screen_height.saturating_sub(1),
            },
            focused_window_id,
            &buffers,
            &mode_line_format,
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
                title: float.title.clone(),
                is_focused,
                cursor_rel_pos,
                lines: extract_buffer_lines(&float.window, &float.rect, &buffers),
                highlights: region_highlights(&float.window, &float.rect, &buffers),
                // A float says what it is on its border, so a status line
                // would be a second answer to the same question.
                mode_line: None,
                has_border: float.has_border,
            });
        }

        FrameSnapshot {
            views,
            echo_message,
            pending_input,
            theme,
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

    /// Return the editor echo string.
    ///
    /// The message as stored, whether or not it is still being shown: expiry
    /// is a question for the view, decided in [`Self::snapshot`], and state is
    /// not rewritten by the passage of time.
    pub fn get_echo_message(&self) -> String {
        self.echo_message
            .read()
            .expect("Failed to acquire read lock on echo_message")
            .text
            .clone()
    }

    /// How long until the current echo message stops being shown, or `None`
    /// when nothing is waiting to expire -- there is no message, no timeout is
    /// set, or the message has already expired.
    ///
    /// This is what a UI event loop needs in order to redraw when a message
    /// vanishes. Without it the loop blocks on the next key and the message
    /// stays on screen until the user happens to press one, which is not a
    /// timeout so much as a coincidence.
    pub fn echo_expiry_in(&self, env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
        let timeout = echo_timeout(env)?;
        let elapsed = self
            .echo_message
            .read()
            .expect("Failed to acquire read lock on echo_message")
            .visible_for(Some(timeout))?;
        timeout.checked_sub(elapsed).filter(|left| !left.is_zero())
    }

    /// Set the echo message to be MSG, and start its timeout running.
    pub fn set_echo_message(&self, msg: &str) {
        *self
            .echo_message
            .write()
            .expect("Failed to acquire write lock on echo_message") = EchoMessage::new(msg);
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
    pub(crate) fn push_pending_command(
        &self,
        name: String,
        remaining: Vec<ArgSpec>,
        invocation: Invocation,
    ) {
        self.pending_commands
            .write()
            .expect("Failed to acquire write lock on pending_commands")
            .push(PendingCommand::new(name, remaining, invocation));
    }

    /// Everything the editor can answer on the user's behalf, as it stands now.
    ///
    /// Taken once, when a command starts. See [`Invocation`] for why it is not
    /// read again later.
    pub(crate) fn capture_invocation(&self) -> Invocation {
        let buffer = self.get_current_buffer();
        let buffer = buffer
            .read()
            .expect("Failed to acquire read lock on current buffer");
        Invocation {
            prefix_arg: self.prefix_argument(),
            region: crate::buffer::mark::region_bounds(
                buffer.mark,
                buffer.text.cursor_pos_1d(),
                buffer.text.len(),
            ),
        }
    }

    /// Answer every argument the editor can answer itself, in order, and
    /// return the first one that still needs the user.
    ///
    /// `None` means the command has everything it needs and is ready to run.
    ///
    /// Done in one loop with the prompted arguments rather than as a separate
    /// pass: a spec list like `["p", "sReplace with: "]` interleaves the two
    /// kinds, and two code paths that both maintain the pending stack would
    /// have to agree about it forever.
    pub(crate) fn fill_answerable_args(&self) -> Option<ArgSpec> {
        let mut stack = self
            .pending_commands
            .write()
            .expect("Failed to acquire write lock on pending_commands");
        let pending = stack.last_mut()?;
        while let Some(spec) = pending.remaining.first() {
            if spec.prompts() {
                return Some(spec.clone());
            }
            let values = answer_spec::<B>(spec, &pending.invocation);
            pending.remaining.remove(0);
            pending.collected.extend(values);
        }
        None
    }

    /// How far through its arguments the innermost pending command is:
    /// `(command name, 1-based position of the argument being read, total)`.
    ///
    /// Used to title the prompt, so that answering the second of two questions
    /// says which command asked and which question it is. Without it a prompt
    /// reading `Find file:` gives no hint that `find-file` is what is waiting
    /// on the answer -- and with two arguments, no hint of which one is being
    /// asked for.
    pub(crate) fn pending_progress(&self) -> Option<(String, usize, usize)> {
        let stack = self
            .pending_commands
            .read()
            .expect("Failed to acquire read lock on pending_commands");
        let pending = stack.last()?;
        let done = pending.collected.len();
        Some((pending.name.clone(), done + 1, done + pending.remaining.len()))
    }

    /// The argument the innermost pending command is waiting on.
    pub(crate) fn pending_current_spec(&self) -> Option<ArgSpec> {
        self.pending_commands
            .read()
            .expect("Failed to acquire read lock on pending_commands")
            .last()
            .and_then(|pending| pending.current().cloned())
    }

    /// Record VALUE as the innermost pending command's next argument.
    ///
    /// What to do next is [`Self::fill_answerable_args`]'s answer, not this
    /// one's: the specs after this may be a mix of answerable and prompted,
    /// and only one place should know how to walk them.
    pub(crate) fn accept_pending_arg(&self, value: ELispExp<B>) {
        let mut stack = self
            .pending_commands
            .write()
            .expect("Failed to acquire write lock on pending_commands");
        let Some(pending) = stack.last_mut() else {
            return;
        };
        if !pending.remaining.is_empty() {
            pending.remaining.remove(0);
        }
        pending.collected.push(value);
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

    // ---------------------------------------------------------------
    // What the previous command was, and where vertical movement is aiming
    // ---------------------------------------------------------------

    /// Whether the previous command was NAME.
    ///
    /// A predicate rather than a getter because the question asked of
    /// `last-command` is always "was it this one?", and answering it by
    /// handing out a copy of the name would allocate on a path that runs
    /// between a key being pressed and the character appearing.
    pub(crate) fn last_command_is(&self, name: &str) -> bool {
        self.last_command
            .read()
            .expect("Failed to acquire read lock on last_command")
            .as_deref()
            .map(String::as_str)
            == Some(name)
    }

    // ---------------------------------------------------------------
    // Faces and the theme
    // ---------------------------------------------------------------

    /// Bind FACE to STYLE for the whole editor.
    pub(crate) fn set_face_style(&self, face: Face, style: Style) {
        self.theme
            .write()
            .expect("Failed to acquire write lock on theme")
            .set(face, style);
    }

    pub(crate) fn face_style(&self, face: Face) -> Style {
        self.theme
            .read()
            .expect("Failed to acquire read lock on theme")
            .style(face)
    }

    pub(crate) fn theme(&self) -> Theme {
        *self
            .theme
            .read()
            .expect("Failed to acquire read lock on theme")
    }

    // ---------------------------------------------------------------
    // The kill ring
    // ---------------------------------------------------------------

    /// Save TEXT as killed text.
    ///
    /// A run of kill commands accumulates into one entry rather than filling
    /// the ring with fragments -- that is what makes repeated `C-k` yank back
    /// as the whole passage. DIRECTION says which end of the entry a
    /// continued kill joins onto, so a backward kill does not assemble its
    /// text inside out.
    pub(crate) fn kill(&self, text: String, direction: Direction) {
        let continuing = self.last_command_killed.load(Ordering::Relaxed);
        self.this_command_killed.store(true, Ordering::Relaxed);
        let mut ring = self
            .kill_ring
            .write()
            .expect("Failed to acquire write lock on kill_ring");
        if continuing {
            ring.append(text, direction);
        } else {
            ring.push(text);
        }
    }

    /// What `yank` would insert, if anything.
    pub(crate) fn current_kill(&self) -> Option<String> {
        self.kill_ring
            .read()
            .expect("Failed to acquire read lock on kill_ring")
            .current()
            .map(str::to_string)
    }

    /// The entry N kills back, without moving the ring.
    pub(crate) fn nth_kill(&self, n: usize) -> Option<String> {
        self.kill_ring
            .read()
            .expect("Failed to acquire read lock on kill_ring")
            .nth(n)
            .map(str::to_string)
    }

    /// Step the ring back one entry and return what is now current.
    pub(crate) fn rotate_kill_ring(&self) -> Option<String> {
        self.kill_ring
            .write()
            .expect("Failed to acquire write lock on kill_ring")
            .rotate()
            .map(str::to_string)
    }

    pub(crate) fn set_kill_ring_max(&self, max: usize) {
        self.kill_ring
            .write()
            .expect("Failed to acquire write lock on kill_ring")
            .set_max(max);
    }

    pub(crate) fn kill_ring_len(&self) -> usize {
        self.kill_ring
            .read()
            .expect("Failed to acquire read lock on kill_ring")
            .len()
    }

    /// Remember that a yank put LEN characters at AT, so `yank-pop` knows what
    /// to take back out.
    pub(crate) fn note_yank(&self, at: usize, len: usize) {
        self.this_command_yanked.store(true, Ordering::Relaxed);
        *self
            .last_yank
            .write()
            .expect("Failed to acquire write lock on last_yank") = Some((at, len));
    }

    /// What the previous command yanked, if the previous command was a yank.
    ///
    /// `yank-pop` replaces the text a yank just inserted, so it is only
    /// meaningful directly after one; anything else in between and there is
    /// nothing it would be safe to remove.
    pub(crate) fn yank_to_replace(&self) -> Option<(usize, usize)> {
        if !self.last_command_yanked.load(Ordering::Relaxed) {
            return None;
        }
        *self
            .last_yank
            .read()
            .expect("Failed to acquire read lock on last_yank")
    }

    /// Roll "this command" into "the previous command" for the flags that a
    /// command needs to ask about its predecessor.
    fn roll_over_command_flags(&self) {
        for (last, this) in [
            (&self.last_command_killed, &self.this_command_killed),
            (&self.last_command_yanked, &self.this_command_yanked),
        ] {
            last.store(this.swap(false, Ordering::Relaxed), Ordering::Relaxed);
        }
    }

    pub(crate) fn set_last_command(&self, name: Option<Arc<String>>) {
        *self
            .last_command
            .write()
            .expect("Failed to acquire write lock on last_command") = name;
    }

    pub(crate) fn goal_column(&self) -> Option<usize> {
        *self
            .goal_column
            .read()
            .expect("Failed to acquire read lock on goal_column")
    }

    pub(crate) fn set_goal_column(&self, col: Option<usize>) {
        *self
            .goal_column
            .write()
            .expect("Failed to acquire write lock on goal_column") = col;
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
        self.current_buffer_name_shared().to_string()
    }

    /// The current buffer's name without copying it.
    pub(crate) fn current_buffer_name_shared(&self) -> Arc<str> {
        self.current_buffer_name
            .read()
            .expect("Failed to acquire read lock on current_buffer_name")
            .clone()
    }

    /// Set the name of the current buffer
    pub(crate) fn set_current_buffer_name(&self, name: &str) {
        *self
            .current_buffer_name
            .write()
            .expect("Failed to acquire write lock on current_buffer_name") = Arc::from(name);
    }

    /// Returns an Arc reference to the current buffer
    pub(crate) fn get_current_buffer(&self) -> Arc<RwLock<Buffer<B>>> {
        self.buffers
            .read()
            .expect("Failed to acquire read lock on buffers")
            .get(&*self.current_buffer_name_shared())
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

/// What the editor hands a command for an argument it answers itself.
///
/// One spec can produce more than one value -- `r` is the region's *two* ends,
/// exactly as `interactive "r"` is in Emacs -- so this returns a list rather
/// than a value.
fn answer_spec<B: BufferTrait>(spec: &ArgSpec, invocation: &Invocation) -> Vec<ELispExp<B>> {
    match spec {
        // `p`: a plain count, and one when the user asked for nothing. This is
        // what makes `(forward-char)` and `C-u 4 C-f` the same code path.
        ArgSpec::Count => vec![ELispExp::number(
            invocation.prefix_arg.map(|arg| arg.count()).unwrap_or(1) as f64,
        )],
        // `P`: the argument as given, so a command can tell "no argument" from
        // "the argument 1", and a bare `C-u` from `C-u 4`. A bare `C-u` is a
        // one-element list, as in Emacs, which is why `p` and `P` both exist.
        ArgSpec::RawCount => vec![match invocation.prefix_arg {
            None => ELispExp::nil(),
            Some(PrefixArg::Raw(times)) => {
                ELispExp::proper_list(vec![ELispExp::number(4i32.saturating_pow(times) as f64)])
            }
            Some(PrefixArg::Number(n)) => ELispExp::number(n as f64),
            Some(PrefixArg::Negative) => ELispExp::symbol("-".into()),
        }],
        // `r`: start then end. `call-interactively` refuses the command before
        // this is reached when there is no region, so the fallback is
        // unreachable rather than a silent default.
        ArgSpec::Region => {
            let (start, end) = invocation.region.unwrap_or((0, 0));
            vec![ELispExp::number(start as f64), ELispExp::number(end as f64)]
        }
        prompted => {
            debug_assert!(prompted.prompts(), "an unprompted spec with no answer");
            Vec::new()
        }
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

    // How long a message stays in the echo area. A number of seconds arms the
    // timeout; nil leaves messages up until something replaces them. Set here
    // rather than in a `.lisp` file so the default holds even with no Lisp
    // loaded, and so that `describe`-style introspection finds it bound.
    env.set_variable(
        ECHO_MESSAGE_TIMEOUT.into(),
        ELispExp::number(DEFAULT_ECHO_MESSAGE_TIMEOUT),
    );

    // What each window's status line shows. Set here rather than in a `.lisp`
    // file so a window is labelled even with no configuration loaded.
    env.set_variable(
        MODE_LINE_FORMAT.into(),
        ELispExp::string(DEFAULT_MODE_LINE_FORMAT.to_string()),
    );

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
