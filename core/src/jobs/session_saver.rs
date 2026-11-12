//! The session saver: the job that writes the session file while the editor
//! runs, every `session-save-interval` seconds, when what it would write is
//! not what the file holds. What the exit writes, written sooner, so that an
//! editor that stops without being asked loses little.
//!
//! Design: `doc/architettura-editor.typ, <memoria>`.
use crate::editor::file_io::Lane;
use crate::editor::session::{DEFAULT_SESSION_SAVE_INTERVAL, SESSION_SAVE_INTERVAL};
use crate::{
    BufferTrait, EditorState,
    work::{Exit, Job, JobError, Spec, Start, Step, StepContext},
};
use risp::{Env, LispContext};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long the job sleeps while it is switched off, before it reads the
/// setting again.
const PARKED: Duration = Duration::from_secs(DEFAULT_SESSION_SAVE_INTERVAL as u64);

/// Returns how often the session is written, as Lisp has it now: the seconds
/// of `session-save-interval` when they are a number of at least one, and
/// `None`, for only at exit, otherwise.
pub fn session_save_interval<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
    env.number_at_least(SESSION_SAVE_INTERVAL, 1.0)
        .map(|seconds| Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX))
}

/// Writes the session file in a directory, at intervals. It carries the
/// environment, as the auto-saver does, for the settings that pace it and say
/// what a session holds.
pub struct SessionSaver<B: BufferTrait> {
    env: Arc<Env<EditorState<B>>>,
    /// The state directory, or `None` when there is none to write in.
    state: Option<PathBuf>,
    /// Whether the last write failed: what keeps the log from hearing about
    /// it again at every turn until one works.
    failing: Arc<AtomicBool>,
}

impl<B: BufferTrait> SessionSaver<B> {
    /// Returns a saver that reads its settings from ENV and writes in STATE.
    pub fn new(env: Arc<Env<EditorState<B>>>, state: Option<PathBuf>) -> Self {
        Self {
            env,
            state,
            failing: Arc::default(),
        }
    }

    /// Returns how it is scheduled: in the normal class, first writing one
    /// interval from now, since the session just read is what the file holds.
    pub(crate) fn spec(&self) -> Spec {
        Spec::new("session-saver")
            .unit("writes")
            .start(Start::After(
                session_save_interval(&self.env).unwrap_or(PARKED),
            ))
    }
}

impl<B: BufferTrait> Job<EditorState<B>> for SessionSaver<B> {
    fn step(&mut self, cx: &mut StepContext<'_, EditorState<B>>) -> Result<Step, JobError> {
        // Switched off, it is parked rather than ended, as the auto-saver is.
        let Some(interval) = session_save_interval(&self.env) else {
            return Ok(Step::Sleep(PARKED));
        };
        if let Some(state) = self.state.clone() {
            // On a thread of the file operations, not on the executor's, which
            // colours too: the write is a disk's. One in progress is left to
            // finish, and this turn writes nothing.
            let (worker, env) = (cx.state.clone(), self.env.clone());
            let failing = self.failing.clone();
            cx.state.start_file_io(
                Lane::Recovery,
                "session".into(),
                "Saving the session",
                None,
                crate::editor::file_io::timeout(&self.env),
                move || worker.write_session(&env, Ok(state)),
                move |editor, _, written| match written {
                    Ok(_) => failing.store(false, Ordering::Relaxed),
                    Err(why) => {
                        if !failing.swap(true, Ordering::Relaxed) {
                            editor.log_diagnostic(&format!("[WARNING] {why}"));
                        }
                    }
                },
            );
            cx.worked(1, "session writes");
        }
        Ok(Step::Sleep(interval))
    }

    fn exit(&mut self, state: &EditorState<B>, why: Exit) {
        if why == Exit::Panicked {
            let (env, directory) = (self.env.clone(), self.state.clone());
            state.restart_after_panic("session saver", move || {
                let saver = SessionSaver::new(env, directory);
                (saver.spec(), Box::new(saver))
            });
        }
    }
}
