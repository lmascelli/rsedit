//! Bringing an editor into being: its first state, the Lisp environment wired
//! to it, the settings with their first values, and the configuration loaded.
//!
//! Design: `doc/architettura-editor.typ, <avvio>`.

use super::*;
use crate::paths::{AppDir, CACHE_DIR, CONFIG_DIR, DATA_DIR, DirOptions, app_dir};

/// The `init.lisp` the editor writes for somebody who has none.
///
/// `require` lines and settings, and nothing that needs a macro: the modules
/// it loads are what define the macros, and without one -- no `data/lisp`
/// beside a test binary, a module taken out -- a form using it is an ordinary
/// call, which fails and takes the editor down with it. A command belongs in a
/// module, or in Rust.
///
/// A raw string, where a backslash is a backslash: Lisp written here with the
/// escapes of an ordinary string literal reaches the parser with them still
/// in, and fails only when the command it is in runs. A named constant, so
/// that a test can evaluate it, in a bare environment, as it is written to be.
pub(crate) const DEFAULT_INIT_LISP: &str = r#";; rsedit init.lisp
;; Add your configuration here.
;;
;; Each `require' loads a module once, after the modules it needs, and asking
;; again for one already loaded does nothing. See the `configuration' manual
;; page.
(require 'commands)
(require 'debug)
(require 'common-keymaps)
(require 'indent)          ; what Tab does; a mode plugs its own rule in
(require 'minibuffer)

;; Modules. Each is optional -- comment one out and the editor comes up
;; without it, missing exactly that feature, unless a module still loaded
;; needs it and loads it anyway.
(require 'rust-mode)       ; colouring for Rust source
(require 'risp-mode)       ; colouring and indentation for this editor's own Lisp
(require 'cc-mode)         ; colouring and indentation for C/C++ source
(require 'python-mode)     ; colouring and indentation for Python
(require 'toml-mode)       ; colouring and indentation for TOML
(require 'bash-mode)       ; colouring and indentation for Bash
(require 'dired)           ; a directory in a buffer (C-x d)
;; Makes dired sort directory entries by type (folders, files).
(setq dired-sort 'type)


(require 'completion)      ; Tab shows every candidate at once, in a strip
(require 'preview)         ; C-x r t shows its replacement before you commit to it
(require 'clipboard)       ; kills also go to the system clipboard
(require 'electric-pair)   ; typing "(" gives you "()"
(require 'buffer-list)     ; C-x b to switch, C-x C-b for the whole list
(require 'shell)           ; M-! runs a command and shows what it said
(require 'manpage)         ; C-h m, and K on a word
(require 'help)            ; C-h f, C-h k, C-h b -- the editor describing itself
(require 'risp-repl)       ; M-x risp-repl, a buffer where C-j evaluates Lisp
(require 'occur)           ; M-s o and M-s g, and the lists M-g n walks
(require 'compile)         ; C-c c, and M-g n to walk what it complained about
(require 'theme)           ; C-c t to choose how faces are drawn
(require 'vim)             ; M-x vim-mode enables modal editing in this buffer
;; (vim-mode t)           ; start the initial buffer in Normal state
;; (global-vim-mode t)    ; existing and future text/code buffers
;; Or only selected major modes, even without global-vim-mode:
;; (setq vim-mode-modes '(rust-mode risp-mode))
;; (vim-mode-refresh)     ; also apply changed settings to existing buffers

;; Where completions come from, for C-M-i in a buffer. The command is built in
;; and works without this; what this adds is the five sources it asks. Take one
;; out with `set-completion-functions', or add one of your own for a single
;; mode with `add-completion-function'.
(require 'completion-at-point)

;; A plugin, installed in a directory of its own under `plugins' in the data
;; directory, is found at start and loaded by name as a module is. See the
;; `plugins' manual page.
;; (require 'my-plugin)

;; How much one step of background work may do before it is stopped, and how
;; the scheduler that runs it takes turns. See the `background' manual page.
;; (setq worker-fuel 200000)
;; (set-work-option 'high-steps 4)

;; A copy of unsaved work, for when the editor stops without being asked.
;; `M-x recover-file' brings it back. See the `files' manual page.
;; (setq auto-save nil)
;; (setq auto-save-directory "~/.cache/rsedit")

;; Files changing underneath you: a clean buffer is reloaded, a modified one is
;; flagged and `C-x C-s' asks. See the `files' manual page.
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

/// Returns the editor's WorkManager, which reports to LOG, and whose watchdog
/// writes LOG's file: in manual mode in this crate's tests, with an executor
/// thread everywhere else.
fn work_manager<B: BufferTrait>(log: &Arc<RwLock<Log>>) -> WorkManager<EditorState<B>> {
    let report = {
        let log = log.clone();
        move |line: &str| Log::append(&log, line)
    };
    let work = if cfg!(test) {
        WorkManager::manual(report)
    } else {
        WorkManager::threaded(report)
    };
    // What logs does not write the file: the watchdog does, every turn. See
    // `managers::log`.
    let log = log.clone();
    work.after_watch(move || Log::flush(&log));
    work
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns an editor with the bindings written in Rust, one window showing
    /// `*scratch*`, and the jobs that colour and prescan started, waiting for a
    /// buffer or a grammar to change.
    fn new() -> Self {
        let mut keymaps = Keymap::new();
        fill_default_keymaps(&mut keymaps);

        // Made before the editor, so that the WorkManager's reports can go to
        // it.
        let log = Arc::new(RwLock::new(Log::default()));
        // And the WorkManager before the compartments, so that the topics they
        // emit are its own.
        let work = work_manager(&log);
        let buffers = Buffers::new(work.topic(BUFFER_CHANGED));
        let syntax_changed = work.topic(SYNTAX_CHANGED);
        // Every step may have changed the screen, so every step says so.
        let background_change = Arc::new(work::BackgroundChange::default());
        work.after_step({
            let change = background_change.clone();
            move || change.raise()
        });

        let editor_state = Self {
            running: Arc::new(AtomicBool::new(true)),
            work,
            background_change,
            buffers: Arc::new(RwLock::new(buffers)),
            echo_message: Arc::new(RwLock::new(EchoMessage::new("Welcome to rsedit"))),
            modes: Arc::new(RwLock::new(Modes::new(keymaps))),
            grammar_epoch: Arc::new(AtomicU64::new(0)),
            syntax_changed,
            windows: Arc::new(RwLock::new(Windows::default())),
            commands: Arc::new(RwLock::new(Commands::default())),
            shell_commands: Arc::new(AtomicUsize::new(0)),
            current_results: Arc::new(RwLock::new(None)),
            macros: Arc::new(RwLock::new(Macros::default())),
            command_errors: Arc::new(AtomicUsize::new(0)),
            owed: Arc::new(RwLock::new(Vec::new())),
            kill_yank: Arc::new(RwLock::new(KillYank::default())),
            pending_keys: Arc::new(RwLock::new(Vec::new())),
            runtime: Arc::new(RwLock::new(Runtime::new(Arc::new(FuelMeter::new(
                DEFAULT_FUEL,
            ))))),
            log,
            history: Arc::new(RwLock::new(History::default())),
            loader: Arc::new(RwLock::new(Loader::default())),
            working_directory: Arc::new(RwLock::new(
                crate::paths::starting_directory()
                    .to_string_lossy()
                    .to_string(),
            )),
        };
        // The executor's copy of the editor, which could not exist before the
        // editor did: a cycle, since the editor holds the WorkManager, which
        // `shutdown` breaks.
        editor_state.work.start(editor_state.clone());
        // Colouring runs from here on, a bounded piece at a time, whenever a
        // buffer or a grammar changes: started with the editor, since a job
        // that waits costs nothing, and one never started costs a file with no
        // colour and nothing saying why.
        // TODO(BKG-2): submitted once, so a job that panics is gone for the
        // session, and only the log says so.
        editor_state
            .work
            .submit(Highlighter::spec(), Box::new(Highlighter));
        // And the other lexer over the same text: the scan of balanced
        // expressions that `forward-sexp`, the indenter and `syntax-ppss` read,
        // on the same topics. What it has worked out makes a motion cheap, and
        // where it has not got to yet a motion scans from the top.
        editor_state
            .work
            .submit(Prescanner::spec(), Box::new(Prescanner));

        editor_state
    }
}

/// Returns the environment at the root of ENV's chain, where `init.lisp` is
/// evaluated.
///
/// A `defun`, a `defvar` or the `setq` of a new name binds in the environment
/// it is evaluated in, so one evaluated inside a function is that function's,
/// and goes when it returns. What must outlive the call -- a file being
/// loaded, an expression typed at `M-:` -- is evaluated here instead.
pub(crate) fn top_level<B: BufferTrait>(
    env: &Arc<Env<EditorState<B>>>,
) -> Arc<Env<EditorState<B>>> {
    let mut env = env.clone();
    while let Some(parent) = env.parent.clone() {
        env = parent;
    }
    env
}

/// Points this process's configuration, data and cache at directories of its
/// own, in the system's temporary directory, once: building an editor in a
/// test then neither writes into the configuration of whoever runs the tests,
/// nor loads their plugins, nor depends on where `HOME` points at that
/// instant, which several tests move.
///
/// Through [`CONFIG_DIR`], [`DATA_DIR`] and [`CACHE_DIR`], the doors a person
/// would use, so that the tests go through what `crate::paths` decides.
/// Public for the binary crate's tests, which `cfg(test)` does not reach in a
/// library built as a dependency; hidden from the documentation, since it is
/// scaffolding.
#[doc(hidden)]
pub fn isolate_config_for_tests() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = |what: &str| {
            std::env::temp_dir().join(format!("rsedit-test-{what}-{}", std::process::id()))
        };
        // SAFETY: the environment is one table for the whole process, and a
        // write to it races with any read. So this happens once, and as early
        // as it can -- before the first editor exists, and so before anything
        // has read `RSEDIT_CONFIG_DIR` -- and the values are never changed or
        // removed: one write in the life of the process, and every reader
        // after it sees the same answer. What is left is the race
        // `tests::home_guard` carries too: a thread reading another variable
        // at this instant races with this write, since the table is one.
        unsafe {
            std::env::set_var(CONFIG_DIR, dir("config"));
            std::env::set_var(CACHE_DIR, dir("cache"));
            std::env::set_var(DATA_DIR, dir("data"));
        }
    });
}

/// Sets `lisp-path` and `theme-path` in ENV, each in the order it is
/// searched: the person's own folder under CONFIG, then each plugin's, then
/// the editor's own. The plugins are those under `plugins` in the data
/// directory; they are recorded, and nothing of theirs is loaded.
///
/// The editor's modules are looked for beside the executable only, unlike its
/// themes and pages: a test binary has none there, and so every test starts
/// with no module loaded, whatever its `init.lisp` requires.
fn set_search_paths<B: BufferTrait>(
    editor_state: &EditorState<B>,
    env: &Arc<Env<EditorState<B>>>,
    config: Option<&std::path::Path>,
) {
    let plugins = match app_dir(DirOptions {
        dir: AppDir::Data,
        create: false,
    }) {
        Ok(data) => editor_state.find_plugins(&data.join("plugins")),
        Err(_) => Vec::new(),
    };
    let mut lisp_path: Vec<PathBuf> = config.map(|dir| dir.join("lisp")).into_iter().collect();
    let mut theme_path: Vec<PathBuf> = config.map(|dir| dir.join("themes")).into_iter().collect();
    for plugin in &plugins {
        lisp_path.push(plugin.directory.clone());
        theme_path.push(plugin.directory.join("themes"));
    }
    match std::env::current_exe() {
        Ok(exe) => {
            let shipped = exe
                .parent()
                .expect("Failed to get the parent directory of rsedit")
                .join("data")
                .join("lisp");
            lisp_path.extend(MODULE_KINDS.iter().map(|kind| shipped.join(kind)));
        }
        Err(e) => {
            editor_state.log_diagnostic(&format!(
                "[ERROR] Failed to find the path of rsedit executable. {:?}",
                e
            ));
        }
    }
    theme_path.extend(crate::paths::installed_data_dir().map(|data| data.join("themes")));
    editor_state.loader_mut(|loader| loader.set_plugins(plugins));
    let list = |paths: Vec<PathBuf>| {
        ELispExp::proper_list(
            paths
                .into_iter()
                .map(|path| ELispExp::string(path.display().to_string()))
                .collect(),
        )
    };
    env.set_variable(LISP_PATH.into(), list(lisp_path));
    env.set_variable(THEME_PATH.into(), list(theme_path));
}

/// Builds an editor and the Lisp environment wired to it, and loads the
/// configuration: the interpreter with every primitive and command, the
/// settings with their first values, `init.lisp` -- written first when there
/// is none -- and the jobs that watch the files and save copies of them.
///
/// The environment must not outlive the editor. An error that evaluating
/// `init.lisp` raised is returned.
pub fn create_global_env<B: BufferTrait>()
-> Result<(EditorState<B>, Arc<Env<EditorState<B>>>), EvalError<EditorState<B>>> {
    // Before anything reads the configuration: see `isolate_config_for_tests`.
    // The binary crate's tests, which `cfg(test)` does not reach here, call it
    // themselves.
    #[cfg(test)]
    isolate_config_for_tests();

    let editor_state = EditorState::new();
    let env = bootstrap_vm(&editor_state)?;

    // Off here, and on in the default `init.lisp`: see `mouse_mode`.
    env.set_variable(MOUSE_MODE.into(), ELispExp::nil());

    // The functions called after a resize, each with the new width and height.
    env.set_variable(AFTER_RESIZE_HOOK.into(), ELispExp::nil());

    // Auto-saving and watching files, on, at the pace they keep when nothing
    // says otherwise; and no file manager until a module installs one.
    env.set_variable(crate::jobs::autosave::AUTO_SAVE.into(), ELispExp::t());
    env.set_variable(
        crate::jobs::autosave::AUTO_SAVE_INTERVAL.into(),
        ELispExp::number(crate::jobs::autosave::DEFAULT_AUTO_SAVE_INTERVAL.as_secs_f64()),
    );
    env.set_variable(
        crate::jobs::autosave::AUTO_SAVE_DIRECTORY.into(),
        ELispExp::nil(),
    );
    env.set_variable(crate::jobs::watcher::WATCH_FILES.into(), ELispExp::t());
    env.set_variable(
        crate::jobs::watcher::WATCH_FILE_INTERVAL.into(),
        ELispExp::number(crate::jobs::watcher::DEFAULT_WATCH_INTERVAL.as_secs_f64()),
    );
    env.set_variable(OPEN_DIRECTORY_CALLBACK.into(), ELispExp::nil());

    // A replacement takes the case of what it replaces, and the replace asks
    // its own question until something installs another.
    env.set_variable(CASE_REPLACE.into(), ELispExp::t());
    env.set_variable(REPLACE_READ_FUNCTION.into(), ELispExp::nil());

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
    env.set_variable(
        crate::feature::minibuffer::HISTORY_LENGTH.into(),
        ELispExp::number(crate::managers::DEFAULT_HISTORY_LENGTH as f64),
    );

    // How candidates are shown and matched, and where the next keys go: the
    // editor's own way until a module sets one of these.
    for hook in [
        COMPLETION_READ_FUNCTION,
        COMPLETION_FILTER_FUNCTION,
        COMPLETION_INVALIDATED_FUNCTION,
        COMMAND_COMPLETION_FUNCTION,
        COMMAND_ARG_COMPLETION_FUNCTION,
        KEY_CAPTURE_FUNCTION,
    ] {
        env.set_variable(hook.into(), ELispExp::nil());
    }

    // What each window's status line shows.
    env.set_variable(
        MODE_LINE_FORMAT.into(),
        ELispExp::string(DEFAULT_MODE_LINE_FORMAT.to_string()),
    );

    // The character the separators between windows are drawn with.
    env.set_variable(
        WINDOW_SEPARATOR.into(),
        ELispExp::string(DEFAULT_WINDOW_SEPARATOR.to_string()),
    );

    // How much one step of background work may evaluate.
    env.set_variable(
        WORKER_FUEL.into(),
        ELispExp::number(crate::primitives::lisp_job::DEFAULT_WORKER_FUEL as f64),
    );

    // The line-number gutter, off, at the width it has when nothing is said.
    env.set_variable(DISPLAY_LINE_NUMBERS.into(), ELispExp::nil());
    env.set_variable(
        DISPLAY_LINE_NUMBERS_WIDTH.into(),
        ELispExp::number(DEFAULT_LINE_NUMBER_DIGITS as f64),
    );

    // Where a listing is shown, and where a visit to one of its entries goes.
    env.set_variable(RESULTS_VISIT_SAME_WINDOW.into(), ELispExp::nil());

    env.set_variable(RESULTS_REUSE_BUFFER.into(), ELispExp::nil());

    env.set_variable(
        RESULTS_DISPLAY_LOCATION.into(),
        ELispExp::symbol("bottom".into()),
    );

    document_settings(&env);

    // The mode of a buffer that no other mode claims.
    editor_state.set_mode(
        "fundamental-mode",
        MajorMode::new("fundamental-mode".into()),
    );

    // The primitives and the commands, and the two features written in Rust.
    install_primitives(&editor_state, &env);
    install_minibuffer(&editor_state, env.clone());
    install_isearch(&editor_state, env.clone());

    // The configuration, and before it `rsedit-path`, the executable's path.
    let current_exe_path =
        std::env::current_exe().expect("Failed to locate the path of rsedit executable");
    env.set_variable(
        "rsedit-path".into(),
        ELispExp::string(format!("{}", current_exe_path.display())),
    );

    // The configuration's directory, made if it is not there yet, and its
    // `init.lisp`, written from the default first when it is missing. Where
    // the directory is, on each system, is `crate::paths`'s to say; when there
    // is none to be had the path is empty, which `eval_file` reads as nothing
    // to evaluate.
    let config_dir = match app_dir(DirOptions {
        dir: AppDir::Config,
        create: true,
    }) {
        Ok(dir) => Some(dir),
        Err(why) => {
            editor_state.log_diagnostic(&format!("[ERROR] No configuration: {why}"));
            None
        }
    };
    let user_config_path = config_dir
        .as_ref()
        .map_or_else(PathBuf::new, |dir| dir.join("init.lisp"));

    // Where the modules and the themes are, the plugins among them, before
    // the configuration that requires them.
    set_search_paths(&editor_state, &env, config_dir.as_deref());

    // TODO(AVV-5): written only when missing, so one written by an older
    // editor never loads the modules added since.
    if !user_config_path.as_os_str().is_empty()
        && !user_config_path.exists()
        && let Err(err) = fs::write(&user_config_path, DEFAULT_INIT_LISP)
    {
        editor_state.log_diagnostic(&format!(
            "[ERROR] Failed to write default user configuration {}",
            err
        ));
    }

    // TODO(AVV-7): the first error stops the editor, a module's included, and
    // `main` exits with nothing but the error's name.
    editor_state.eval_file(
        user_config_path
            .to_str()
            .expect("Failed to retrieve a valid String from user_config_path"),
        env.clone(),
    )?;

    // The third job, submitted here and not with the other two: it looks
    // outward, at whether the buffers' files are still what they were, and how
    // often it looks and the switch that turns it off are settings. So it
    // takes the environment, and comes after `init.lisp`, whose settings then
    // count from the first look; it reads both again at every step.
    let watcher = FileWatcher::new(env.clone());
    editor_state.work.submit(watcher.spec(), Box::new(watcher));
    // And the fourth: a copy on disk of what has not been saved, for the one
    // loss nothing else covers, the editor stopping without being asked. Its
    // settings likewise. See `crate::jobs::autosave`.
    let saver = AutoSaver::new(env.clone());
    editor_state.work.submit(saver.spec(), Box::new(saver));

    Ok((editor_state, env))
}
