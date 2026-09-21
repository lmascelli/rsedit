use crate::{
    ELispExp,
    buffer::{Buffer, BufferTrait},
    commands::{ArgSpec, CommandRegistry, Invocation, PendingCommand, PrefixArg},
    input::{
        KeyCode, KeyEvent, Keymap, OnUnbound, TransientKeymap, describe_keys, fill_default_keymaps,
    },
    isearch::install_isearch,
    kill_ring::{Direction, KillRing},
    lisp::{
        DEFAULT_FUEL, Env, EvalError, FuelMeter, FuelScope, LispContext, Parser, bootstrap_vm, eval,
    },
    minibuffer::install_minibuffer,
    modes::highlighter::{Highlighter, TURN_INTERVAL},
    modes::prescan::Prescanner,
    modes::{MajorMode, SyntaxTable},
    primitives::install_primitives,
    search::Isearch,
    task::{BackgroundScheduler, WorkerMessage},
    ui::{
        Division, Face, FloatingWindow, FrameSnapshot, LayoutNode, Orientation, Rect,
        RenderableWindowView, Separator, Style, Theme, Window, extract_buffer_lines,
        region_highlights,
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

/// The names of the Lisp variables that size a minibuffer prompt.
pub const MINIBUFFER_WIDTH: &str = "minibuffer-width";
pub const MINIBUFFER_HEIGHT: &str = "minibuffer-height";

/// How many lines two consecutive screenfuls share.
///
/// Emacs' name and Emacs' value. Without the overlap a reader loses their place
/// at every page: the line they were reading when they pressed the key is gone,
/// and nothing on the new screen says where it was.
pub const NEXT_SCREEN_CONTEXT_LINES: usize = 2;

/// How large a prompt is when nothing says otherwise.
///
/// Wide enough for a path and narrow enough to read as a dialogue rather than
/// as part of the frame. Both are clamped to what the terminal can actually
/// hold, so these are a preference and not a promise -- a 40-column terminal
/// gets a 38-column prompt rather than one running off the edge.
///
/// Three rows, of which one is text: a border above and below, and the line
/// being typed between them.
pub const DEFAULT_MINIBUFFER_WIDTH: f64 = 60.0;
pub const DEFAULT_MINIBUFFER_HEIGHT: f64 = 3.0;

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

pub const WINDOW_SEPARATOR: &str = "window-separator";
pub const DEFAULT_WINDOW_SEPARATOR: char = '\u{2502}';

fn window_separator<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> char {
    match env.get_variable(WINDOW_SEPARATOR) {
        Some(ELispExp::String(text)) => text.chars().next().unwrap_or(' '),
        Some(value) if value.is_nil() => ' ',
        _ => DEFAULT_WINDOW_SEPARATOR,
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
    /// Where completions come from in any buffer, whatever its mode -- the
    /// global half of what `MajorMode::completion_functions` holds per mode,
    /// and the same relationship `keymaps` has to `MajorMode::keymaps`.
    pub completion_functions: Arc<RwLock<Vec<ELispExp<B>>>>,
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

    /// Buffer names in the order they were last current, most recent first.
    ///
    /// # What it is for
    ///
    /// Answering "and what should I show instead?". When a buffer is killed,
    /// every window showing it needs somewhere to point, and `*scratch*` is a
    /// poor answer when the person had three files open -- they want one of
    /// the files. That question cannot be answered from the buffer table,
    /// which is a `HashMap` and has no order at all.
    ///
    /// Updated by [`Self::set_current_buffer_name`], which is the one place a
    /// buffer becomes current, so nothing else has to remember to record
    /// anything. Names of killed buffers are left in the list rather than
    /// pruned on every kill; readers skip the ones that are gone, which costs
    /// a lookup there and saves a scan on a path that runs per keystroke.
    buffer_recency: Arc<RwLock<Vec<String>>>,

    /// How many shell commands are still running.
    ///
    /// # Why the editor counts them
    ///
    /// Output arrives from the worker thread, on its own, with nobody touching
    /// the keyboard -- the same situation syntax colouring is in, and it has
    /// the same consequence: a renderer blocked on input sleeps straight
    /// through it, and the output appears only when some key is pressed. So
    /// [`Self::next_redraw_in`] asks this, and keeps waking while anything is
    /// running.
    ///
    /// A count rather than a flag because several commands can run at once,
    /// each into a buffer of its own.
    shell_commands: Arc<AtomicUsize>,

    /// Hooks that run in every major mode, keyed by hook name.
    ///
    /// A mode's own hooks live on the mode (see `MajorMode::hooks`), which is
    /// right for anything that is about *this kind of buffer*. Some things are
    /// not: the completion strip has to redraw after any command that changed
    /// what was typed, and what the buffer's mode happens to be has nothing to
    /// do with it. Registering such a hook in every mode separately would work
    /// until somebody defined a mode afterwards.
    ///
    /// Reached from Lisp as `(add-hook nil HOOK FUNCTION)` -- nil meaning
    /// everywhere, the same way it does in `define-key`.
    global_hooks: Arc<RwLock<HashMap<String, Vec<ELispExp<B>>>>>,

    /// The last command as a form that can be evaluated again, which is what
    /// `repeat` re-runs.
    ///
    /// Separate from `last_command` because that holds a *name*, and a name is
    /// not enough to run anything: the keymaps store `(self-insert "a")`, and
    /// the character is in the form rather than in the name. Stored after the
    /// `call-interactively` rewrite, so what is kept is the form that actually
    /// ran -- which means repeating a command that prompts, prompts again.
    last_command_form: Arc<RwLock<Option<ELispExp<B>>>>,

    /// Killed and copied text, shared by every buffer so that a kill in one
    /// can be yanked into another.
    kill_ring: Arc<RwLock<KillRing>>,

    /// Text waiting to be handed to the *system* clipboard, or `None` when
    /// there is nothing outstanding.
    ///
    /// # Why the editor cannot just do it
    ///
    /// Putting text in the system clipboard means writing an OSC 52 escape to
    /// the terminal, and the editor does not own the terminal -- it does not
    /// know it has one. So a kill leaves the text here, [`Self::snapshot`]
    /// *takes* it onto the frame, and the renderer -- the one part that does
    /// own stdout -- emits it. Same shape as everything else the renderer
    /// draws, arrived at for the same reason.
    ///
    /// Taken rather than read, so one kill sends one escape however many
    /// frames get drawn afterwards.
    pending_clipboard: Arc<RwLock<Option<String>>>,

    /// How each face is drawn. One theme for the whole editor -- a per-buffer
    /// theme would mean two windows on the same file disagreeing about what a
    /// keyword looks like.
    /// How each face is drawn. One theme for the whole editor.
    ///
    /// `Arc<Theme>` inside the lock, not a bare `Theme`: every frame takes a
    /// copy, and now that the face set is open a theme is a heap-allocated
    /// `Vec` rather than a fixed array -- so copying one per frame would mean
    /// allocating per frame. Sharing it costs a refcount, and changing it
    /// (`set-face`, which happens when configuration is read) makes a new one.
    theme: Arc<RwLock<Arc<Theme>>>,

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
    /// The incremental search currently running, between one keystroke and the
    /// next. See `crate::search::Isearch`.
    isearch: Arc<RwLock<Option<Isearch>>>,

    /// A keymap consulted before every other, for as long as it is installed.
    /// See [`TransientKeymap`].
    ///
    /// The flag beside it is the *gate*: it is what every keystroke reads, and
    /// the lock is opened only when it says there is something to read. A
    /// transient map is up for a handful of keystrokes and absent for all the
    /// rest, so making the common answer an atomic load rather than a lock
    /// acquisition keeps the feature off the typing path.
    ///
    /// The two are written together and only together, by the methods below:
    /// the payload is stored before the gate opens and cleared after it closes,
    /// so a reader that gets through the gate always finds a map there.
    transient_keymap: Arc<RwLock<Option<TransientKeymap<B>>>>,
    transient_up: Arc<AtomicBool>,

    /// Which commands offer to repeat, and with which key. See
    /// `install_repeat_keymap`.
    ///
    /// Gated by an atomic for the same reason: until something declares a
    /// repeat key, no keystroke pays anything at all to ask.
    repeat_keys: Arc<RwLock<HashMap<String, KeyEvent>>>,
    any_repeat_keys: Arc<AtomicBool>,

    /// Which file names get which major mode, in the order they were declared.
    ///
    /// An ordered list rather than a map: patterns overlap -- `\.rs$` and
    /// `^Cargo\.` both match `Cargo.rs` -- so which one wins has to be a
    /// decision somebody made rather than whichever the hash happened to
    /// offer. First declared, first tried.
    auto_modes: Arc<RwLock<Vec<(regex::Regex, String)>>>,

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
            completion_functions: Arc::new(RwLock::new(Vec::new())),
            mode_registry: Arc::new(RwLock::new(HashMap::new())),
            layout_root: Arc::new(RwLock::new(LayoutNode::Leaf(Window::new(0, "*scratch*")))),
            floating_windows: Arc::new(RwLock::new(Vec::new())),
            focused_window_id: Arc::new(RwLock::new(0)),
            next_window_id: Arc::new(AtomicUsize::new(1)),
            commands: Arc::new(RwLock::new(CommandRegistry::new())),
            pending_commands: Arc::new(RwLock::new(Vec::new())),
            last_command: Arc::new(RwLock::new(None)),
            last_command_form: Arc::new(RwLock::new(None)),
            global_hooks: Arc::new(RwLock::new(HashMap::new())),
            shell_commands: Arc::new(AtomicUsize::new(0)),
            buffer_recency: Arc::new(RwLock::new(Vec::new())),
            kill_ring: Arc::new(RwLock::new(KillRing::default())),
            pending_clipboard: Arc::new(RwLock::new(None)),
            theme: Arc::new(RwLock::new(Arc::new(Theme::default()))),
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
            isearch: Arc::new(RwLock::new(None)),
            transient_keymap: Arc::new(RwLock::new(None)),
            transient_up: Arc::new(AtomicBool::new(false)),
            repeat_keys: Arc::new(RwLock::new(HashMap::new())),
            any_repeat_keys: Arc::new(AtomicBool::new(false)),
            auto_modes: Arc::new(RwLock::new(Vec::new())),
            call_stack: Arc::new(RwLock::new(Vec::new())),
        };
        BackgroundScheduler::spawn(receiver, editor_state.clone());
        // Colouring runs from here on, a bounded chunk at a time. Started at
        // construction rather than when a grammar is first defined, because a
        // job that has nothing to do costs one comparison per turn and a job
        // that was never started costs a file with no colour and no
        // explanation.
        let _ = editor_state.worker_mailbox.send(WorkerMessage::Schedule {
            task: Box::new(Highlighter),
            interval: TURN_INTERVAL,
        });
        // And the other lexer over the same text: the balanced-expression scan
        // that `forward-sexp', the indenter and `syntax-ppss' read. Same
        // interval and same bargain -- what it has worked out makes a motion
        // cheap, and what it has not yet reached makes one scan from the top,
        // which is what every motion did before this job existed.
        let _ = editor_state.worker_mailbox.send(WorkerMessage::Schedule {
            task: Box::new(Prescanner),
            interval: TURN_INTERVAL,
        });

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
                    // An explicit mode wins; otherwise the file's own name
                    // decides, which is how a language module ever gets used.
                    new_buf.current_mode = start_mode
                        .or_else(|| self.auto_mode_for(file_path))
                        .unwrap_or_else(|| "fundamental-mode".into());
                    new_buf.file_path = Some(file_path.to_string());

                    let mut buffers_lock = self
                        .buffers
                        .write()
                        .expect("Failed to get write lock on buffers");
                    buffers_lock.insert(name.to_string(), Arc::new(RwLock::new(new_buf)));

                    self.show_in_focused_window(name);

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

    /// An empty buffer that will be saved to PATH, for a file that is not
    /// there yet.
    ///
    /// # Why this is not `new_buffer` with a path
    ///
    /// That one reads the file, and reports failure when it cannot. A file
    /// that does not exist is not a failure here -- it is the ordinary way a
    /// file gets created, by opening it and typing. So this makes the empty
    /// buffer and remembers where it goes.
    ///
    /// The buffer is *not* modified. An empty buffer with no changes is the
    /// truth: nothing has been written, so leaving without saving loses
    /// nothing and should ask nothing. `C-x C-s` creates the file, because
    /// saving writes `file_path` whether or not anything was typed.
    pub(crate) fn new_file_buffer(
        &self,
        name: &str,
        path: &str,
        start_mode: Option<String>,
    ) -> String {
        let mut new_buf = Buffer::new(name);
        new_buf.current_mode = start_mode
            .or_else(|| self.auto_mode_for(path))
            .unwrap_or_else(|| "fundamental-mode".into());
        new_buf.file_path = Some(path.to_string());
        self.buffers
            .write()
            .expect("Failed to get write lock on buffers")
            .insert(name.to_string(), Arc::new(RwLock::new(new_buf)));
        self.show_in_focused_window(name);
        name.to_string()
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
        self.show_in_focused_window(name);
        true
    }

    /// Show the buffer named NAME in the focused window, and make it current.
    ///
    /// # Why this is a method and not five lines at each call site
    ///
    /// It is the mirror of [`EditorState::set_focused_window_id`]. That one
    /// moves focus and brings the current buffer along; this one changes the
    /// buffer and leaves focus where it is. Between them they are every way
    /// the pair (focused window, current buffer) is allowed to change, and
    /// keeping them in step is the whole job -- let them drift and the editor
    /// draws a cursor in one window and types into another.
    ///
    /// It was five lines at each of three call sites, and all three named the
    /// focused window and then edited whichever window the lookup handed back
    /// -- which, until [`LayoutNode::window_mut`] was fixed, was the leftmost
    /// one. Three copies is also three places for the next such bug to be
    /// fixed in only two of.
    fn show_in_focused_window(&self, name: &str) {
        if let Some(window) = self
            .layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root")
            .window_mut(self.get_focused_window_id())
        {
            window.buffer_name = name.to_string();
        }
        // After the layout lock is released: the current buffer name sits
        // before the layout in the canonical order, so taking it while
        // holding the layout would invert them.
        self.set_current_buffer_name(name);
    }

    // ---------------------------------------------------------------
    // Splitting, closing and cycling through windows
    // ---------------------------------------------------------------

    /// Split the focused window, and return the new window's id.
    ///
    /// Focus stays where it was, as it does in Emacs: `C-x 2` then typing
    /// continues in the window you were already in.
    /// Split the focused window, giving the two halves DIVISION.
    ///
    /// `Division::Ratio(0.5)` is the ordinary `C-x 2`/`C-x 3`. A caller that
    /// wants to keep a particular size -- a directory listing that should stay
    /// a fixed width while the file beside it takes the rest -- passes
    /// `Division::FirstFixed` instead, the first child being the window that
    /// was split.
    pub(crate) fn split_focused_window(
        &self,
        orientation: Orientation,
        division: Division,
    ) -> Option<usize> {
        let focused = self.get_focused_window_id();
        let new_id = self.get_next_window_id();
        let mut layout = self
            .layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root");
        // The new window shows the same buffer, scrolled the same way, so a
        // split looks like what it is: one view becoming two of the same
        // thing rather than a jump somewhere else.
        let existing = layout.window(focused)?.clone();
        let new_window = Window {
            id: new_id,
            ..existing
        };
        layout
            .split_window(focused, orientation, new_window, division)
            .then_some(new_id)
    }

    /// Close the focused window. Returns false when it is the only one.
    /// Open a full-width window of exactly HEIGHT rows at the bottom of the
    /// frame, showing BUFFER, and return its id.
    ///
    /// # What makes this different from a split
    ///
    /// It divides the *whole frame* rather than one window, so it appears below
    /// everything and every window above it gives up a share of the space. That
    /// is what a strip is: a thing the frame has, not a thing one window was
    /// cut in half to make.
    ///
    /// # Focus does not move
    ///
    /// Deliberately, and it is the property everything else rests on. A strip
    /// is shown *while something else is being typed into* -- completions
    /// beneath a prompt being the case this was built for -- and focus moving
    /// would make the strip's buffer current, so the next keystroke would be
    /// typed into the list of suggestions instead of into the prompt.
    pub(crate) fn open_bottom_window(&self, buffer: &str, height: usize) -> usize {
        let id = self.get_next_window_id();
        let window = Window {
            // No status line: the height asked for is the height of what the
            // caller wanted shown, and spending a row of it saying
            // "*Completions*" would make six mean five.
            show_mode_line: false,
            ..Window::new(id, buffer)
        };
        let mut layout = self
            .layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root");
        let existing = std::mem::replace(&mut *layout, LayoutNode::Leaf(window.clone()));
        *layout = LayoutNode::Split {
            orientation: Orientation::Horizontal,
            division: Division::SecondFixed(height),
            left: Box::new(existing),
            right: Box::new(LayoutNode::Leaf(window)),
        };
        id
    }

    /// Move the focused window's view by AMOUNT screenfuls, forwards when
    /// AMOUNT is positive, and drag point along if it would otherwise be left
    /// outside.
    ///
    /// Returns false when the view could not move at all -- already showing
    /// the end and asked to go forward, or the beginning and asked to go back
    /// -- so the caller can say so rather than leaving the key looking broken.
    ///
    /// # Why point moves second
    ///
    /// Scrolling and moving point are different things, and Emacs keeps them
    /// different: `C-v` moves the *view*, and point comes along only because it
    /// has to stay somewhere visible. Moving point first and letting the
    /// renderer's cursor-following do the scrolling would look similar and be
    /// wrong in the case that matters -- point would land at the window's edge
    /// rather than keeping its place on the screen.
    /// Scroll the focused window so that the line point is on sits `where_to`
    /// of the way down it: 0.0 the top row, 0.5 the middle, 1.0 the bottom.
    ///
    /// Point does not move. This is the opposite of `scroll_focused_window`,
    /// which moves the view and drags point along only when it would otherwise
    /// fall off the screen: here the cursor is the fixed thing and the text
    /// slides under it, which is what makes `C-l` a way of *looking* rather
    /// than a way of moving.
    ///
    /// False when nothing changed, so a caller can tell a no-op from a scroll.
    pub(crate) fn recenter_focused_window(&self, where_to: f64) -> bool {
        let id = self.get_focused_window_id();
        let Some(buffer) = self
            .focused_window_buffer()
            .and_then(|n| self.get_buffer(&n))
        else {
            return false;
        };
        let (line_count, point_line) = {
            let buf = buffer.read().expect("read lock on buffer");
            (buf.text.line_count(), buf.text.cursor_pos().0)
        };

        let mut layout = self
            .layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root");
        let Some(window) = layout.window_mut(id) else {
            return false;
        };
        // A window the layout has not drawn yet has no height; one row keeps
        // the arithmetic below sane. See `scroll_focused_window`.
        let height = window.text_height.max(1);
        let above = ((height - 1) as f64 * where_to.clamp(0.0, 1.0)).round() as usize;

        // Clamped at the top, and *not* at the bottom. Near the end of a file
        // there are not enough lines left to fill the window, and refusing to
        // scroll past that would make `C-l` do nothing for the last screenful
        // -- exactly where centring is most wanted. A few blank rows below the
        // last line is the price, and it is what Emacs shows too.
        let target = point_line
            .saturating_sub(above)
            .min(line_count.saturating_sub(1));
        if target == window.scroll_y {
            return false;
        }
        window.scroll_y = target;
        true
    }

    pub(crate) fn scroll_focused_window(&self, amount: isize) -> bool {
        let id = self.get_focused_window_id();
        let Some(buffer) = self
            .focused_window_buffer()
            .and_then(|n| self.get_buffer(&n))
        else {
            return false;
        };
        let (line_count, point_line, point_column) = {
            let buf = buffer.read().expect("read lock on buffer");
            let (line, column) = buf.text.cursor_pos();
            (buf.text.line_count(), line, column)
        };

        let mut layout = self
            .layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root");
        let Some(window) = layout.window_mut(id) else {
            return false;
        };
        // A window that has never been drawn has no height to scroll by: the
        // layout has not run, so nothing has worked one out. Treating it as one
        // row keeps every calculation below sane -- `bottom` does not run off
        // the bottom of zero, and `furthest` does not let the view past the end
        // by a line. It barely shows: the floor on `step` below already makes
        // such a window scroll, so what this changes is one line of travel, in
        // a state that ends with the first frame.
        let height = window.text_height.max(1);
        // Two lines of overlap, as Emacs keeps: a screenful with nothing in
        // common with the last one gives the reader nothing to place
        // themselves by. The floor matters for a genuinely tiny window -- one
        // or two rows -- where the overlap would otherwise be the whole of it
        // and the key would do nothing at all.
        let step = height.saturating_sub(NEXT_SCREEN_CONTEXT_LINES).max(1) as isize;
        // Far enough that the last line sits on the bottom row, and no
        // further. Past that the window fills with the blank space after the
        // end of the buffer -- a screen showing nothing at all, which the user
        // then has to scroll back out of by hand.
        let furthest = line_count.saturating_sub(height) as isize;
        let target = (window.scroll_y as isize + amount * step).clamp(0, furthest);
        if target == window.scroll_y as isize {
            return false;
        }
        window.scroll_y = target as usize;
        let top = target as usize;
        let bottom = top + height - 1;
        drop(layout);

        // Point only if it fell outside. Inside the new view it keeps the line
        // it was on, which is what makes two screenfuls of reading leave the
        // cursor where the eye left it.
        let last_line = line_count.saturating_sub(1);
        let moved_to = if point_line < top {
            Some(top)
        } else if point_line > bottom {
            Some(bottom.min(last_line))
        } else {
            None
        };
        if let Some(line) = moved_to {
            self.mutate_buffer(buffer, |buf| buf.text.cursor_move(line, point_column));
        }
        true
    }

    /// Close the window with ID, whoever has focus.
    ///
    /// Separate from `delete_focused_window` because a strip is closed by
    /// whatever put it there, which by then is running in a different window
    /// entirely -- giving the strip focus first, just to be able to close it,
    /// would make its buffer current and defeat the point of never focusing it.
    ///
    /// Returns false when there is no such window, or when it is the only one:
    /// a frame with no windows has nowhere to draw a cursor.
    pub(crate) fn delete_window_by_id(&self, id: usize) -> bool {
        let focused = self.get_focused_window_id();
        let survivor = {
            let mut layout = self
                .layout_root
                .write()
                .expect("Failed to acquire write lock on layout_root");
            if !layout.remove_window(id) {
                return false;
            }
            // Only when the window that went was the focused one. Closing a
            // strip must not move focus, which would change the current buffer
            // out from under whatever asked for the strip in the first place.
            (focused == id)
                .then(|| layout.window_ids().first().copied())
                .flatten()
        };
        if let Some(id) = survivor {
            self.set_focused_window_id(id);
        }
        true
    }

    pub(crate) fn delete_focused_window(&self) -> bool {
        let focused = self.get_focused_window_id();
        let survivor = {
            let mut layout = self
                .layout_root
                .write()
                .expect("Failed to acquire write lock on layout_root");
            if !layout.remove_window(focused) {
                return false;
            }
            // Focus is pointing at a window that no longer exists, so it has
            // to move before anything tries to draw a cursor in it.
            layout.window_ids().first().copied()
        };
        if let Some(id) = survivor {
            self.set_focused_window_id(id);
        }
        true
    }

    /// Close every window but the focused one.
    pub(crate) fn delete_other_windows(&self) -> bool {
        let focused = self.get_focused_window_id();
        self.layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root")
            .keep_only(focused)
    }

    /// Move focus COUNT windows on, wrapping round.
    ///
    /// A negative count goes the other way, which is what lets one command
    /// serve `C-x o` and a reversed `C-x o` alike.
    pub(crate) fn focus_other_window(&self, count: isize) -> bool {
        let ids = self
            .layout_root
            .read()
            .expect("Failed to acquire read lock on layout_root")
            .window_ids();
        if ids.len() < 2 {
            return false;
        }
        let here = ids
            .iter()
            .position(|id| *id == self.get_focused_window_id())
            .unwrap_or(0) as isize;
        let len = ids.len() as isize;
        // `rem_euclid` rather than `%`, so a negative count wraps round to the
        // end instead of producing a negative index.
        let there = (here + count).rem_euclid(len) as usize;
        self.set_focused_window_id(ids[there]);
        true
    }

    /// How many tiled windows the frame holds.
    pub(crate) fn window_count(&self) -> usize {
        self.layout_root
            .read()
            .expect("Failed to acquire read lock on layout_root")
            .window_ids()
            .len()
    }

    /// The buffer shown by the focused window, which is not always the current
    /// buffer: a floating prompt takes the current buffer without taking a
    /// tiled window's place.
    pub(crate) fn focused_window_buffer(&self) -> Option<String> {
        self.layout_root
            .read()
            .expect("Failed to acquire read lock on layout_root")
            .window(self.get_focused_window_id())
            .map(|window| window.buffer_name.clone())
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
        let window = Window::new(new_id, buf_name);

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

        // No `set_current_buffer_name` here: the float is in the list before
        // focus moves, so the chokepoint finds it and makes it current. Setting
        // it a second time would work today and rot the moment the two
        // disagree about what "the buffer of window N" means.
        self.set_focused_window_id(new_id);
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

    /// Install a keymap that is consulted before every other until it goes
    /// away. See [`TransientKeymap`].
    ///
    /// Replaces any map already installed rather than stacking: two maps
    /// competing for the same keystroke could not both win, and the newer one
    /// is always the more recent thing the user asked for.
    pub(crate) fn set_transient_keymap(&self, map: TransientKeymap<B>) {
        *self
            .transient_keymap
            .write()
            .expect("Failed to acquire write lock on transient_keymap") = Some(map);
        // Opened last, so nothing can get through the gate before the map it is
        // meant to find is there.
        self.transient_up.store(true, Ordering::Release);
    }

    /// Take the map down. Idempotent, so a command that ends one can call it
    /// without first asking whether one is up.
    pub(crate) fn clear_transient_keymap(&self) {
        // Closed first, for the mirror-image reason: no reader may be sent to a
        // map that is about to be taken away.
        self.transient_up.store(false, Ordering::Release);
        *self
            .transient_keymap
            .write()
            .expect("Failed to acquire write lock on transient_keymap") = None;
    }

    /// What the installed map wants shown, or empty when none is installed.
    pub(crate) fn transient_message(&self) -> String {
        if !self.transient_up.load(Ordering::Acquire) {
            return String::new();
        }
        self.transient_keymap
            .read()
            .expect("Failed to acquire read lock on transient_keymap")
            .as_ref()
            .map(|map| map.message.clone())
            .unwrap_or_default()
    }

    /// Whether a transient keymap is installed. Only for reporting.
    pub(crate) fn transient_keymap_active(&self) -> bool {
        self.transient_up.load(Ordering::Acquire)
    }

    // -----------------------------------------------------------------------
    // Completion sources
    // -----------------------------------------------------------------------
    //
    // A list per mode and one global list, reached through the five methods
    // below so that no caller has to know there are two places. `nil` means
    // the global list everywhere, exactly as it does in `define-key`.

    /// Every completion source to try in MODE, most specific first.
    ///
    /// The mode's own sources come before the global ones because a mode knows
    /// something the editor does not: in a Lisp buffer the interpreter's
    /// function names are the good answer and the words lying around in other
    /// buffers are the fallback, and only `risp-mode` is in a position to say
    /// so.
    ///
    /// Both lists are copied out and both locks released before the caller
    /// gets them. Every one of these is about to be *called*, and a source is
    /// arbitrary Lisp that may load a module, define a mode, or open a buffer
    /// -- the same rule `run_hook` is written to, for the same reason.
    pub(crate) fn completion_sources(&self, mode: &str) -> Vec<ELispExp<B>> {
        let mut sources: Vec<ELispExp<B>> = self
            .mode_registry
            .read()
            .expect("Failed to acquire read lock on mode_registry")
            .get(mode)
            .map(|mode| mode.completion_functions.clone())
            .unwrap_or_default();
        sources.extend(
            self.completion_functions
                .read()
                .expect("Failed to acquire read lock on completion_functions")
                .iter()
                .cloned(),
        );
        sources
    }

    /// Append a source to MODE's list, or to the global one when MODE is
    /// `None`. False if MODE names a mode that does not exist.
    pub(crate) fn add_completion_function(
        &self,
        mode: Option<&str>,
        function: ELispExp<B>,
    ) -> bool {
        match mode {
            None => {
                self.completion_functions
                    .write()
                    .expect("Failed to acquire write lock on completion_functions")
                    .push(function);
                true
            }
            Some(name) => self.with_mode_mut(name, |mode| mode.completion_functions.push(function)),
        }
    }

    /// Replace a whole list. This is how a source is removed or the order
    /// changed -- `(set-completion-functions nil (list ...))` -- which a
    /// bare `add` could not express.
    pub(crate) fn set_completion_functions(
        &self,
        mode: Option<&str>,
        functions: Vec<ELispExp<B>>,
    ) -> bool {
        match mode {
            None => {
                *self
                    .completion_functions
                    .write()
                    .expect("Failed to acquire write lock on completion_functions") = functions;
                true
            }
            Some(name) => self.with_mode_mut(name, |mode| mode.completion_functions = functions),
        }
    }

    /// One list on its own, unmerged, or `None` if MODE is unknown.
    pub(crate) fn completion_function_list(&self, mode: Option<&str>) -> Option<Vec<ELispExp<B>>> {
        match mode {
            None => Some(
                self.completion_functions
                    .read()
                    .expect("Failed to acquire read lock on completion_functions")
                    .clone(),
            ),
            Some(name) => self
                .mode_registry
                .read()
                .expect("Failed to acquire read lock on mode_registry")
                .get(name)
                .map(|mode| mode.completion_functions.clone()),
        }
    }

    /// Change one mode in the registry, reporting whether it was there.
    ///
    /// The write lock is taken and released inside, and the closure is given
    /// only the mode -- so nothing that runs under this lock can reach the
    /// interpreter.
    fn with_mode_mut(&self, name: &str, edit: impl FnOnce(&mut MajorMode<B>)) -> bool {
        match self
            .mode_registry
            .write()
            .expect("Failed to acquire write lock on mode_registry")
            .get_mut(name)
        {
            Some(mode) => {
                edit(mode);
                true
            }
            None => false,
        }
    }

    /// Say that a file whose name matches PATTERN opens in MODE.
    ///
    /// Without this a language module can be loaded and never selected: nothing
    /// else maps a file to a mode, so every grammar would have to be reached by
    /// hand.
    pub(crate) fn add_auto_mode(&self, pattern: regex::Regex, mode: &str) {
        self.auto_modes
            .write()
            .expect("Failed to acquire write lock on auto_modes")
            .push((pattern, mode.to_string()));
    }

    /// The mode a file called PATH should open in, if any pattern claims it.
    ///
    /// Matched against the whole path, so a pattern can key on a directory as
    /// well as an extension.
    pub(crate) fn auto_mode_for(&self, path: &str) -> Option<String> {
        self.auto_modes
            .read()
            .expect("Failed to acquire read lock on auto_modes")
            .iter()
            .find(|(pattern, _)| pattern.is_match(path))
            .map(|(_, mode)| mode.clone())
    }

    /// Say that COMMAND may be repeated by pressing KEYS on its own afterwards.
    ///
    /// Declared rather than inferred. The tempting rule -- "after a sequence
    /// ending in K, a bare K repeats" -- would make `C-x C-f` followed by `f`
    /// re-open `find-file`, which is not a convenience.
    pub(crate) fn set_repeat_key(&self, command: &str, key: KeyEvent) {
        self.repeat_keys
            .write()
            .expect("Failed to acquire write lock on repeat_keys")
            .insert(command.to_string(), key);
        self.any_repeat_keys.store(true, Ordering::Release);
    }

    /// The key that repeats COMMAND, if it has one.
    fn repeat_key(&self, command: &str) -> Option<KeyEvent> {
        if !self.any_repeat_keys.load(Ordering::Acquire) {
            return None;
        }
        self.repeat_keys
            .read()
            .expect("Failed to acquire read lock on repeat_keys")
            .get(command)
            .cloned()
    }

    /// Offer to repeat COMMAND, if it said it could be.
    ///
    /// Run after every command, which is what makes repeating work by the same
    /// path whether the command was reached by its full sequence or by the
    /// repeat key it offered last time.
    fn install_repeat_keymap(&self, command: Option<&str>) {
        let Some(key) = command.and_then(|name| self.repeat_key(name)) else {
            // Nothing to offer, and nothing to take down either: a `Release`
            // map was already dismissed by key resolution before this command
            // ran, which is what ends a run of `C-x o o o`.
            //
            // Taking one down here as well would be actively wrong. A `Refuse`
            // map -- a question -- is answered by *its own* commands, and this
            // runs after every one of them: clearing here would dismiss the
            // question the moment it was answered, before the next one could
            // be asked.
            return;
        };
        self.set_transient_keymap(TransientKeymap::repeating(
            key,
            command.expect("a repeat key was found, so there is a command"),
        ));
    }

    /// Start, or continue, an incremental search.
    pub(crate) fn begin_isearch(&self, session: Isearch) {
        *self
            .isearch
            .write()
            .expect("Failed to acquire write lock on isearch") = Some(session);
    }

    /// Take the running search *out* of the editor, leaving none behind.
    ///
    /// Out rather than borrowed, because acting on a session means taking
    /// buffer locks and moving point. Handing a `&mut` to a closure would mean
    /// holding this lock across all of that, putting an ordering between it and
    /// the buffers that nothing else in the editor respects. Taking it out owes
    /// no ordering to anything -- the caller puts it back with
    /// [`Self::begin_isearch`] when the search continues, and simply drops it
    /// when it does not.
    pub(crate) fn take_isearch(&self) -> Option<Isearch> {
        self.isearch
            .write()
            .expect("Failed to acquire write lock on isearch")
            .take()
    }

    /// Whether an incremental search is running. Only for reporting -- anything
    /// that acts on the session takes it.
    pub(crate) fn isearch_active(&self) -> bool {
        self.isearch
            .read()
            .expect("Failed to acquire read lock on isearch")
            .is_some()
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

        // A transient keymap is consulted before every other, for as long as
        // it is installed. See `TransientKeymap` -- and note that the gate is
        // an atomic, because on the overwhelming majority of keystrokes the
        // answer is "no map", and that answer costs a load rather than a lock.
        let transient = self.transient_up.load(Ordering::Acquire).then(|| {
            let map = self
                .transient_keymap
                .read()
                .expect("Failed to acquire read lock on transient_keymap");
            map.as_ref()
                .map(|map| {
                    (
                        map.keymap.get(&pending).cloned(),
                        map.keymap.is_prefix(&pending),
                        map.on_unbound,
                    )
                })
                // The gate was open but the map had gone. Only reachable if
                // something took it down between the two, which the
                // single-threaded key path does not do; treated as "no map",
                // which is the safe direction -- the key reaches the ordinary
                // keymaps rather than vanishing.
                .unwrap_or((None, false, OnUnbound::Release))
        });
        if let Some((bound, is_prefix, on_unbound)) = transient {
            match (bound, is_prefix, on_unbound) {
                (Some(ast), _, _) => {
                    pending.clear();
                    drop(pending);
                    return Some(ast);
                }
                // Part-way through one of the map's own sequences.
                (None, true, _) => {
                    drop(pending);
                    return None;
                }
                // Refused, and nothing said about it: the map's message is
                // still in the frame, and anything written to the echo area
                // would be drawn under it.
                (None, false, OnUnbound::Refuse) => {
                    pending.clear();
                    drop(pending);
                    return None;
                }
                // Handed on. The map goes away and the key carries on to the
                // keymaps below exactly as though it had never been there --
                // which is what makes the offer free to ignore.
                // Handed on, and the map stays. The key carries on to the
                // keymaps below and the map is consulted again next time,
                // which is what lets the completion strip be typed at without
                // either swallowing the letter or dismissing itself.
                (None, false, OnUnbound::Pass) => {}
                (None, false, OnUnbound::Release) => {
                    // The gate is closed first and the map dropped after, the
                    // same order `clear_transient_keymap` uses -- holding the
                    // bindings of a map nobody can reach would be a small leak
                    // that lasted until the next one was installed.
                    self.transient_up.store(false, Ordering::Release);
                    *self
                        .transient_keymap
                        .write()
                        .expect("Failed to acquire write lock on transient_keymap") = None;
                }
            }
        }

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
        self.run_command_form(ast, env);
    }

    /// Run AST as a command: one undo group, one `post-command-hook`, one
    /// entry in `last-command`.
    ///
    /// Split out of [`Self::handle_key_event`] because a keystroke is no
    /// longer the only thing that runs a command -- a bracketed paste does
    /// too, and it has to be grouped, hooked and remembered exactly as a
    /// keystroke is. Two dispatch paths would drift; one cannot.
    fn run_command_form(&self, mut ast: ELispExp<B>, env: &Arc<Env<EditorState<B>>>) {
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
        // Before `post-command-hook', and only for the command that typed a
        // character. This is where electric-pair and anything else that
        // reacts to typing hangs; running it after the general hook would put
        // the pair in after a mode had already looked at the line.
        //
        // Deliberately *not* run for a paste, which reaches the buffer as
        // `insert-pasted-text' rather than as a run of `self-insert'. A hook
        // that fires per character would auto-pair every bracket in pasted
        // code, which is exactly the mangling bracketed paste exists to stop.
        if this_command.as_deref().map(String::as_str) == Some("self-insert") {
            self.run_hook(&current_mode_name, "post-self-insert-hook", env);
        }
        self.run_hook(&current_mode_name, "post-command-hook", env);
        // Offered after the command has run and its hooks have fired, so that
        // pressing the repeat key goes through every step the first invocation
        // did. A command with no repeat key takes down whatever the previous
        // one offered, which is what ends a run of `C-x o o o`.
        // Remembered only for a command that is not itself `repeat'. Were
        // `repeat' to overwrite this with its own form, the second press would
        // repeat the repeating rather than the thing repeated, and every press
        // after that would too.
        if this_command.as_deref().map(String::as_str) != Some("repeat") {
            *self
                .last_command_form
                .write()
                .expect("Failed to acquire write lock on last_command_form") = Some(ast);
        }
        self.install_repeat_keymap(this_command.as_deref().map(String::as_str));
        self.set_last_command(this_command);
    }

    /// Insert TEXT as a single bracketed paste.
    ///
    /// # Why paste is not typing
    ///
    /// Without bracketed paste a terminal delivers a paste as the keystrokes
    /// it looks like, and the editor cannot tell the difference: N characters
    /// become N `self-insert' commands, N undo entries, N runs of every hook.
    /// Pasting a function and then pressing `undo' would walk back through it
    /// one character at a time, and auto-pairing would double every bracket in
    /// it.
    ///
    /// The terminal knows the difference and says so, so this takes the whole
    /// paste as one command: one undo group, one syntax invalidation, one
    /// `post-command-hook', and no `post-self-insert-hook' at all.
    pub fn handle_paste(&self, text: String, env: &Arc<Env<EditorState<B>>>) {
        if text.is_empty() {
            return;
        }
        self.run_command_form(
            ELispExp::form(vec![
                ELispExp::symbol("insert-pasted-text".into()),
                ELispExp::string(text),
            ]),
            env,
        );
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
        let mut hooks: Vec<ELispExp<B>> = self
            .mode_registry
            .read()
            .expect("Failed to acquire read lock on mode registry")
            .get(mode_name)
            .and_then(|mode| mode.hooks.get(hook_name))
            .cloned()
            .unwrap_or_default();
        // The mode's own first, then the ones registered for every mode. A
        // mode-specific hook is the more specific statement about this buffer,
        // so it gets to act before anything general reacts to the result.
        hooks.extend(
            self.global_hooks
                .read()
                .expect("Failed to acquire read lock on global hooks")
                .get(hook_name)
                .cloned()
                .unwrap_or_default(),
        );

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
        }

        // *Every* tiled window showing it, not only the focused one.
        //
        // # The bug this fixes
        //
        // This used to repoint the focused window alone. Open one buffer in
        // two windows, kill it, and the other window was left naming a buffer
        // that no longer existed. Nothing complained until focus moved there
        // -- at which point `current_buffer_name` became that dead name and
        // the next `get_current_buffer` hit its "Corruption in the hashmap of
        // buffers" panic, taking the editor down with whatever was unsaved in
        // the other windows.
        //
        // Worked out before the layout lock is taken: `most_recent_buffer`
        // reads `buffer_recency` and the buffer table, and taking those while
        // holding the layout would invert the canonical lock order.
        let replacement = self
            .most_recent_buffer(name)
            .unwrap_or_else(|| "*scratch*".to_string());
        self.layout_root
            .write()
            .expect("Failed to acquire write lock on layout_root")
            .each_window_mut(&mut |window| {
                if window.buffer_name == name {
                    window.buffer_name = replacement.clone();
                }
            });

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
            // The same buffer the windows were repointed at, so that what is
            // current and what is on screen agree. They disagreeing is how the
            // panic above was reached in the first place.
            let fallback = if self.get_buffer(&replacement).is_some() {
                replacement
            } else {
                "*scratch*".to_string()
            };
            self.set_current_buffer_name(&fallback);
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
        let prompt = self.transient_message();
        let mode_line_format = mode_line_format(env);
        let separator_char = window_separator(env);
        // Asked before the structural locks, alongside the other independent
        // reads: it takes the buffers and the mode registry, and the registry
        // has no place in the ordering that begins below.
        let colouring_pending = self.colouring_pending();

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
        let mut separator_rects = Vec::new();
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
            &mut separator_rects,
        );

        let separators: Vec<Separator> = separator_rects
            .into_iter()
            .map(|rect| Separator {
                rect,
                ch: separator_char,
                face: Face::WINDOW_SEPARATOR,
            })
            .collect();

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
            prompt,
            theme,
            focused_window_id,
            width: screen_width,
            height: screen_height,
            colouring_pending,
            separators,
            clipboard: self.take_pending_clipboard(),
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

    /// How long a renderer may wait for input before the screen it has just
    /// drawn will want drawing again, or `None` when it may wait indefinitely.
    ///
    /// # Why the renderer asks rather than decides
    ///
    /// Almost everything on screen changes because the user did something, so a
    /// renderer can draw, block on input, and be right. Two things do not: an
    /// echo message that expires on a timer, and colour that arrives from the
    /// highlighter's thread. A renderer blocked on input sleeps through both.
    ///
    /// Which of those are outstanding, and how soon each matters, are questions
    /// about the editor, not about drawing -- so they are answered here and the
    /// renderer is handed a single duration. A second frontend gets the
    /// behaviour by asking the same question, rather than by remembering to
    /// reimplement two special cases.
    ///
    /// FRAME is the one just drawn, because the question is whether *that*
    /// frame goes stale. Taking it from the frame also means the colouring is
    /// not asked about twice per redraw, once to compose and once to wait.
    ///
    /// `None` is the ordinary answer, and it is the one that matters: with
    /// nothing being coloured and no message pending there is no wake-up at
    /// all, so an idle editor costs nothing.
    pub fn next_redraw_in(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        frame: &FrameSnapshot,
    ) -> Option<Duration> {
        // One more turn is the soonest new colour can appear, so it is the
        // longest this may sleep without being late for it.
        let colouring = frame.colouring_pending.then_some(TURN_INTERVAL);
        // Output from a running command arrives without anybody pressing a
        // key, so the renderer has to come back and look. Same interval as
        // colouring for the same reason: it is short enough to read as live
        // and long enough to cost nothing.
        let shell = (self.shell_commands_running() > 0).then_some(TURN_INTERVAL);
        [self.echo_expiry_in(env), colouring, shell]
            .into_iter()
            .flatten()
            .min()
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
        Some((
            pending.name.clone(),
            done + 1,
            done + pending.remaining.len(),
        ))
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
    /// Put every face back to how the editor ships it.
    ///
    /// The compiled-in defaults rather than a snapshot taken at startup, so
    /// this means the same thing however many themes have been applied since.
    pub(crate) fn reset_theme(&self) {
        *self
            .theme
            .write()
            .expect("Failed to acquire write lock on theme") = Arc::new(Theme::default());
    }

    pub(crate) fn set_face_style(&self, face: Face, style: Style) {
        // Copy-on-write: whatever frames are already holding keep the theme
        // they were composed under, and the next one picks this up.
        let mut theme = self
            .theme
            .write()
            .expect("Failed to acquire write lock on theme");
        Arc::make_mut(&mut theme).set(face, style);
    }

    pub(crate) fn face_style(&self, face: Face) -> Style {
        self.theme
            .read()
            .expect("Failed to acquire read lock on theme")
            .style(face)
    }

    pub(crate) fn theme(&self) -> Arc<Theme> {
        self.theme
            .read()
            .expect("Failed to acquire read lock on theme")
            .clone()
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
    pub(crate) fn kill(&self, text: String, direction: Direction, env: &Arc<Env<Self>>) {
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
        // The system clipboard gets whatever the ring now holds, not the
        // fragment that just arrived: a run of `C-k` is one kill as far as the
        // user is concerned, and sending each line on its own would leave the
        // clipboard holding the last line of a passage they meant to take
        // whole. `current` is the entry `yank` would insert, which is exactly
        // the promise the clipboard should be making.
        if Self::clipboard_sync_enabled(env)
            && let Some(current) = ring.current()
        {
            *self
                .pending_clipboard
                .write()
                .expect("Failed to acquire write lock on pending_clipboard") =
                Some(current.to_string());
        }
    }

    /// Whether killed text should also reach the system clipboard.
    ///
    /// Unbound means no. The variable is set by `clipboard.lisp`, so the
    /// editor comes up with it on; a harness that loads no Lisp -- which is
    /// every test in this crate -- gets the old behaviour untouched rather
    /// than queueing a clipboard payload on every kill it makes.
    fn clipboard_sync_enabled(env: &Arc<Env<Self>>) -> bool {
        env.get_variable("clipboard-sync")
            .is_some_and(|flag| flag.is_truthy())
    }

    /// Take the text owed to the system clipboard, leaving nothing behind.
    ///
    /// Called once per frame by [`Self::snapshot`]. Taking rather than reading
    /// is what stops a redraw of an unchanged frame from re-sending the same
    /// escape.
    pub(crate) fn take_pending_clipboard(&self) -> Option<String> {
        self.pending_clipboard
            .write()
            .expect("Failed to acquire write lock on pending_clipboard")
            .take()
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

    /// Note that a shell command has started.
    pub(crate) fn begin_shell_command(&self) {
        self.shell_commands.fetch_add(1, Ordering::Relaxed);
    }

    /// Note that one has finished. Called from the worker thread, after the
    /// last of its output is in the buffer.
    pub(crate) fn finish_shell_command(&self) {
        // Saturating rather than wrapping: a stray extra call would otherwise
        // take the count to `usize::MAX` and leave the renderer spinning for
        // the rest of the session.
        let _ = self
            .shell_commands
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_sub(1))
            });
    }

    /// How many shell commands are still running.
    pub(crate) fn shell_commands_running(&self) -> usize {
        self.shell_commands.load(Ordering::Relaxed)
    }

    /// Register FUNCTION to run under HOOK_NAME in every major mode.
    pub(crate) fn add_global_hook(&self, hook_name: &str, function: ELispExp<B>) {
        self.global_hooks
            .write()
            .expect("Failed to acquire write lock on global hooks")
            .entry(hook_name.to_string())
            .or_default()
            .push(function);
    }

    /// The last command as a runnable form, for `repeat`.
    pub(crate) fn last_command_form(&self) -> Option<ELispExp<B>> {
        self.last_command_form
            .read()
            .expect("Failed to acquire read lock on last_command_form")
            .clone()
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

    /// Move focus to window ID, and make what it shows the current buffer.
    ///
    /// # Why the two move together
    ///
    /// They are one idea with two representations. The focused window is what
    /// the cursor is drawn in; the current buffer is what a command edits. Let
    /// them disagree and the editor draws a cursor in one place and types in
    /// another -- `C-x o` then `C-n` moves point in the buffer you just left,
    /// so the cursor you *can* see sits perfectly still while text you cannot
    /// see scrolls past.
    ///
    /// That was the bug, and it is fixed here rather than in `other-window`
    /// because there are four ways to move focus -- cycling, closing a window,
    /// opening a floating one, and closing it again -- and any of them could
    /// have forgotten. Nothing can move focus without going through this.
    /// Give focus to the window with ID, if there is still one.
    ///
    /// False when there is not, which is the useful answer rather than a
    /// failure: a window remembered earlier may have been closed since, and
    /// the caller wants to open a new one rather than be stopped.
    pub(crate) fn select_window(&self, id: usize) -> bool {
        let exists = self
            .layout_root
            .read()
            .expect("Failed to acquire read lock on layout_root")
            .window(id)
            .is_some()
            || self
                .floating_windows
                .read()
                .expect("Failed to acquire read lock on floating_windows")
                .iter()
                .any(|float| float.window.id == id);
        if exists {
            self.set_focused_window_id(id);
        }
        exists
    }

    pub(crate) fn set_focused_window_id(&self, id: usize) {
        *self
            .focused_window_id
            .write()
            .expect("Failed to acquire write lock on focused_window_id") = id;
        // Released before the lookup below: that takes the layout and the
        // floating windows, which sit *after* this one in the lock ordering.
        if let Some(name) = self.window_buffer(id) {
            self.set_current_buffer_name(&name);
        }
    }

    /// What window ID is showing, whether it is tiled or floating.
    ///
    /// Both, because a minibuffer prompt is a floating window and giving it
    /// focus has to make its buffer current in exactly the same way -- that is
    /// what every prompt in the editor depends on.
    fn window_buffer(&self, id: usize) -> Option<String> {
        if let Some(window) = self
            .layout_root
            .read()
            .expect("Failed to acquire read lock on layout_root")
            .window(id)
        {
            return Some(window.buffer_name.clone());
        }
        self.floating_windows
            .read()
            .expect("Failed to acquire read lock on floating_windows")
            .iter()
            .find(|float| float.window.id == id)
            .map(|float| float.window.buffer_name.clone())
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
        self.record_buffer_use(name);
    }

    /// Move NAME to the front of the recency list.
    fn record_buffer_use(&self, name: &str) {
        let mut recency = self
            .buffer_recency
            .write()
            .expect("Failed to acquire write lock on buffer_recency");
        if recency.first().is_some_and(|first| first == name) {
            // Already the most recent, which is the common case by far: this
            // runs whenever a buffer becomes current, `with-current-buffer'
            // included, so it is worth not touching the list at all.
            return;
        }
        recency.retain(|existing| existing != name);
        recency.insert(0, name.to_string());
    }

    /// The most recently current buffer that still exists and is not EXCEPT.
    ///
    /// `None` when there is no such buffer. The caller answers that for
    /// itself -- there is always `*scratch*`, but falling back to it is a
    /// policy this does not get to make.
    pub(crate) fn most_recent_buffer(&self, except: &str) -> Option<String> {
        let recency = self
            .buffer_recency
            .read()
            .expect("Failed to acquire read lock on buffer_recency");
        recency
            .iter()
            .find(|name| name.as_str() != except && self.get_buffer(name).is_some())
            .cloned()
    }

    /// Make NAME the current buffer without showing it, and give back whatever
    /// was current before.
    ///
    /// This is Emacs' `set-buffer`, and the difference from `switch-to-buffer`
    /// is the whole reason it exists: that one also points the focused window
    /// at the buffer, which is right when a person asked to see it and wrong
    /// when a piece of Lisp merely wants to *act* on it. Rebinding the window
    /// for the duration of a computation and putting it back would be a window
    /// doing something nobody asked for.
    ///
    /// Returns `None` when there is no such buffer, having changed nothing.
    pub(crate) fn set_current_buffer(&self, name: &str) -> Option<Arc<str>> {
        self.get_buffer(name)?;
        let previous = self.current_buffer_name_shared();
        self.set_current_buffer_name(name);
        Some(previous)
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

    /// The syntax table for MODE, or the default one when it has none.
    ///
    /// Cloned out and the lock released, because every caller is about to run a
    /// scan with it  and a scan reads a whole buffer, far too long to hold the
    /// registry against everything else that wants a mode.
    pub(crate) fn syntax_table(&self, mode: &str) -> SyntaxTable {
        self.mode_registry
            .read()
            .expect("Failed to acquire read lock on mode_registry")
            .get(mode)
            .and_then(|mode| mode.syntax_table.clone())
            .unwrap_or_default()
    }

    /// The syntax table in force in the current buffer.
    pub(crate) fn current_syntax_table(&self) -> SyntaxTable {
        let mode = self
            .get_current_buffer()
            .read()
            .expect("Failed to acquire read lock on buffer")
            .current_mode
            .clone();
        self.syntax_table(&mode)
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
    // timeout; nil leaves messages up until something replaces them.
    env.set_variable(
        ECHO_MESSAGE_TIMEOUT.into(),
        ELispExp::number(DEFAULT_ECHO_MESSAGE_TIMEOUT),
    );

    // How large a prompt is. Set here so that `(setq minibuffer-width 40)` in a
    // configuration is an adjustment to a value that already exists, rather
    // than the thing that brings the setting into being.
    env.set_variable(
        MINIBUFFER_WIDTH.into(),
        ELispExp::number(DEFAULT_MINIBUFFER_WIDTH),
    );
    env.set_variable(
        MINIBUFFER_HEIGHT.into(),
        ELispExp::number(DEFAULT_MINIBUFFER_HEIGHT),
    );

    // What each window's status line shows.
    env.set_variable(
        MODE_LINE_FORMAT.into(),
        ELispExp::string(DEFAULT_MODE_LINE_FORMAT.to_string()),
    );

    // The char used to draw vertical windows separators.
    env.set_variable(
        WINDOW_SEPARATOR.into(),
        ELispExp::string(DEFAULT_WINDOW_SEPARATOR.to_string()),
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
    install_isearch(&editor_state, env.clone());

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
;; Add your configuration here.
;;
;; The order matters: each of these uses what the ones above it define.
;; `commands' defines `defcommand', which the modules below are written with,
;; and `debug' defines `message', which they report through.
(eval-file "commands")
(eval-file "debug")
(eval-file "common-keymaps")
(eval-file "indent")      ; what Tab does; a mode plugs its own rule in
(eval-file "minibuffer")

;; Modules. Each is optional -- comment one out and the editor comes up
;; without it, missing exactly that feature and nothing else.
(eval-file "rust-mode")   ; colouring for Rust source
(eval-file "risp-mode")   ; colouring and indentation for this editor's own Lisp
(eval-file "dired")       ; a directory in a buffer (C-x d)
(eval-file "completion")  ; Tab shows every candidate at once, in a strip
(eval-file "clipboard")   ; kills also go to the system clipboard
(eval-file "electric-pair") ; typing "(" gives you "()"
(eval-file "buffer-list")  ; C-x b to switch, C-x C-b for the whole list
(eval-file "shell")       ; M-! runs a command and shows what it said
(eval-file "manpage")     ; C-h m, and K on a word
(eval-file "compile")     ; C-c c, and M-g n to walk what it complained about
(eval-file "theme")       ; C-c t to choose how faces are drawn

;; Where completions come from, for C-M-i in a buffer. The command is built in
;; and works without this; what this adds is the five sources it asks. Take one
;; out with `set-completion-functions', or add one of your own for a single
;; mode with `add-completion-function'.
(eval-file "completion-at-point")


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
