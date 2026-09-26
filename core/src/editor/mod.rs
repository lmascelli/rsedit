//! The editor facade, and how it is laid out across this directory.
//!
//! [`EditorState`] owns one handle per compartment (see [`crate::managers`])
//! and nothing else that matters. Its own methods are spread over the files
//! here, under one rule:
//!
//! > A file named after a compartment holds that compartment's facade -- the
//! > methods that forward to it -- plus any coordination that spans several.
//! > A method that forwards to exactly one compartment lives in that
//! > compartment's file, whatever it is called.
//!
//! So `buffers.rs`, `windows.rs`, `commands.rs`, `modes.rs`, `kill_yank.rs`,
//! `runtime.rs` and `log.rs` each pair with the manager of the same name. The
//! rest are named after a *concern* rather than a compartment, because what
//! they do spans several by nature: `keys.rs` resolves a keystroke,
//! `mouse.rs` turns a pointer event into a command, `frame.rs` composes what
//! the renderer draws, `boot.rs` brings an editor into existence and
//! `settings.rs` reads the Lisp variables.
//!
//! # Why the forwarders are not just removed
//!
//! They look like duplication -- `is_command` forwarding to
//! `Commands::is_command` -- and mostly they rename: `Commands::specs` and
//! `Commands::names` are unambiguous because the type says what they are
//! about, where `EditorState` needs `command_specs` and `command_names`
//! because it has buffer names and window ids too. Collapsing the two layers
//! would make one naming scheme serve both, and would put "which compartment
//! holds this" into every call site -- so that moving a field between
//! compartments edited every caller instead of one forwarder.

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

pub(crate) mod boot;
mod buffers;
mod commands;
mod frame;
mod keys;
mod kill_yank;
mod log;
mod modes;
mod mouse;
mod runtime;
mod settings;
mod windows;

pub use boot::create_global_env;
pub use settings::*;

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

    /// Ask the runtime something. Same rules as [`EditorState::windows`].
    pub(crate) fn runtime<R>(&self, f: impl FnOnce(&Runtime) -> R) -> R {
        f(&self.runtime.read().expect("read lock on runtime"))
    }

    /// Change it.
    pub(crate) fn runtime_mut<R>(&self, f: impl FnOnce(&mut Runtime) -> R) -> R {
        f(&mut self.runtime.write().expect("write lock on runtime"))
    }
}
