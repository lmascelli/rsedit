//! The editor facade: [`EditorState`], its fields, and the doors through which
//! every compartment is reached. Its methods are spread over the files here: a
//! file named after a compartment holds that compartment's half of the facade,
//! and the others are named after what they coordinate.
//!
//! Design: `doc/architettura-editor.typ, <coppie>`.

use crate::{
    ELispExp,
    buffer::{Buffer, BufferTrait, FileStamp},
    commands::{ArgSpec, Invocation, PrefixArg},
    feature::isearch::install_isearch,
    feature::minibuffer::install_minibuffer,
    input::{
        KeyEvent, Keymap, MouseButton, MouseEvent, MouseKind, OnUnbound, TransientKeymap,
        describe_keys, fill_default_keymaps,
    },
    jobs::autosave::{AutoSaver, auto_save_directory},
    jobs::highlighter::Highlighter,
    jobs::prescan::Prescanner,
    jobs::watcher::FileWatcher,
    managers::{
        Binding, BufferRemoved, BufferRenamed, Buffers, ClickCount, Commands, History, Hit,
        KillYank, Log, Macros, Modes, MouseDrag, Runtime, Scrolled, WindowRemoved, Windows,
    },
    modes::{Grammar, MajorMode, SyntaxTable},
    primitives::{install_primitives, movement::goto_offset},
    text::isearch::Isearch,
    text::kill_ring::Direction,
    ui::{
        ComposeSettings, Division, Face, FloatingWindow, Focus, FrameSnapshot, GutterSpec,
        LineNumbers, Orientation, Rect, RenderableWindowView, Separator, Side, Style, Theme,
        Window, WindowId, compose_layout, layout, region_highlights,
    },
    work::{Stop, Topic, WorkId, WorkManager},
};

use risp::{
    DEFAULT_FUEL, Env, EvalError, FuelMeter, FuelScope, LispContext, Parser, bootstrap_vm,
    call_callable, eval,
};

use std::{
    fs::{self, File},
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

mod background;
pub(crate) mod boot;
mod buffers;
mod commands;
mod frame;
mod keys;
mod kill_yank;
mod log;
mod macros;
mod modes;
mod mouse;
mod replace;
mod results;
mod runtime;
mod settings;
mod windows;
mod work;

pub use boot::{create_global_env, isolate_config_for_tests};
pub(crate) use frame::TURN_INTERVAL;
pub use keys::KEY_CAPTURE_FUNCTION;
pub use settings::*;
pub(crate) use work::{BUFFER_CHANGED, SYNTAX_CHANGED, WorkerTurn, in_worker};

/// What the echo area shows, and since when.
///
/// The text and its instant share one lock, so that a reader never pairs a
/// new message with the old one's instant, and hides it as soon as it is
/// shown.
#[derive(Debug, Clone, PartialEq)]
pub struct EchoMessage {
    pub text: String,
    /// When [`EditorState::set_echo_message`] wrote `text`: each message is
    /// shown for its whole timeout, however soon it followed the last.
    pub set_at: Instant,
}

impl EchoMessage {
    /// Returns TEXT as a message set now.
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            set_at: Instant::now(),
        }
    }

    /// Returns how long the message has been shown, or `None` once TIMEOUT
    /// has run out, and `None` when there is no message: an empty echo area
    /// is never waiting to expire.
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

/// The editor: its buffers and windows, its modes and commands, the keys being
/// typed, and the work running in the background -- over B, the storage of a
/// buffer's text.
///
/// A handle: every field is shared, so a clone is cheap and sees the same
/// editor, which is how the threads of the background work act on it. The
/// fields are private; each compartment is reached through its doors, below.
#[derive(Clone)]
pub struct EditorState<B: BufferTrait> {
    /// False once `quit` has been asked for.
    running: Arc<AtomicBool>,

    /// Where asynchronous work runs: colouring, the prescanner, the file
    /// watcher, the auto-saver and the Lisp jobs as jobs, shell commands and
    /// searches on threads of their own. See [`crate::work`].
    ///
    /// In this crate's tests it is in manual mode: no job runs unless the test
    /// drives it. A dedicated thread is a real one in both modes.
    work: WorkManager<EditorState<B>>,

    /// Whether a job has changed the screen since the last snapshot, and how
    /// the host is told: raised by the WorkManager after every step. See
    /// [`EditorState::on_background_change`].
    background_change: Arc<work::BackgroundChange>,

    /// Every buffer, which one is current, and which were current lately. See
    /// [`Buffers`].
    buffers: Arc<RwLock<Buffers<B>>>,
    /// The echo area's text, with when it was set.
    echo_message: Arc<RwLock<EchoMessage>>,

    /// The registry of the modes, the keymaps, the completion sources, the
    /// file-name patterns, the transient keymap, the repeat keys and the
    /// global hooks. See [`Modes`].
    modes: Arc<RwLock<Modes<B>>>,

    /// How many times a grammar of the registry has changed: what a colouring
    /// is stamped with, so that one computed under an older grammar is done
    /// again -- see [`crate::buffer::syntax::SyntaxCache::describes`].
    ///
    /// Beside the registry rather than in it, so that a frame asks every
    /// buffer whether its colouring is behind without taking the registry's
    /// lock. It moves only under that lock's write side, once the grammar has
    /// changed, and is read under it with the grammar it describes.
    grammar_epoch: Arc<AtomicU64>,
    /// Emitted when a grammar or a syntax table changes: the editor's
    /// `syntax-changed`, resolved once. See [`work::SYNTAX_CHANGED`].
    ///
    /// Not the epoch above, which moves with it only for a grammar: a syntax
    /// table is the other lexer's, and changing one must not make every
    /// colouring look out of date.
    syntax_changed: Topic,
    /// Every window: the tiled tree, the floats over it, the focus, the next
    /// id, and what the mouse is doing. See [`Windows`].
    windows: Arc<RwLock<Windows>>,

    /// What can be run by name, what is waiting for its arguments, and what
    /// ran last. See [`Commands`].
    commands: Arc<RwLock<Commands<B>>>,

    /// How many shell commands are running: what `shell-command-running-p`
    /// answers. A count, since several can run at once, each into a buffer of
    /// its own.
    shell_commands: Arc<AtomicUsize>,

    /// The buffer whose results `next-error` walks, if any.
    ///
    /// The session's rather than a buffer's: `M-g n` is pressed in the file
    /// being fixed, not in the listing. The results themselves are on their
    /// own buffer.
    current_results: Arc<RwLock<Option<String>>>,

    /// The keyboard macro being recorded, the last one, how deep the replays
    /// are nested, and the counter. See [`Macros`].
    macros: Arc<RwLock<Macros>>,

    /// How many commands have failed: a replay compares it before and after
    /// each key, and stops at the first that failed. A count rather than a
    /// flag, so that nothing has to clear it.
    command_errors: Arc<AtomicUsize>,

    /// Calls that background jobs have asked for and the command thread has
    /// not made yet. See [`background`].
    owed: Arc<RwLock<Vec<background::OwedCallback<B>>>>,

    /// Killed text, what a yank put where, and whether the command before this
    /// one did either. See [`KillYank`].
    kill_yank: Arc<RwLock<KillYank>>,

    /// The keys typed so far of a sequence that is not yet a whole binding.
    ///
    /// The editor's rather than a mode's, so that a sequence begun under one
    /// keymap is not half-remembered by another. Pushed to and cleared rather
    /// than rebuilt: a key that is a whole binding allocates nothing.
    pending_keys: Arc<RwLock<Vec<KeyEvent>>>,

    /// The diagnostics, and the file they are copied to. See [`Log`].
    log: Arc<RwLock<Log>>,

    /// What has been typed at each prompt, and where a walk back through one
    /// has got to. See [`History`].
    history: Arc<RwLock<History>>,

    /// The execution budget, the call stack, the theme, the goal column, and
    /// the search or replace in progress. See [`Runtime`].
    runtime: Arc<RwLock<Runtime>>,
}

impl<B: BufferTrait> LispContext for EditorState<B> {
    fn consume_fuel(&self, amount: u32) -> Result<(), EvalError<EditorState<B>>> {
        // The meter reports a host-agnostic `Exhausted`, and naming it as a
        // Lisp error is the host's part.
        self.runtime(|runtime| runtime.fuel())
            .consume(amount)
            .map_err(|_| EvalError::OutOfFuel)
    }

    fn log_diagnostic(&self, msg: &str) {
        Log::append(&self.log, msg);
    }

    fn with_unwind<R>(&self, cleanup: impl FnOnce() -> R) -> R {
        self.runtime(|runtime| runtime.fuel())
            .with_unwind(100_000, cleanup)
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

// TODO(DOC-2): `LispContext` asks for both, and neither is meant to be used:
// `{:?}` on an editor panics, and so does comparing two.
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

// The doors: the only code that takes a compartment's lock. Each runs a closure
// under it, so that the lock lasts as long as the closure. The closure must not
// call back into the editor, whose methods may take the same lock again: a
// second read on one thread can wait for ever behind a writer.
impl<B: BufferTrait> EditorState<B> {
    /// Runs F on the buffer table, under its read lock.
    pub(crate) fn buffers<R>(&self, f: impl FnOnce(&Buffers<B>) -> R) -> R {
        f(&self.buffers.read().expect("read lock on buffers"))
    }

    /// Runs F on the buffer table, under its write lock.
    pub(crate) fn buffers_mut<R>(&self, f: impl FnOnce(&mut Buffers<B>) -> R) -> R {
        f(&mut self.buffers.write().expect("write lock on buffers"))
    }

    /// Runs F on the windows, under their read lock.
    pub(crate) fn windows<R>(&self, f: impl FnOnce(&Windows) -> R) -> R {
        f(&self.windows.read().expect("read lock on windows"))
    }

    /// Runs F on the windows, under their write lock.
    pub(crate) fn windows_mut<R>(&self, f: impl FnOnce(&mut Windows) -> R) -> R {
        f(&mut self.windows.write().expect("write lock on windows"))
    }

    /// Runs F on the windows, under their write lock, and on the buffer table,
    /// under its read lock: the one door to two compartments, and the order
    /// the editor takes them in, windows first.
    ///
    /// For [`Self::snapshot`], which composes each window against the buffer
    /// it shows, and moves the window's scroll to keep point in view.
    pub(crate) fn windows_mut_with_buffers<R>(
        &self,
        f: impl FnOnce(&mut Windows, &Buffers<B>) -> R,
    ) -> R {
        let mut windows = self.windows.write().expect("write lock on windows");
        let buffers = self.buffers.read().expect("read lock on buffers");
        f(&mut windows, &buffers)
    }

    /// Runs F on the modes, under their read lock.
    ///
    /// The hooks and completion sources they hold are Lisp: F copies them out,
    /// and they are called once it has returned.
    pub(crate) fn modes<R>(&self, f: impl FnOnce(&Modes<B>) -> R) -> R {
        f(&self.modes.read().expect("read lock on modes"))
    }

    /// Runs F on the modes, under their write lock.
    pub(crate) fn modes_mut<R>(&self, f: impl FnOnce(&mut Modes<B>) -> R) -> R {
        f(&mut self.modes.write().expect("write lock on modes"))
    }

    /// Runs F on the commands, under their read lock.
    pub(crate) fn commands<R>(&self, f: impl FnOnce(&Commands<B>) -> R) -> R {
        f(&self.commands.read().expect("read lock on commands"))
    }

    /// Runs F on the commands, under their write lock.
    pub(crate) fn commands_mut<R>(&self, f: impl FnOnce(&mut Commands<B>) -> R) -> R {
        f(&mut self.commands.write().expect("write lock on commands"))
    }

    /// Runs F on the kill ring, under its read lock.
    pub(crate) fn kill_yank<R>(&self, f: impl FnOnce(&KillYank) -> R) -> R {
        f(&self.kill_yank.read().expect("read lock on kill_yank"))
    }

    /// Runs F on the kill ring, under its write lock.
    pub(crate) fn kill_yank_mut<R>(&self, f: impl FnOnce(&mut KillYank) -> R) -> R {
        f(&mut self.kill_yank.write().expect("write lock on kill_yank"))
    }

    /// Runs F on the keyboard macros, under their read lock.
    pub(crate) fn macros<R>(&self, f: impl FnOnce(&Macros) -> R) -> R {
        f(&self.macros.read().expect("read lock on macros"))
    }

    /// Runs F on the keyboard macros, under their write lock.
    pub(crate) fn macros_mut<R>(&self, f: impl FnOnce(&mut Macros) -> R) -> R {
        f(&mut self.macros.write().expect("write lock on macros"))
    }

    /// Runs F on what has been typed at the prompts, under its read lock.
    pub(crate) fn history<R>(&self, f: impl FnOnce(&History) -> R) -> R {
        f(&self.history.read().expect("read lock on history"))
    }

    /// Runs F on what has been typed at the prompts, under its write lock.
    pub(crate) fn history_mut<R>(&self, f: impl FnOnce(&mut History) -> R) -> R {
        f(&mut self.history.write().expect("write lock on history"))
    }

    /// Runs F on the log, under its read lock.
    ///
    /// Logging does not come through here: [`Log::append`] takes the lock
    /// itself, for the WorkManager, which logs without an editor.
    pub(crate) fn log<R>(&self, f: impl FnOnce(&Log) -> R) -> R {
        f(&self.log.read().expect("read lock on log"))
    }

    /// Runs F on the log, under its write lock.
    pub(crate) fn log_mut<R>(&self, f: impl FnOnce(&mut Log) -> R) -> R {
        f(&mut self.log.write().expect("write lock on log"))
    }

    /// Runs F on the runtime, under its read lock.
    pub(crate) fn runtime<R>(&self, f: impl FnOnce(&Runtime) -> R) -> R {
        f(&self.runtime.read().expect("read lock on runtime"))
    }

    /// Runs F on the runtime, under its write lock.
    pub(crate) fn runtime_mut<R>(&self, f: impl FnOnce(&mut Runtime) -> R) -> R {
        f(&mut self.runtime.write().expect("write lock on runtime"))
    }
}
