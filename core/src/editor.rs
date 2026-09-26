use crate::{
    ELispExp,
    buffer::{Buffer, BufferTrait},
    commands::{ArgSpec, Invocation, PrefixArg},
    input::{
        KeyEvent, Keymap, MouseButton, MouseEvent, MouseKind, OnUnbound, TransientKeymap,
        describe_keys, fill_default_keymaps,
    },
    isearch::install_isearch,
    kill_ring::Direction,
    lisp::{
        DEFAULT_FUEL, Env, EvalError, FuelMeter, FuelScope, LispContext, Parser, bootstrap_vm, eval,
    },
    managers::{
        BufferRemoved, Buffers, Commands, Hit, KillYank, Log, Modes, MouseDrag, Runtime, Scrolled,
        WindowRemoved, Windows,
    },
    minibuffer::install_minibuffer,
    modes::highlighter::{Highlighter, TURN_INTERVAL},
    modes::prescan::Prescanner,
    modes::{MajorMode, SyntaxTable},
    primitives::{edits::goto_offset, install_primitives},
    search::Isearch,
    task::{BackgroundScheduler, WorkerMessage},
    ui::{
        Division, Face, FloatingWindow, Focus, FrameSnapshot, Orientation, Rect,
        RenderableWindowView, Separator, Side, Style, Theme, Window, WindowId,
        extract_buffer_lines, region_highlights,
    },
};
use std::{
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

pub const MOUSE_MODE: &str = "mouse-mode";

/// Whether the editor is reading the mouse, as Lisp currently defines it.
///
/// Off unless something says otherwise, and the default `init.lisp` says
/// otherwise. It is a setting at all because it costs something: a terminal
/// reporting the mouse to the editor is not using it for its own selection, so
/// turning this on trades the terminal's copy-and-paste for the editor's. That
/// is the right trade for most people and an unpleasant surprise for anyone
/// who was never asked -- which is why upgrading does not make it for them.
pub fn mouse_mode<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.get_variable(MOUSE_MODE)
        .is_some_and(|value| !value.is_nil())
}

/// What the keymaps had to say about the key sequence typed so far.
///
/// Four answers rather than the `(Option<ELispExp>, bool)` pair this replaced.
/// That pair could represent `(Some(command), true)` -- bound *and* a prefix --
/// which is not a thing a keymap can mean, and it had no way at all to say
/// "bound to nothing, and say nothing about it", which the transient map's
/// `Refuse` needs; that case had to return early from the middle of the lookup
/// instead.
enum Bound<B: BufferTrait> {
    /// Run this.
    Command(ELispExp<B>),
    /// Part-way through a sequence. The keys are kept.
    Prefix,
    /// Bound to nothing, and nothing is to be said about it.
    Refused,
    /// Bound to nothing. Say so.
    Unbound,
}

/// A command form with numeric arguments, built rather than parsed.
///
/// Built, because the arguments are numbers the editor just worked out: going
/// through the parser would mean formatting them into text for it to read back.
fn mouse_form<B: BufferTrait>(name: &str, args: &[f64]) -> ELispExp<B> {
    let mut items = vec![ELispExp::symbol(name.into())];
    items.extend(args.iter().copied().map(ELispExp::number));
    ELispExp::form(items)
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
    running: Arc<AtomicBool>,
    /// A channel that is used to send work to a worker thread like
    /// the syntax highlighting computation
    worker_mailbox: Sender<WorkerMessage<B>>,

    /// Every buffer the editor holds, which one is current, and which were
    /// current lately. See [`Buffers`], which says why those are one lock and
    /// not three.
    ///
    /// Private, like [`EditorState::windows`]'s field, and for the same
    /// reason: the invariant that `current` names a buffer the table holds is
    /// only an invariant while nothing outside can set one without the other.
    buffers: Arc<RwLock<Buffers<B>>>,
    /// The echo area's text together with when it was set, in one lock so a
    /// reader can never pair a new message with an old timestamp.
    echo_message: Arc<RwLock<EchoMessage>>,

    /// What a mode is, what keys do, and where completions come from: the
    /// registry, the global keymap, the global completion list, the file-name
    /// patterns, the transient keymap and the repeat keys. See [`Modes`],
    /// which says why those are one lock and not eight.
    modes: Arc<RwLock<Modes<B>>>,
    /// Every window the frame has: the tiled tree, the floats drawn over it,
    /// which one has focus, the next id to hand out and what a held mouse
    /// button is doing. See [`Windows`], which says why those are one lock and
    /// not five.
    ///
    /// Private, and reached only through [`EditorState::windows`] and
    /// [`EditorState::windows_mut`]. That is the whole point of the
    /// compartment: a caller that cannot name the lock cannot take it out of
    /// order, cannot hold it across a call into the interpreter, and cannot
    /// leave focus naming a window the layout has already removed.
    windows: Arc<RwLock<Windows>>,

    /// What can be invoked by name, what is part-way through being invoked,
    /// and what was invoked last. See [`Commands`], which says why those are
    /// one lock and not five.
    commands: Arc<RwLock<Commands<B>>>,

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

    /// Killed text, what a yank put where, and whether the command before
    /// this one did either. See [`KillYank`], which says why those are one
    /// lock and not seven.
    kill_yank: Arc<RwLock<KillYank>>,

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

    /// Diagnostics, and the file they are mirrored to when one is enabled.
    /// See [`Log`].
    log: Arc<RwLock<Log>>,

    /// The execution budget, the call stack, the theme, where vertical
    /// movement is aiming and the search in progress. See [`Runtime`] -- which
    /// is candid about grouping by lifetime rather than by a shared invariant.
    runtime: Arc<RwLock<Runtime>>,
}

impl<B: BufferTrait> LispContext for EditorState<B> {
    fn consume_fuel(&self, amount: u32) -> Result<(), EvalError<EditorState<B>>> {
        // The meter reports a host-agnostic `Exhausted`; naming it as a Lisp
        // error is the host's job, which is the point of the split.
        self.runtime(|runtime| runtime.fuel())
            .consume(amount)
            .map_err(|_| EvalError::OutOfFuel)
    }

    fn log_diagnostic(&self, msg: &str) {
        // The sink comes back out with the lock, and the write happens with it
        // given back: a disk write inside the lock every diagnostic takes is a
        // queue everything else logging has to wait in, and diagnostics are
        // logged from the worker thread as well as this one.
        let sink = self.log.write().expect("write lock on log").record(msg);
        if let Some(file) = sink {
            file.write()
                .expect("Failed to acquire write lock on log file")
                .write_all(format!("{msg}\n").as_bytes())
                .expect("Failed to write into log file");
        }
    }

    fn begin_unwind(&self) {
        // Roughly a hundredth of a command's budget: ample for closing a file
        // or restoring a variable, far too little to hide a runaway loop.
        self.runtime(|runtime| runtime.fuel()).grant(100_000);
    }

    fn begin_thread_evaluation(&self) {
        self.runtime(|runtime| runtime.fuel()).arm_thread();
    }

    fn push_call_frame(&self, frame: &str) {
        self.runtime_mut(|runtime| runtime.push_call_frame(frame));
    }

    fn pop_call_frame(&self) {
        self.runtime_mut(|runtime| runtime.pop_call_frame());
    }

    fn call_frame_depth(&self) -> usize {
        self.runtime(|runtime| runtime.call_frame_depth())
    }

    fn truncate_call_frames(&self, depth: usize) {
        self.runtime_mut(|runtime| runtime.truncate_call_frames(depth));
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
        let mut keymaps = Keymap::new();
        fill_default_keymaps(&mut keymaps);

        // Create a secondary worker thread and the communication channels with it.
        let (sender, receiver) = std::sync::mpsc::channel();

        let editor_state = Self {
            running: Arc::new(AtomicBool::new(true)),
            worker_mailbox: sender,
            buffers: Arc::new(RwLock::new(Buffers::default())),
            echo_message: Arc::new(RwLock::new(EchoMessage::new("Welcome to rsedit"))),
            modes: Arc::new(RwLock::new(Modes::new(keymaps))),
            windows: Arc::new(RwLock::new(Windows::default())),
            commands: Arc::new(RwLock::new(Commands::default())),
            shell_commands: Arc::new(AtomicUsize::new(0)),
            kill_yank: Arc::new(RwLock::new(KillYank::default())),
            pending_keys: Arc::new(RwLock::new(Vec::new())),
            runtime: Arc::new(RwLock::new(Runtime::new(Arc::new(FuelMeter::new(
                DEFAULT_FUEL,
            ))))),
            log: Arc::new(RwLock::new(Log::default())),
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
    /// Start mirroring diagnostics to a file at PATH, writing out everything
    /// logged so far first.
    ///
    /// `&self` rather than `&mut self`: the file used to be the one field here
    /// not behind a lock, which meant enabling it needed exclusive access to
    /// the whole editor -- and, because `EditorState` is `Clone`, that a clone
    /// made afterwards was the only one that mirrored anything.
    pub fn enable_log_file<P: AsRef<std::path::Path>>(&self, path: P) -> std::io::Result<()> {
        // TODO(uncertain) maybe this is an unwanted change, i don't know if it's better to be
        // able to enable the writing of the logs only at specific times and maybe disable it
        // to get only some logs.
        let file = File::create(path)?;
        self.log
            .write()
            .expect("write lock on log")
            .enable_file(file)
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

                    self.buffers_mut(|buffers| buffers.insert(name, new_buf));
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
            self.buffers_mut(|buffers| buffers.insert(name, new_buf));
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
        self.buffers_mut(|buffers| buffers.insert(name, new_buf));
        self.show_in_focused_window(name);
        name.to_string()
    }

    /// Make the buffer named NAME the one shown in the focused window and
    /// the current buffer. Returns `false` (logging a diagnostic) if no
    /// buffer named NAME exists, `true` otherwise. Shared by the
    /// `switch-to-buffer` primitive and the built-in minibuffer's cleanup.
    pub(crate) fn switch_to_buffer(&self, name: &str) -> bool {
        if !self.has_buffer(name) {
            self.log_diagnostic(&format!("[LOG] buffer {} does not exist.", name));
            return false;
        }
        self.show_in_focused_window(name);
        true
    }

    // ---------------------------------------------------------------
    // The window compartment
    // ---------------------------------------------------------------

    /// Ask the windows something.
    ///
    /// A closure rather than a returned guard, so the lock cannot outlive the
    /// question. Every deadlock this file has had came from a guard living
    /// longer than the line that needed it -- read into a second lock, held
    /// across a call into Lisp, taken twice on one thread. A closure makes
    /// each of those a thing you have to write on purpose.
    ///
    /// The one rule for what goes inside: **no call back into `self`**. A
    /// method on the editor may take this lock again, and `RwLock` does not
    /// promise that a second read on one thread succeeds -- a writer waiting
    /// between the two is entitled to make it wait forever. Copy out, let go,
    /// then ask.
    pub(crate) fn windows<R>(&self, f: impl FnOnce(&Windows) -> R) -> R {
        f(&self.windows.read().expect("read lock on windows"))
    }

    /// Change the windows. The same rule applies, and more sharply.
    pub(crate) fn windows_mut<R>(&self, f: impl FnOnce(&mut Windows) -> R) -> R {
        f(&mut self.windows.write().expect("write lock on windows"))
    }

    // ---------------------------------------------------------------
    // The buffer compartment
    // ---------------------------------------------------------------

    /// Ask the buffer table something. Same rules as [`EditorState::windows`].
    pub(crate) fn buffers<R>(&self, f: impl FnOnce(&Buffers<B>) -> R) -> R {
        f(&self.buffers.read().expect("read lock on buffers"))
    }

    /// Change the buffer table -- add one, remove one, move what is current.
    pub(crate) fn buffers_mut<R>(&self, f: impl FnOnce(&mut Buffers<B>) -> R) -> R {
        f(&mut self.buffers.write().expect("write lock on buffers"))
    }

    /// Read the buffer named NAME. `None` when there is no such buffer.
    ///
    /// # Why the table's lock is not held while F runs
    ///
    /// The handle is cloned out and the table's lock given back *before* the
    /// buffer's is taken. Holding both would make the table a bottleneck on
    /// every keystroke: the highlighter and the prescanner walk it from their
    /// own threads, and a scan that had to wait for whoever was typing -- or a
    /// keystroke that had to wait for a scan -- is exactly what those threads
    /// exist to avoid.
    ///
    /// The closure gets a locked buffer and cannot keep it. That is the whole
    /// difference from the `get_buffer` this replaced, which handed back an
    /// `Arc` and left every caller to remember which lock to take and when to
    /// let it go.
    pub(crate) fn with_buffer<R>(&self, name: &str, f: impl FnOnce(&Buffer<B>) -> R) -> Option<R> {
        let handle = self.buffers(|buffers| buffers.handle(name))?;
        let guard = handle.read().expect("read lock on buffer");
        Some(f(&guard))
    }

    /// Change the buffer named NAME. `None` when there is no such buffer.
    pub(crate) fn with_buffer_mut<R>(
        &self,
        name: &str,
        f: impl FnOnce(&mut Buffer<B>) -> R,
    ) -> Option<R> {
        let handle = self.buffers(|buffers| buffers.handle(name))?;
        let mut guard = handle.write().expect("write lock on buffer");
        Some(f(&mut guard))
    }

    /// Read the current buffer.
    ///
    /// Infallible, unlike [`EditorState::with_buffer`]: there is always a
    /// current buffer, and [`Buffers`] is what makes that true rather than
    /// hopeful.
    pub(crate) fn with_current_buffer<R>(&self, f: impl FnOnce(&Buffer<B>) -> R) -> R {
        let handle = self.buffers(|buffers| buffers.current_handle());
        let guard = handle.read().expect("read lock on buffer");
        f(&guard)
    }

    /// Change the current buffer.
    pub(crate) fn with_current_buffer_mut<R>(&self, f: impl FnOnce(&mut Buffer<B>) -> R) -> R {
        let handle = self.buffers(|buffers| buffers.current_handle());
        let mut guard = handle.write().expect("write lock on buffer");
        f(&mut guard)
    }

    /// Whether a buffer named NAME exists.
    pub(crate) fn has_buffer(&self, name: &str) -> bool {
        self.buffers(|buffers| buffers.contains(name))
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
        self.windows_mut(|windows| windows.show_in_focused(name));
        // After the window lock is released: the current buffer name sits
        // before the windows in the canonical order, so taking it while
        // holding them would invert the two.
        self.set_current_buffer_name(name);
    }

    // ---------------------------------------------------------------
    // Splitting, closing and cycling through windows
    // ---------------------------------------------------------------

    /// Split the focused window, giving the two halves DIVISION, and return
    /// the new window's id. See [`Windows::split_focused`].
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
    ) -> Option<WindowId> {
        self.windows_mut(|windows| windows.split_focused(orientation, division))
    }

    /// Open a full-width window of exactly HEIGHT rows at the bottom of the
    /// frame, showing BUFFER, and return its id.
    ///
    /// Focus does not move, which is the property everything else rests on.
    /// See [`Windows::open_bottom`] for why.
    pub(crate) fn open_bottom_window(&self, buffer: &str, height: usize) -> WindowId {
        self.windows_mut(|windows| windows.open_bottom(buffer, height))
    }

    /// Scroll the focused window so that the line point is on sits `where_to`
    /// of the way down it, without moving point. See
    /// [`Windows::recenter_focused`].
    ///
    /// The coordination here is reading the buffer first: the compartment is
    /// told the two numbers it needs rather than being handed the buffer, so
    /// the two locks are never held together.
    ///
    /// False when nothing changed, so a caller can tell a no-op from a scroll.
    pub(crate) fn recenter_focused_window(&self, where_to: f64) -> bool {
        let Some(name) = self.focused_window_buffer() else {
            return false;
        };
        let Some((line_count, point_line)) = self.with_buffer(&name, |buf| {
            (buf.text.line_count(), buf.text.cursor_pos().0)
        }) else {
            return false;
        };
        self.windows_mut(|windows| windows.recenter_focused(where_to, line_count, point_line))
    }

    /// Move the focused window's view by AMOUNT screenfuls, forwards when
    /// AMOUNT is positive, and drag point along if it would otherwise be left
    /// outside. See [`Windows::scroll_focused`].
    ///
    /// Returns false when the view could not move at all -- already showing
    /// the end and asked to go forward, or the beginning and asked to go back
    /// -- so the caller can say so rather than leaving the key looking broken.
    pub(crate) fn scroll_focused_window(&self, amount: isize) -> bool {
        let Some(name) = self.focused_window_buffer() else {
            return false;
        };
        let Some((line_count, point_line, point_column)) = self.with_buffer(&name, |buf| {
            let (line, column) = buf.text.cursor_pos();
            (buf.text.line_count(), line, column)
        }) else {
            return false;
        };
        // The window lock is let go before point is touched. Moving point is
        // a change to a *buffer*, which is why the compartment names a line
        // rather than making the move itself: it has no business holding a
        // buffer, and this way it never does.
        let Scrolled::Yes { drag_point_to } =
            self.windows_mut(|windows| windows.scroll_focused(amount, line_count, point_line))
        else {
            return false;
        };
        if let Some(line) = drag_point_to {
            self.with_buffer_mut(&name, |buf| buf.text.cursor_move(line, point_column));
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
    pub(crate) fn delete_window_by_id(&self, id: WindowId) -> bool {
        // `Refocus` means the compartment has already moved focus. What is
        // left is the editor's half of a focus change -- making that window's
        // buffer current and putting point back where it was -- and it is done
        // out here, after the lock is given back, because both touch buffers.
        match self.windows_mut(|windows| windows.remove(id)) {
            WindowRemoved::No => false,
            WindowRemoved::Yes => true,
            WindowRemoved::Refocus(survivor) => {
                self.follow_focus(survivor);
                true
            }
        }
    }

    /// Close the focused window. False when it is the only one.
    ///
    /// The same operation as `delete_window_by_id` and now written as one: the
    /// two used to differ in whether focus moved afterwards, which was never a
    /// difference between them but a difference between *which* window went.
    /// [`Windows::remove`] answers that, so there is one rule rather than two
    /// copies of it that could drift.
    pub(crate) fn delete_focused_window(&self) -> bool {
        self.delete_window_by_id(self.get_focused_window_id())
    }

    /// Close every window but the focused one.
    pub(crate) fn delete_other_windows(&self) -> bool {
        self.windows_mut(|windows| windows.delete_others())
    }

    /// Move focus COUNT windows on, wrapping round.
    ///
    /// A negative count goes the other way, which is what lets one command
    /// serve `C-x o` and a reversed `C-x o` alike.
    pub(crate) fn focus_other_window(&self, count: isize) -> bool {
        // As in `delete_window_by_id`: the compartment moves focus, and the
        // buffer half of the move happens after the lock is back.
        let Some(landed) = self.windows_mut(|windows| windows.focus_other(count)) else {
            return false;
        };
        self.follow_focus(landed);
        true
    }

    /// How many tiled windows the frame holds.
    pub(crate) fn window_count(&self) -> usize {
        self.windows(|windows| windows.count())
    }

    /// The buffer shown by the focused window, which is not always the current
    /// buffer: a floating prompt takes the current buffer without taking a
    /// tiled window's place.
    pub(crate) fn focused_window_buffer(&self) -> Option<String> {
        self.windows(|windows| windows.focused_buffer())
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

        // The id and the push are one acquisition rather than two, which is
        // what a single lock over the five old fields buys: no other thread
        // can see an id handed out and not yet used.
        let new_id = self.windows_mut(|windows| {
            let new_id = windows.next_id();
            windows.floating_mut().push(FloatingWindow {
                window: Window::new(new_id, buf_name),
                rect: Rect {
                    x,
                    y,
                    width,
                    height,
                },
                has_border: true,
                title,
                previous_focused_window_id,
            });
            new_id
        });

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
        self.commands_mut(|commands| commands.read_prefix_argument(event))
    }

    // ---------------------------------------------------------------
    // The command compartment
    // ---------------------------------------------------------------

    /// Ask the commands something. Same rules as [`EditorState::windows`].
    pub(crate) fn commands<R>(&self, f: impl FnOnce(&Commands<B>) -> R) -> R {
        f(&self.commands.read().expect("read lock on commands"))
    }

    /// Change them -- register one, take a prefix argument, push an argument
    /// onto the pending stack.
    pub(crate) fn commands_mut<R>(&self, f: impl FnOnce(&mut Commands<B>) -> R) -> R {
        f(&mut self.commands.write().expect("write lock on commands"))
    }

    // ---------------------------------------------------------------
    // The mode compartment
    // ---------------------------------------------------------------

    /// Ask the modes something. Same rules as [`EditorState::windows`].
    ///
    /// The rule matters more here than anywhere else: the lists this holds are
    /// Lisp, and calling one re-enters the editor through any primitive it
    /// likes. Copy out, let go, *then* call.
    pub(crate) fn modes<R>(&self, f: impl FnOnce(&Modes<B>) -> R) -> R {
        f(&self.modes.read().expect("read lock on modes"))
    }

    /// Change the modes -- define one, bind a key, add a completion source.
    pub(crate) fn modes_mut<R>(&self, f: impl FnOnce(&mut Modes<B>) -> R) -> R {
        f(&mut self.modes.write().expect("write lock on modes"))
    }

    /// Install a keymap that is consulted before every other until it goes
    /// away. See [`TransientKeymap`].
    pub(crate) fn set_transient_keymap(&self, map: TransientKeymap<B>) {
        self.modes_mut(|modes| modes.set_transient(map));
    }

    /// Take the map down. Idempotent, so a command that ends one can call it
    /// without first asking whether one is up.
    pub(crate) fn clear_transient_keymap(&self) {
        self.modes_mut(|modes| modes.clear_transient());
    }

    /// What the installed map wants shown, or empty when none is installed.
    pub(crate) fn transient_message(&self) -> String {
        self.modes(|modes| modes.transient_message())
    }

    /// Whether a transient keymap is installed. Only for reporting.
    pub(crate) fn transient_keymap_active(&self) -> bool {
        self.modes(|modes| modes.transient_active())
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
        self.modes(|modes| modes.completion_sources(mode))
    }

    /// Append a source to MODE's list, or to the global one when MODE is
    /// `None`. False if MODE names a mode that does not exist.
    pub(crate) fn add_completion_function(
        &self,
        mode: Option<&str>,
        function: ELispExp<B>,
    ) -> bool {
        self.modes_mut(|modes| modes.add_completion(mode, function))
    }

    /// Replace a whole list. This is how a source is removed or the order
    /// changed -- `(set-completion-functions nil (list ...))` -- which a
    /// bare `add` could not express.
    pub(crate) fn set_completion_functions(
        &self,
        mode: Option<&str>,
        functions: Vec<ELispExp<B>>,
    ) -> bool {
        self.modes_mut(|modes| modes.set_completions(mode, functions))
    }

    /// One list on its own, unmerged, or `None` if MODE is unknown.
    pub(crate) fn completion_function_list(&self, mode: Option<&str>) -> Option<Vec<ELispExp<B>>> {
        self.modes(|modes| modes.completion_list(mode))
    }

    /// Say that a file whose name matches PATTERN opens in MODE.
    ///
    /// Without this a language module can be loaded and never selected: nothing
    /// else maps a file to a mode, so every grammar would have to be reached by
    /// hand.
    pub(crate) fn add_auto_mode(&self, pattern: regex::Regex, mode: &str) {
        self.modes_mut(|modes| modes.add_auto_mode(pattern, mode));
    }

    /// The mode a file called PATH should open in, if any pattern claims it.
    ///
    /// Matched against the whole path, so a pattern can key on a directory as
    /// well as an extension.
    pub(crate) fn auto_mode_for(&self, path: &str) -> Option<String> {
        self.modes(|modes| modes.auto_mode_for(path))
    }

    /// Say that COMMAND may be repeated by pressing KEYS on its own afterwards.
    ///
    /// Declared rather than inferred. The tempting rule -- "after a sequence
    /// ending in K, a bare K repeats" -- would make `C-x C-f` followed by `f`
    /// re-open `find-file`, which is not a convenience.
    pub(crate) fn set_repeat_key(&self, command: &str, key: KeyEvent) {
        self.modes_mut(|modes| modes.set_repeat_key(command, key));
    }

    /// The key that repeats COMMAND, if it has one.
    fn repeat_key(&self, command: &str) -> Option<KeyEvent> {
        self.modes(|modes| modes.repeat_key(command))
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
        self.runtime_mut(|runtime| runtime.begin_isearch(session));
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
        self.runtime_mut(|runtime| runtime.take_isearch())
    }

    /// Whether an incremental search is running. Only for reporting -- anything
    /// that acts on the session takes it.
    pub(crate) fn isearch_active(&self) -> bool {
        self.runtime(|runtime| runtime.isearch_active())
    }

    /// The argument waiting for the next command, if any.
    pub(crate) fn prefix_argument(&self) -> Option<PrefixArg> {
        self.commands(|commands| commands.prefix_argument())
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
        self.commands_mut(|commands| commands.clear_prefix_argument());
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

        // Asked before the modes are taken: buffers come before modes in the
        // canonical order.
        let current_mode = self.with_current_buffer(|buf| buf.current_mode.clone());

        // One acquisition answers the whole question -- the transient map, the
        // mode's keymap and the global one. It used to be three locks and an
        // atomic gate in front of the first of them, with a written protocol
        // keeping the gate and the map it guarded in step. See [`Modes`].
        let mut release_transient = false;
        let bound = self.modes(|modes| {
            // A transient keymap is consulted before every other, for as long
            // as it is installed. See `TransientKeymap`.
            if let Some(map) = modes.transient() {
                let hit = map.keymap.get(&pending).cloned();
                let prefix = map.keymap.is_prefix(&pending);
                match (hit, prefix, map.on_unbound) {
                    (Some(ast), _, _) => return Bound::Command(ast),
                    // Part-way through one of the map's own sequences.
                    (None, true, _) => return Bound::Prefix,
                    // Refused, and nothing said about it: the map's message is
                    // still in the frame, and anything written to the echo
                    // area would be drawn under it.
                    (None, false, OnUnbound::Refuse) => return Bound::Refused,
                    // Handed on, and the map stays. The key carries on to the
                    // keymaps below and the map is consulted again next time,
                    // which is what lets the completion strip be typed at
                    // without either swallowing the letter or dismissing
                    // itself.
                    (None, false, OnUnbound::Pass) => {}
                    // Handed on, and the map goes. The key carries on exactly
                    // as though it had never been there -- which is what makes
                    // the offer free to ignore. Taken down after this lock is
                    // given back, since dropping it needs the write side.
                    (None, false, OnUnbound::Release) => release_transient = true,
                }
            }

            // The mode's own keymap wins, then the global one -- and a mode
            // that binds a prefix keeps the sequence alive even when only the
            // global map completes it.
            let mode_keymap = modes.mode_keymap(&current_mode);
            let global = modes.global_keymap();
            let hit = mode_keymap
                .and_then(|keymap| keymap.get(&pending))
                .or_else(|| global.get(&pending))
                .cloned();
            let prefix = mode_keymap.is_some_and(|keymap| keymap.is_prefix(&pending))
                || global.is_prefix(&pending);
            match (hit, prefix) {
                (Some(ast), _) => Bound::Command(ast),
                (None, true) => Bound::Prefix,
                (None, false) => Bound::Unbound,
            }
        });
        if release_transient {
            // Holding the bindings of a map nobody can reach would be a small
            // leak that lasted until the next one was installed.
            self.clear_transient_keymap();
        }

        // Everything the keymaps had to say, said, and the lock given back --
        // so reporting, which writes the echo area, happens under none of it.
        let described = describe_keys(&pending);
        if !matches!(bound, Bound::Prefix) {
            pending.clear();
        }
        drop(pending);

        match bound {
            Bound::Command(ast) => Some(ast),
            // Nothing to say: the sequence is in `pending_input`, which the
            // frame carries and which does not expire the way a message does.
            Bound::Prefix | Bound::Refused => None,
            Bound::Unbound => {
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
        let current_mode_name = self.with_current_buffer(|buf| buf.current_mode.clone());
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
            self.commands_mut(|commands| commands.set_last_form(Some(ast)));
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

    /// Scroll WINDOW by LINES, towards the end of the buffer when positive.
    ///
    /// Neither focus nor point moves. False when there is no such window or
    /// the view was already as far as it goes.
    ///
    /// The two locks are taken one at a time and given straight back rather
    /// than nested: the line count has to come from the buffer and the scroll
    /// has to be written to the window, and holding both would put an edge in
    /// the ordering for the sake of an operation that does not need one.
    pub(crate) fn scroll_window_by(&self, window: WindowId, lines: isize) -> bool {
        let Some(name) = self.windows(|windows| windows.buffer_of(window)) else {
            return false;
        };
        let Some(line_count) = self.with_buffer(&name, |buf| buf.text.line_count()) else {
            return false;
        };
        self.windows_mut(|windows| windows.scroll_by(window, lines, line_count))
    }

    /// What the pointer at (X, Y) is over, in cells of the last frame.
    ///
    /// `None` when it is over nothing -- the echo area, or a gap no window
    /// claims.
    pub(crate) fn hit_test(&self, x: isize, y: isize) -> Option<Hit> {
        self.windows(|windows| windows.hit_test(x, y))
    }

    pub(crate) fn take_mouse_drag(&self) -> Option<MouseDrag> {
        self.windows_mut(|windows| windows.take_drag())
    }

    /// Move the boundary of the split at PATH by DELTA cells.
    pub(crate) fn resize_dragged_split(&self, path: &[Side], delta: isize) -> bool {
        self.windows_mut(|windows| windows.resize_split(path, delta))
    }

    fn set_mouse_drag(&self, drag: Option<MouseDrag>) {
        self.windows_mut(|windows| windows.set_drag(drag));
    }

    pub(crate) fn mouse_drag(&self) -> Option<MouseDrag> {
        self.windows(|windows| windows.drag())
    }

    /// Where in WINDOW's buffer the cell (X, Y) is, clamped to what the window
    /// is showing.
    ///
    /// Clamped rather than refused because this answers a *drag*, and a drag
    /// that leaves the window is a perfectly ordinary way to select to its
    /// edge. Dragging beyond an edge selects to that edge and stops; it does
    /// not scroll the window after the pointer, which is a separate feature
    /// and a worse one to get subtly wrong.
    fn position_in(&self, window: WindowId, x: isize, y: isize) -> Option<(usize, usize)> {
        self.windows(|windows| windows.position_in(window, x, y))
    }

    /// Which way a drag that has left its window wants the view to move, if it
    /// has left it at all.
    ///
    /// Vertically only. Dragging off the side of a window is a request to
    /// select to the end of the lines you are over, which clamping already
    /// gives; dragging off the top or bottom is a request for lines that are
    /// not on screen, which nothing but scrolling can answer.
    fn drag_scroll_step(&self, window: WindowId, y: isize) -> Option<isize> {
        self.windows(|windows| windows.drag_scroll_step(window, y))
    }

    /// How long until a drag that has left its window should scroll again, or
    /// `None` when none is waiting to.
    ///
    /// # Why this is a timer and not an event
    ///
    /// A pointer held still outside a window sends nothing at all, and that is
    /// exactly the position somebody selecting a long passage leaves it in.
    /// Scrolling only on movement would mean the selection stopped the moment
    /// they stopped jiggling the mouse, which reads as the editor having lost
    /// interest.
    pub fn drag_scroll_in(&self) -> Option<Duration> {
        self.windows(|windows| windows.drag_scroll_in())
    }

    /// Scroll a drag that has left its window by one line, and take the
    /// selection with it. False when there was nothing to do.
    ///
    /// One line a turn rather than a distance that grows with how far outside
    /// the pointer is: a selection running away faster the further the hand
    /// strays is hard to stop where you meant to, and the turn is short enough
    /// that holding the pointer out is smooth anyway.
    pub fn drag_scroll_tick(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        let Some(MouseDrag::Text { window, at: (x, y) }) = self.mouse_drag() else {
            return false;
        };
        let Some(step) = self.drag_scroll_step(window, y) else {
            return false;
        };
        if !self.scroll_window_by(window, step) {
            // Already as far as the buffer goes. The selection is already at
            // that end, so there is nothing left to extend either.
            return false;
        }
        let Some((line, column)) = self.position_in(window, x, y) else {
            return false;
        };
        self.run_command_form(
            mouse_form(
                "mouse-drag-to",
                &[window.0 as f64, line as f64, column as f64],
            ),
            env,
        );
        true
    }

    /// One more event of a drag already in progress.
    fn continue_drag(&self, x: isize, y: isize, env: &Arc<Env<EditorState<B>>>) -> bool {
        let form = match self.mouse_drag() {
            Some(MouseDrag::Text { window, .. }) => {
                self.set_mouse_drag(Some(MouseDrag::Text { window, at: (x, y) }));
                // Against the window the drag began in, not whatever is under
                // the pointer now: a selection that changed buffers halfway
                // through is nobody's idea of a selection.
                self.position_in(window, x, y).map(|(line, column)| {
                    mouse_form(
                        "mouse-drag-to",
                        &[window.0 as f64, line as f64, column as f64],
                    )
                })
            }
            Some(MouseDrag::Divider {
                path,
                orientation,
                last,
            }) => {
                // How far the pointer has come since the last event, along the
                // axis the boundary moves in. Kept as a running position
                // rather than compared with where the drag started, so a
                // boundary that could not move as far as the pointer did does
                // not then lag behind it for the rest of the drag.
                let now = match orientation {
                    Orientation::Horizontal => y,
                    Orientation::Vertical => x,
                };
                self.set_mouse_drag(Some(MouseDrag::Divider {
                    path,
                    orientation,
                    last: now,
                }));
                (now != last).then(|| mouse_form("mouse-resize", &[(now - last) as f64]))
            }
            None => None,
        };
        let Some(form) = form else {
            return false;
        };
        self.run_command_form(form, env);
        true
    }

    /// Act on a mouse event from the frontend.
    ///
    /// # Why this resolves and then dispatches
    ///
    /// Working out *where* a click landed needs the layout and every window's
    /// scroll, which only the editor has -- so it happens here. Deciding what
    /// a click *means* is a different kind of question, and it goes through
    /// the command machinery exactly as `handle_paste` does: one undo step,
    /// one `post-command-hook`, a name that `M-x` and the logs can see, and a
    /// binding that can be replaced later without touching this function.
    ///
    /// Answers whether the event was acted on. A terminal that is reporting
    /// the mouse sends motion and drag events by the dozen per second, and
    /// nearly all of them mean nothing here -- so the frontend needs to know
    /// which ones are worth a redraw, or it repaints the screen continuously
    /// for frames identical to the last.
    pub fn handle_mouse_event(&self, event: MouseEvent, env: &Arc<Env<EditorState<B>>>) -> bool {
        // Checked here rather than only where the terminal switches capture
        // on, so the setting means the same thing to every frontend: a GUI
        // that always delivers mouse events has to obey it too.
        if !mouse_mode(env) {
            return false;
        }
        let (x, y) = (event.column as isize, event.row as isize);

        // Continuing or ending a drag is answered before asking what is under
        // the pointer, because a drag is about where it *began*. By the time
        // one is a few rows long the pointer is often over another window or
        // off the frame entirely, where the hit test answers nothing -- and a
        // selection that stopped growing the moment you overshot the window
        // would be a selection you could not make.
        match event.kind {
            MouseKind::Drag(MouseButton::Left) => return self.continue_drag(x, y, env),
            MouseKind::Up(MouseButton::Left) => {
                self.take_mouse_drag();
                return false;
            }
            _ => (),
        }

        let Some(hit) = self.hit_test(x, y) else {
            return false;
        };
        let form = match (event.kind, hit) {
            (
                MouseKind::Down(MouseButton::Left),
                Hit::Text {
                    window,
                    line,
                    column,
                },
            ) => {
                self.set_mouse_drag(Some(MouseDrag::Text { window, at: (x, y) }));
                Some(mouse_form(
                    "mouse-set-point",
                    &[window.0 as f64, line as f64, column as f64],
                ))
            }
            (MouseKind::Down(MouseButton::Left), Hit::Separator { path, orientation }) => {
                self.set_mouse_drag(Some(MouseDrag::Divider {
                    path,
                    orientation,
                    last: x,
                }));
                None
            }
            (MouseKind::Down(MouseButton::Left), Hit::ModeLine { window }) => {
                // A status line divides a stacked pair, and which pair is a
                // question about the tree -- asked here, once, rather than on
                // every event of the drag that follows.
                let divider = self.windows(|windows| windows.root().mode_line_divider(window));
                if let Some(path) = divider {
                    self.set_mouse_drag(Some(MouseDrag::Divider {
                        path,
                        orientation: Orientation::Horizontal,
                        last: y,
                    }));
                }
                None
            }
            (MouseKind::ScrollUp, Hit::Text { window, .. }) => {
                Some(mouse_form("mouse-scroll", &[window.0 as f64, -1.0]))
            }
            (MouseKind::ScrollDown, Hit::Text { window, .. }) => {
                Some(mouse_form("mouse-scroll", &[window.0 as f64, 1.0]))
            }
            // Everything else is swallowed on purpose -- every click on a
            // float, and every button nothing is bound to yet.
            _ => None,
        };
        let Some(form) = form else {
            return false;
        };
        self.run_command_form(form, env);
        true
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
        // One acquisition for both halves. The mode's own hooks and the ones
        // registered for every mode used to be two locks read in sequence; they
        // are one compartment now, so a hook added to one half between the two
        // reads can no longer produce a list that was never true at any instant.
        let hooks = self.modes(|modes| modes.hooks_for(mode_name, hook_name));

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
        let Some(closing_mode) = self.with_buffer(name, |buf| buf.current_mode.clone()) else {
            return false;
        };

        // Detach NAME from wherever it's currently displayed.
        //
        // Finding the float and removing it are now one acquisition rather
        // than a read followed by a write. They were never safe apart: between
        // the two, another thread opening or closing a float shifts the index,
        // and what came back was a *different* window than the one looked for.
        let restore_id = self.windows_mut(|windows| {
            let idx = windows
                .floating()
                .iter()
                .position(|f| f.window.buffer_name == name)?;
            Some(
                windows
                    .floating_mut()
                    .remove(idx)
                    .previous_focused_window_id,
            )
        });
        // Outside the closure, because moving focus makes a buffer current and
        // takes the window lock again to find out which.
        if let Some(restore_id) = restore_id {
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
        // Worked out before the window lock is taken: `most_recent_buffer`
        // reads the buffer compartment, and taking that while holding the
        // windows would invert the canonical lock order.
        let replacement = self
            .most_recent_buffer(name)
            .unwrap_or_else(|| "*scratch*".to_string());
        self.windows_mut(|windows| {
            windows.root_mut().each_window_mut(&mut |window| {
                if window.buffer_name == name {
                    window.show(&replacement);
                }
            })
        });

        // Removing it and settling what is current are one acquisition. They
        // used to be four -- remove, re-add `*scratch*` if that emptied the
        // table, read the current name, then set it -- and between any two of
        // them the current name could be read by another thread while it
        // named the buffer that had just gone. That read is the panic
        // described above.
        if let BufferRemoved::Current(successor) = self.buffers_mut(|buffers| buffers.remove(name))
        {
            // The compartment picked the most recent survivor. Prefer the one
            // the windows were just repointed at, when it is still there, so
            // that what is current and what is on screen agree -- them
            // disagreeing is how the panic above was reached in the first
            // place.
            if replacement != successor {
                self.set_current_buffer_name(&replacement);
            }
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
    /// `echo_message` -> `windows` -> `buffers` -> an individual `Buffer`
    ///
    /// It used to be five links long. Three of them -- the focused id, the
    /// layout and the floating windows -- are one lock now, so the order they
    /// had to be taken in is not a rule anybody can get wrong any more.
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
    /// # Note on `windows`
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

        // One acquisition for the tiled tree, the floats and the focused id.
        // The three used to be read separately, which is how a frame could
        // report a cursor in a window the layout had already removed.
        let (views, separators, focused_window_id) = self.windows_mut(|windows| {
            let buffers = self
                .buffers
                .read()
                .expect("Failed to acquire read lock on buffers");

            let focused_window_id = windows.focused();
            let mut views = Vec::new();
            let mut separator_rects = Vec::new();
            let focus = Focus {
                id: focused_window_id,
                tiled: windows.root().contains_window(focused_window_id),
            };
            windows.root_mut().compute_tiled_views(
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
                focus,
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

            for float in windows.floating().iter() {
                let is_focused = float.window.id == focused_window_id;
                // Unlike a tiled window, a float is not auto-scrolled to follow the
                // cursor; its scroll offsets are whatever whoever opened it set.
                let cursor_rel_pos = is_focused
                    .then(|| buffers.handle(&float.window.buffer_name))
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
                    rect: float.rect,
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

            (views, separators, focused_window_id)
        });

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
        self.modes_mut(|modes| modes.insert(mode_name, mode));
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

    /// Send a task to the worker thread, and say whether it was accepted.
    ///
    /// The mailbox used to be a public field, so anything could post work to
    /// the background scheduler. It is a method now for the same reason the
    /// rest of the state is: there is exactly one queue, and the count of what
    /// is in flight has to be kept in step with it -- see `begin_shell_command`.
    pub(crate) fn send_to_worker(&self, message: WorkerMessage<B>) -> bool {
        self.worker_mailbox.send(message).is_ok()
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
        self.log.read().expect("read lock on log").lines()
    }

    /// Return the call stack captured at the point of the most recent
    /// uncaught error, innermost (deepest) call first -- or an empty list
    /// if nothing has errored since the last `clear_backtrace`. See
    /// `LispContext::push_call_frame` for the capture protocol and its
    /// tail-call caveat.
    pub fn backtrace(&self) -> Vec<String> {
        self.runtime(|runtime| runtime.backtrace())
    }

    /// Discard the captured backtrace, so the next error starts from a
    /// clean stack instead of stacking on top of a stale one. Callers that
    /// catch and report an error (a key handler, `eval_file`, ...) should
    /// call this once they're done reading `backtrace()`.
    pub fn clear_backtrace(&self) {
        self.runtime_mut(|runtime| runtime.clear_backtrace());
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
    pub(crate) fn begin_command(&self) -> FuelScope {
        // The meter is cloned out and the lock given straight back: a metered
        // scope lasts a whole command, and holding this lock for that long
        // would be holding it across the interpreter.
        self.runtime(|runtime| runtime.fuel()).begin()
    }

    /// Ask the runtime something. Same rules as [`EditorState::windows`].
    pub(crate) fn runtime<R>(&self, f: impl FnOnce(&Runtime) -> R) -> R {
        f(&self.runtime.read().expect("read lock on runtime"))
    }

    /// Change it.
    pub(crate) fn runtime_mut<R>(&self, f: impl FnOnce(&mut Runtime) -> R) -> R {
        f(&mut self.runtime.write().expect("write lock on runtime"))
    }

    /// The execution meter behind [`Self::begin_command`].
    ///
    /// Exposed for `lisp::measure`, which needs the meter to hold a scope of
    /// its own for the duration of a measurement.
    pub(crate) fn fuel_meter(&self) -> Arc<FuelMeter> {
        self.runtime(|runtime| runtime.fuel())
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
        self.commands_mut(|commands| commands.push_pending(name, remaining, invocation));
    }

    /// Everything the editor can answer on the user's behalf, as it stands now.
    ///
    /// Taken once, when a command starts. See [`Invocation`] for why it is not
    /// read again later.
    pub(crate) fn capture_invocation(&self) -> Invocation {
        // The prefix argument is read *before* the buffer is locked: two
        // compartments, never held together.
        let prefix_arg = self.prefix_argument();
        let region = self.with_current_buffer(|buf| {
            crate::buffer::mark::region_bounds(buf.mark, buf.text.cursor_pos_1d(), buf.text.len())
        });
        Invocation { prefix_arg, region }
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
        self.commands_mut(|commands| commands.fill_answerable_args())
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
        self.commands(|commands| commands.pending_progress())
    }

    /// The argument the innermost pending command is waiting on.
    pub(crate) fn pending_current_spec(&self) -> Option<ArgSpec> {
        self.commands(|commands| commands.pending_current_spec())
    }

    /// Record VALUE as the innermost pending command's next argument.
    ///
    /// What to do next is [`Self::fill_answerable_args`]'s answer, not this
    /// one's: the specs after this may be a mix of answerable and prompted,
    /// and only one place should know how to walk them.
    pub(crate) fn accept_pending_arg(&self, value: ELispExp<B>) {
        self.commands_mut(|commands| commands.accept_pending_arg(value));
    }

    /// Remove and return the innermost pending command.
    pub(crate) fn take_pending_command(&self) -> Option<(String, Vec<ELispExp<B>>)> {
        self.commands_mut(|commands| commands.take_pending())
    }

    /// Drop every pending command.
    ///
    /// Called when a fresh command starts with no minibuffer open, which means
    /// any entry still on the stack belongs to a prompt that was closed by some
    /// path other than confirm or cancel. Without this, that orphan would be
    /// fed the *next* command's input.
    pub(crate) fn clear_pending_commands(&self) {
        self.commands_mut(|commands| commands.clear_pending());
    }

    /// Whether a minibuffer prompt is currently open.
    pub(crate) fn minibuffer_is_open(&self) -> bool {
        self.has_buffer("*Minibuffer*")
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
        self.commands(|commands| commands.last_was(name))
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
        self.runtime_mut(|runtime| runtime.reset_theme());
    }

    pub(crate) fn set_face_style(&self, face: Face, style: Style) {
        // Copy-on-write: whatever frames are already holding keep the theme
        // they were composed under, and the next one picks this up.
        self.runtime_mut(|runtime| runtime.set_face_style(face, style));
    }

    pub(crate) fn face_style(&self, face: Face) -> Style {
        self.runtime(|runtime| runtime.face_style(face))
    }

    pub(crate) fn theme(&self) -> Arc<Theme> {
        self.runtime(|runtime| runtime.theme())
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
        // Asked before the lock is taken. It is a Lisp variable, and no
        // compartment may be holding a lock when the interpreter is touched.
        let to_clipboard = Self::clipboard_sync_enabled(env);
        self.kill_yank_mut(|kills| kills.kill(text, direction, to_clipboard));
    }

    /// Ask the kill ring something. Same rules as [`EditorState::windows`].
    pub(crate) fn kill_yank<R>(&self, f: impl FnOnce(&KillYank) -> R) -> R {
        f(&self.kill_yank.read().expect("read lock on kill_yank"))
    }

    /// Change it -- kill, yank, rotate, or roll the flags over.
    pub(crate) fn kill_yank_mut<R>(&self, f: impl FnOnce(&mut KillYank) -> R) -> R {
        f(&mut self.kill_yank.write().expect("write lock on kill_yank"))
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
        self.kill_yank_mut(|kills| kills.take_pending_clipboard())
    }

    /// What `yank` would insert, if anything.
    pub(crate) fn current_kill(&self) -> Option<String> {
        self.kill_yank(|kills| kills.current())
    }

    /// The entry N kills back, without moving the ring.
    pub(crate) fn nth_kill(&self, n: usize) -> Option<String> {
        self.kill_yank(|kills| kills.nth(n))
    }

    /// Step the ring back one entry and return what is now current.
    pub(crate) fn rotate_kill_ring(&self) -> Option<String> {
        self.kill_yank_mut(|kills| kills.rotate())
    }

    pub(crate) fn set_kill_ring_max(&self, max: usize) {
        self.kill_yank_mut(|kills| kills.set_max(max));
    }

    pub(crate) fn kill_ring_len(&self) -> usize {
        self.kill_yank(|kills| kills.len())
    }

    /// Remember that a yank put LEN characters at AT, so `yank-pop` knows what
    /// to take back out.
    pub(crate) fn note_yank(&self, at: usize, len: usize) {
        self.kill_yank_mut(|kills| kills.note_yank(at, len));
    }

    /// What the previous command yanked, if the previous command was a yank.
    ///
    /// `yank-pop` replaces the text a yank just inserted, so it is only
    /// meaningful directly after one; anything else in between and there is
    /// nothing it would be safe to remove.
    pub(crate) fn yank_to_replace(&self) -> Option<(usize, usize)> {
        self.kill_yank(|kills| kills.yank_to_replace())
    }

    /// Roll "this command" into "the previous command" for the flags that a
    /// command needs to ask about its predecessor.
    fn roll_over_command_flags(&self) {
        self.kill_yank_mut(|kills| kills.roll_over());
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
        self.modes_mut(|modes| modes.add_global_hook(hook_name, function));
    }

    /// The last command as a runnable form, for `repeat`.
    pub(crate) fn last_command_form(&self) -> Option<ELispExp<B>> {
        self.commands(|commands| commands.last_form())
    }

    pub(crate) fn set_last_command(&self, name: Option<Arc<String>>) {
        self.commands_mut(|commands| commands.set_last_name(name));
    }

    pub(crate) fn goal_column(&self) -> Option<usize> {
        self.runtime(|runtime| runtime.goal_column())
    }

    pub(crate) fn set_goal_column(&self, col: Option<usize>) {
        self.runtime_mut(|runtime| runtime.set_goal_column(col));
    }

    /// Every live buffer's name, sorted. Used for buffer-name completion.
    pub(crate) fn buffer_names(&self) -> Vec<String> {
        self.buffers(|buffers| buffers.names())
    }

    /// Every live buffer's name with the focused window's at the head.
    ///
    /// For the background walkers, which both want to reach what is on screen
    /// before what is not. The focused window is asked *before* the table is
    /// taken: two compartments, never held together.
    pub(crate) fn buffer_names_focused_first(&self) -> Vec<String> {
        let focused = self.focused_window_buffer();
        self.buffers(|buffers| buffers.names_with_first(focused.as_deref()))
    }

    /// Register NAME as a command taking SPECS.
    ///
    /// Idempotent by name: re-registering replaces the previous specs, so a
    /// user can change how an existing command prompts without restarting.
    pub(crate) fn register_command(&self, name: &str, specs: Vec<ArgSpec>) {
        self.commands_mut(|commands| commands.register(name, specs));
    }

    /// The arguments to collect for NAME, or `None` if it is not a command.
    ///
    /// Returns owned data, and every other accessor here does too. That is not
    /// incidental: `call-interactively` looks a command up and then evaluates
    /// Lisp, and Lisp can call `register-command`. Handing back a guard would
    /// mean holding this lock across `eval` -- exactly the reentrancy that
    /// deadlocked `run_hook`.
    pub(crate) fn command_specs(&self, name: &str) -> Option<Vec<ArgSpec>> {
        self.commands(|commands| commands.specs(name))
    }

    pub(crate) fn is_command(&self, name: &str) -> bool {
        self.commands(|commands| commands.is_command(name))
    }

    /// Every command name, sorted, for M-x completion.
    pub(crate) fn command_names(&self) -> Vec<String> {
        self.commands(|commands| commands.names())
    }

    /// Set how much fuel a fresh command receives, and top the current thread's
    /// remaining fuel up to it. Exposed so the `set-command-fuel` primitive --
    /// and tests that want a deliberately tiny budget -- can reach it.
    pub(crate) fn set_fuel_budget(&self, budget: u32) {
        self.runtime(|runtime| runtime.fuel()).set_budget(budget);
    }

    /// Get the ID of the current focused window
    pub fn get_focused_window_id(&self) -> WindowId {
        self.windows(|windows| windows.focused())
    }

    /// Move focus to window ID, and make what it shows the current buffer.
    ///
    /// False when there is not, which is the useful answer rather than a
    /// failure: a window remembered earlier may have been closed since, and
    /// the caller wants to open a new one rather than be stopped.
    pub(crate) fn select_window(&self, id: WindowId) -> bool {
        // Tiled or floating: a prompt is a float, and selecting one has to
        // work exactly as selecting a tiled window does.
        let exists = self.windows(|windows| windows.buffer_of(id).is_some());
        if exists {
            self.set_focused_window_id(id);
        }
        exists
    }

    pub(crate) fn set_focused_window_id(&self, id: WindowId) {
        self.windows_mut(|windows| windows.set_focused(id));
        self.follow_focus(id);
    }

    /// The buffer half of a focus change: make what the newly focused window
    /// shows the current buffer, and put point back where that window left it.
    ///
    /// Separate from [`EditorState::set_focused_window_id`] because focus does
    /// not always move from out here. `Windows::focus_other` and
    /// `Windows::remove` move it themselves -- they have the lock open and the
    /// list of survivors in hand -- and what they cannot do is touch a buffer.
    /// This is the other half, and every path that moves focus ends in it.
    ///
    /// Called with no lock held, deliberately. It takes the windows to ask
    /// what the window shows, and then the buffers to move point.
    fn follow_focus(&self, id: WindowId) {
        if let Some(name) = self.window_buffer(id) {
            self.set_current_buffer_name(&name);
            self.restore_window_point(id, &name);
        }
    }

    /// Put point back where the window taking focus last had it.
    ///
    /// Only tiled windows: a floating one is not in the layout, so it answers
    /// `None` and nothing happens -- which is right, since a prompt's point
    /// belongs to the prompt.
    fn restore_window_point(&self, id: WindowId, buffer_name: &str) {
        let remembered = self.windows(|windows| windows.root().window(id).and_then(|w| w.point));
        let Some(offset) = remembered else {
            return;
        };
        // The window lock is let go above rather than held across the write
        // below. Windows come before buffers in the ordering, so holding both
        // would be legal -- but every other path here copies out and lets go,
        // and the one that does not is the one that eventually deadlocks.
        self.with_buffer_mut(buffer_name, |buf| goto_offset(&mut buf.text, offset));
    }

    /// What window ID is showing, whether it is tiled or floating.
    ///
    /// Both, because a minibuffer prompt is a floating window and giving it
    /// focus has to make its buffer current in exactly the same way -- that is
    /// what every prompt in the editor depends on.
    fn window_buffer(&self, id: WindowId) -> Option<String> {
        self.windows(|windows| windows.buffer_of(id))
    }

    /// Get the name of the current buffer
    pub(crate) fn get_current_buffer_name(&self) -> String {
        self.current_buffer_name_shared().to_string()
    }

    /// The current buffer's name without copying it.
    pub(crate) fn current_buffer_name_shared(&self) -> Arc<str> {
        self.buffers(|buffers| buffers.current_name())
    }

    /// Set the name of the current buffer.
    ///
    /// Silently does nothing when there is no such buffer, which is the
    /// refusal that keeps the invariant: it is no longer possible from
    /// anywhere in the editor to leave the current name pointing at a buffer
    /// the table does not hold.
    pub(crate) fn set_current_buffer_name(&self, name: &str) {
        self.buffers_mut(|buffers| buffers.make_current(name));
    }

    /// The most recently current buffer that still exists and is not EXCEPT.
    ///
    /// `None` when there is no such buffer. The caller answers that for
    /// itself -- there is always `*scratch*`, but falling back to it is a
    /// policy this does not get to make.
    pub(crate) fn most_recent_buffer(&self, except: &str) -> Option<String> {
        self.buffers(|buffers| buffers.most_recent(except))
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
        self.buffers_mut(|buffers| buffers.make_current(name))
    }

    /// The syntax table for MODE, or the default one when it has none.
    pub(crate) fn syntax_table(&self, mode: &str) -> SyntaxTable {
        self.modes(|modes| modes.syntax_table(mode))
    }

    /// The syntax table in force in the current buffer.
    pub(crate) fn current_syntax_table(&self) -> SyntaxTable {
        // The buffer's lock is let go before the registry's is taken: two
        // compartments, one at a time.
        let mode = self.with_current_buffer(|buf| buf.current_mode.clone());
        self.syntax_table(&mode)
    }
}

/// Create a global EditorState environment and a Lisp environment associated to it.
/// It installs in the lisp environment all the primitive functions to use the editor.
/// It is mandatory that the lisp environment does not outlive the EditorState struct.
/// The configuration the editor writes for somebody who has none.
///
/// A named constant rather than a literal inside the function that writes it,
/// so that a test can evaluate it -- and the test evaluates it into a bare
/// environment, because that is the situation it is written for.
///
/// # What may go in here
///
/// `eval-file` lines, and settings. Nothing that needs a macro: the modules
/// this file loads are what *define* the macros, and when one of them is
/// missing -- a test binary with no `data/lisp` beside it, an installation
/// with a module removed -- `defcommand` is not a macro and a form using it is
/// evaluated as an ordinary call. That does not degrade, it raises, and this
/// file is evaluated with `?` so it takes the whole editor down with it.
/// Anything that wants to be a command belongs in a module or in Rust. It is a Rust *raw* string, which means a
/// backslash in it is a backslash: Lisp written here with the escaping habits
/// of an ordinary string literal reaches the parser with the backslashes still
/// attached. Loading survives that -- `defcommand' does not evaluate its body
/// -- and the failure waits until somebody runs the command, which is a long
/// way from here.
pub(crate) const DEFAULT_INIT_LISP: &str = r#";; rsedit init.lisp
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

;; The mouse: click to put point, wheel to scroll the window under the pointer.
;;
;; On here rather than in the editor's own defaults because it costs something.
;; While the editor is reading the mouse the terminal is not, so selecting text
;; with the mouse to paste it into another program stops working -- hold Shift
;; in most terminals to get it back for one drag. Comment this out if you would
;; rather keep the terminal's selection.
(setq mouse-mode t)


"#;

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

    // Off here and turned on by the default init.lisp below. See `mouse_mode`
    // for why it is a setting rather than simply on.
    env.set_variable(MOUSE_MODE.into(), ELispExp::nil());

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
    editor_state.set_mode(
        "fundamental-mode",
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
                if let Err(err) = fs::write(&user_config_path, DEFAULT_INIT_LISP) {
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
