//! Bringing an editor into existence: the initial state, the Lisp that
//! configures it, and the environment the two are wired into.

use super::*;

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
(eval-file "cc-mode")     ; colouring for C/C++ source
(eval-file "dired")       ; a directory in a buffer (C-x d)
;; Makes dired sort directory entries by type (folders, files).
(setq dired-sort 'type)


(eval-file "completion")  ; Tab shows every candidate at once, in a strip
(eval-file "preview")     ; C-x r t shows its replacement before you commit to it
(eval-file "clipboard")   ; kills also go to the system clipboard
(eval-file "electric-pair") ; typing "(" gives you "()"
(eval-file "buffer-list")  ; C-x b to switch, C-x C-b for the whole list
(eval-file "shell")       ; M-! runs a command and shows what it said
(eval-file "manpage")     ; C-h m, and K on a word
(eval-file "help")        ; C-h f, C-h k, C-h b -- the editor describing itself
(eval-file "occur")       ; M-s o and M-s g, and the lists M-g n walks
(eval-file "compile")     ; C-c c, and M-g n to walk what it complained about
(eval-file "theme")       ; C-c t to choose how faces are drawn

;; Where completions come from, for C-M-i in a buffer. The command is built in
;; and works without this; what this adds is the five sources it asks. Take one
;; out with `set-completion-functions', or add one of your own for a single
;; mode with `add-completion-function'.
(eval-file "completion-at-point")

;; How much work one background worker turn may do before it is stopped. See
;; `define-worker'. Raising it lets a worker do more per turn at the cost of
;; how long colouring can be kept waiting behind it.
;; (setq worker-fuel 200000)

;; A copy of unsaved work, written every half minute, for when the editor stops
;; without being asked. `M-x recover-file' brings it back. See
;; common-keymaps.lisp.
;; (setq auto-save nil)
;; (setq auto-save-directory "~/.cache/rsedit")

;; Files changing underneath you: a clean buffer is reloaded, a modified one is
;; flagged and `C-x C-s' asks. See common-keymaps.lisp.
;; (setq watch-files nil)
;; (setq watch-file-interval 10)

;; Line numbers down the left edge. Off, because a gutter costs columns and
;; not everybody wants one; `M-x toggle-line-numbers' turns it on for a look,
;; and 'relative numbers each line by its distance from point instead.
;; (setq display-line-numbers t)

;; The mouse: click to put point, wheel to scroll the window under the pointer.
;;
;; On here rather than in the editor's own defaults because it costs something.
;; While the editor is reading the mouse the terminal is not, so selecting text
;; with the mouse to paste it into another program stops working -- hold Shift
;; in most terminals to get it back for one drag. Comment this out if you would
;; rather keep the terminal's selection.
(setq mouse-mode t)


"#;

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
            background_work: Arc::new(AtomicUsize::new(0)),
            current_results: Arc::new(RwLock::new(None)),
            owed: Arc::new(RwLock::new(Vec::new())),
            kill_yank: Arc::new(RwLock::new(KillYank::default())),
            pending_keys: Arc::new(RwLock::new(Vec::new())),
            runtime: Arc::new(RwLock::new(Runtime::new(Arc::new(FuelMeter::new(
                DEFAULT_FUEL,
            ))))),
            log: Arc::new(RwLock::new(Log::default())),
            history: Arc::new(RwLock::new(History::default())),
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
}

/// The name of the variable that names the configuration directory outright.
///
/// The one override that means the same thing on every platform, which is why
/// it comes first: an absolute path to the directory holding `init.lisp`, with
/// nothing joined onto it and no convention to know. It is what a portable
/// install, a second configuration kept beside the first, and a test suite
/// that must not touch the person's own files all need.
pub const CONFIG_DIR: &str = "RSEDIT_CONFIG_DIR";

/// The freedesktop variable, honoured where it means something.
///
/// Unix only. It is a Linux and BSD convention rather than a cross-platform
/// one: Windows has `%APPDATA%`, and preferring somebody else's answer to the
/// platform's own would put a Windows user's configuration somewhere no
/// Windows program would look for it.
pub const XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";

/// Which candidate root wins, given what the environment says.
///
/// # Why this takes its inputs rather than reading them
///
/// So that the *precedence* can be tested without touching the environment.
/// The environment is process-wide and the test runner is threaded, so a test
/// that checked this by setting a variable would be a test every other test
/// could see -- which is the shape of the bug this file was last fixed for,
/// not a way to check it.
///
/// `rsedit_dir` is used as given: it names the directory itself. `xdg` is a
/// *parent* of application directories by specification, so the application's
/// own name is joined onto it. `platform` is whatever the running system
/// calls the same place, already complete.
pub(crate) fn choose_config_dir(
    rsedit_dir: Option<PathBuf>,
    xdg: Option<PathBuf>,
    platform: Option<PathBuf>,
) -> Option<PathBuf> {
    rsedit_dir
        .or_else(|| xdg.map(|xdg| xdg.join("rsedit")))
        .or(platform)
}

/// Where this installation keeps its configuration, or `None` when nothing in
/// the environment says.
///
/// Three answers, most specific first:
///
/// + `RSEDIT_CONFIG_DIR`, used as given, on every platform.
/// + `XDG_CONFIG_HOME/rsedit`, on Unix only.
/// + The platform's own place: `%APPDATA%\rsedit` on Windows,
///   `$HOME/.config/rsedit` everywhere else.
///
/// # Why macOS gets `~/.config` rather than `~/Library/Application Support`
///
/// Because that is where somebody configuring a terminal editor on a Mac will
/// look. Apple's directory is the convention for bundled applications; the
/// editors a person arrives from -- Emacs under XDG, Neovim, Helix -- all keep
/// their configuration in `~/.config`, and being the one that does not would
/// be a correctness nobody asked for.
fn user_config_dir() -> Option<PathBuf> {
    let named = std::env::var(CONFIG_DIR).ok().map(PathBuf::from);

    #[cfg(unix)]
    let xdg = std::env::var(XDG_CONFIG_HOME).ok().map(PathBuf::from);
    // Set on Windows only by somebody carrying a Unix habit across, and
    // deliberately not acted on: the platform has its own answer below.
    #[cfg(not(unix))]
    let xdg: Option<PathBuf> = None;

    #[cfg(target_os = "windows")]
    let platform = std::env::var("APPDATA")
        .ok()
        .map(|appdata| PathBuf::from(appdata).join("rsedit"));
    #[cfg(not(target_os = "windows"))]
    let platform = std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".config").join("rsedit"));

    choose_config_dir(named, xdg, platform)
}

/// The `init.lisp` this installation reads and, when it is missing, writes.
///
/// An empty path when nothing names a configuration directory, which the
/// caller reads as "there is none" rather than as somewhere to create one.
fn user_config_path() -> PathBuf {
    match user_config_dir() {
        Some(dir) => dir.join("init.lisp"),
        None => PathBuf::new(),
    }
}

/// Point this process's configuration at a directory of its own, once.
///
/// # Why a test suite needs this at all
///
/// Building an editor *creates* its configuration directory and then
/// evaluates what is in it. Under a person's real `HOME` that means `cargo
/// test` writing into their configuration and then running it, so the suite's
/// behaviour depends on whose machine it is on.
///
/// Worse, `HOME` is not stable during a run. Several tests redirect it at a
/// sandbox to check path expansion, and the redirection is process-wide while
/// the test runner is threaded -- so an editor built by *any* concurrent test
/// landed its configuration inside whichever sandbox happened to be installed
/// at that instant. The symptom was a listing of a two-file sandbox that
/// occasionally had a `.config/` in it, failing in whichever test was unlucky
/// rather than in the one that caused it.
///
/// Naming a directory of this process's own removes both -- and it does so
/// through [`CONFIG_DIR`], the same door a person would use, so the
/// resolution above is *exercised* by the suite rather than bypassed by it.
///
/// # Why it is public
///
/// The binary crate's tests build editors too, and `cfg(test)` is not active
/// in a library compiled as a dependency, so nothing conditional in here can
/// reach them. Hidden from the documentation because it is scaffolding rather
/// than a feature.
#[doc(hidden)]
pub fn isolate_config_for_tests() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("rsedit-test-config-{}", std::process::id()));
        // SAFETY: the environment is a process-wide table, and writing to it
        // races with any concurrent read. That is why this happens exactly
        // once and as early as it can -- before the first editor exists, and
        // so before anything has read `RSEDIT_CONFIG_DIR`. The value is never
        // changed and never removed, so there is one write in the life of the
        // process and every later reader sees the same answer.
        //
        // The residual is the same one `tests::home_guard` carries: a thread
        // reading some *other* variable at this instant is racing with this
        // write, because the table is one table. Doing it first, once, is what
        // keeps that window as small as it can be made without taking the
        // environment out of the picture entirely.
        unsafe { std::env::set_var(CONFIG_DIR, dir) };
    });
}

pub fn create_global_env<B: BufferTrait>()
-> Result<(EditorState<B>, Arc<Env<EditorState<B>>>), EvalError<EditorState<B>>> {
    // Before anything reads the configuration, which is the whole of the
    // ordering argument in `isolate_config_for_tests`. The binary crate's
    // tests cannot be reached from here -- `cfg(test)` is not active in a
    // library compiled as a dependency -- so they call it themselves.
    #[cfg(test)]
    isolate_config_for_tests();

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

    let user_config_path = user_config_path();

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

    // The third background job, scheduled here rather than beside the other
    // two: it looks *outwards*, at whether the files the buffers came from are
    // still what they were, and both the interval it runs at and the switch
    // that turns it off are settings -- which means it needs the environment,
    // and the environment does not exist until here.
    let _ = editor_state.send_to_worker(WorkerMessage::Schedule {
        task: Box::new(FileWatcher::new(env.clone())),
        interval: watch_interval(&env),
    });
    // And the fourth: a copy on disk of whatever has not been saved, for the
    // one loss nothing else in this tier covers -- the editor stopping without
    // being asked. See `crate::modes::autosave`.
    let _ = editor_state.send_to_worker(WorkerMessage::Schedule {
        task: Box::new(AutoSaver::new(env.clone())),
        interval: auto_save_interval(&env),
    });

    editor_state.eval_file(
        user_config_path
            .to_str()
            .expect("Failed to retrieve a valid String from user_config_path"),
        env.clone(),
    )?;

    Ok((editor_state, env))
}
