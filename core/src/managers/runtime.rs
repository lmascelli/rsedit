//! What a running editor carries that belongs to no document: the execution
//! budget, the call stack, the theme, where vertical movement is aiming, the
//! search and the replace in progress, and the jobs Lisp started. Grouped by
//! lifetime, the session's, rather than by an invariant they share.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
use crate::{
    text::{isearch::Isearch, replace::Replace},
    ui::{Face, Style, Theme},
    work::WorkId,
};
use risp::FuelMeter;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The session's own state: see the module.
pub struct Runtime {
    /// Column that repeated vertical movement is aiming for.
    goal_column: Option<usize>,
    /// The incremental search currently running, between one keystroke and the
    /// next.
    isearch: Option<Isearch>,
    /// The replace in progress, between one answer and the next.
    replace: Option<Replace>,
    /// The execution budget. It synchronises itself, so it is held by `Arc`
    /// and handed out by clone rather than lent under this lock.
    fuel: Arc<FuelMeter>,
    /// Innermost call last. Frozen at the state of the most recent uncaught
    /// error until something clears it.
    call_stack: Vec<String>,
    /// How faces are drawn now.
    // TODO(improve): the theme is the UI's, and belongs in a UI compartment
    // when there is one.
    theme: Arc<Theme>,
    /// How every face looks as the editor ships it: the compiled-in defaults,
    /// and the styles modules gave their own faces with `define-face`. What
    /// `reset-faces` puts back, since the defaults alone would leave those
    /// faces plain.
    shipped: Arc<Theme>,
    /// The newest job Lisp started under each name, so that
    /// `running-workers` can tell Lisp's jobs from the editor's own in the
    /// WorkManager's list. Which job a queued callback speaks for is the
    /// WorkManager's registry's to say: see `WorkManager::is_latest`.
    lisp_jobs: HashMap<String, WorkId>,
}

impl Runtime {
    /// Creates the runtime, metering with FUEL and drawing faces as the editor
    /// ships them.
    pub fn new(fuel: Arc<FuelMeter>) -> Self {
        let shipped = Arc::new(Theme::default());
        Self {
            goal_column: None,
            isearch: None,
            replace: None,
            fuel,
            call_stack: Vec::new(),
            theme: shipped.clone(),
            shipped,
            lisp_jobs: HashMap::new(),
        }
    }

    // ------------------------------------------------------------------
    // Lisp jobs
    // ------------------------------------------------------------------

    /// Records that job ID was started from Lisp, under NAME.
    pub fn adopt_lisp_job(&mut self, name: &str, id: WorkId) {
        self.lisp_jobs.insert(name.to_string(), id);
    }

    /// Returns the names whose recorded job is among RUNNING, the jobs the
    /// WorkManager lists, in no particular order.
    ///
    /// Two starts under one name on two threads at once may record the one
    /// that lost: the name is then missing until the next start under it.
    /// Nothing else depends on it.
    pub fn lisp_jobs_among(&self, running: &HashSet<WorkId>) -> Vec<String> {
        self.lisp_jobs
            .iter()
            .filter(|(_, id)| running.contains(id))
            .map(|(name, _)| name.clone())
            .collect()
    }

    // ------------------------------------------------------------------
    // The execution budget
    // ------------------------------------------------------------------

    /// Returns the meter, cloned out: a metered scope lasts a whole command,
    /// and must not hold this lock across the interpreter.
    pub fn fuel(&self) -> Arc<FuelMeter> {
        self.fuel.clone()
    }

    // ------------------------------------------------------------------
    // Where vertical movement is aiming
    // ------------------------------------------------------------------

    /// Returns the column vertical movement is aiming for.
    pub fn goal_column(&self) -> Option<usize> {
        self.goal_column
    }

    /// Sets the column vertical movement aims for, or forgets it.
    pub fn set_goal_column(&mut self, col: Option<usize>) {
        self.goal_column = col;
    }

    // ------------------------------------------------------------------
    // The search in progress
    // ------------------------------------------------------------------

    /// Keeps SESSION as the search in progress.
    pub fn begin_isearch(&mut self, session: Isearch) {
        self.isearch = Some(session);
    }

    /// Takes the search in progress out, leaving none.
    pub fn take_isearch(&mut self) -> Option<Isearch> {
        self.isearch.take()
    }

    /// Returns whether a search is in progress.
    pub fn isearch_active(&self) -> bool {
        self.isearch.is_some()
    }

    // ------------------------------------------------------------------
    // The replace in progress
    // ------------------------------------------------------------------

    /// Keeps SESSION as the replace in progress.
    pub fn begin_replace(&mut self, session: Replace) {
        self.replace = Some(session);
    }

    /// Returns the replace in progress, to be read.
    pub fn replace(&self) -> Option<&Replace> {
        self.replace.as_ref()
    }

    /// Returns the replace in progress, to be advanced.
    pub fn replace_mut(&mut self) -> Option<&mut Replace> {
        self.replace.as_mut()
    }

    /// Takes the replace in progress out, leaving none.
    pub fn take_replace(&mut self) -> Option<Replace> {
        self.replace.take()
    }

    // ------------------------------------------------------------------
    // The call stack
    // ------------------------------------------------------------------

    /// Pushes FRAME onto the call stack.
    pub fn push_call_frame(&mut self, frame: &str) {
        self.call_stack.push(frame.to_string());
    }

    /// Pops the innermost frame.
    pub fn pop_call_frame(&mut self) {
        self.call_stack.pop();
    }

    /// Returns how many frames the call stack holds.
    pub fn call_frame_depth(&self) -> usize {
        self.call_stack.len()
    }

    /// Drops the frames beyond the first DEPTH.
    pub fn truncate_call_frames(&mut self, depth: usize) {
        self.call_stack.truncate(depth);
    }

    /// Returns the captured stack, innermost first.
    pub fn backtrace(&self) -> Vec<String> {
        let mut frames = self.call_stack.clone();
        frames.reverse();
        frames
    }

    /// Empties the call stack.
    pub fn clear_backtrace(&mut self) {
        self.call_stack.clear();
    }

    // ------------------------------------------------------------------
    // The theme
    // ------------------------------------------------------------------

    /// Returns the theme faces are drawn in.
    pub fn theme(&self) -> Arc<Theme> {
        self.theme.clone()
    }

    /// Returns how FACE is drawn.
    pub fn face_style(&self, face: Face) -> Style {
        self.theme.style(face)
    }

    /// Draws FACE in STYLE from now on.
    pub fn set_face_style(&mut self, face: Face, style: Style) {
        Arc::make_mut(&mut self.theme).set(face, style);
    }

    /// Makes STYLE what FACE ships with, and draws FACE in it from now on.
    pub fn define_face_style(&mut self, face: Face, style: Style) {
        Arc::make_mut(&mut self.shipped).set(face, style);
        Arc::make_mut(&mut self.theme).set(face, style);
    }

    /// Draws every face as the editor ships it again.
    pub fn reset_theme(&mut self) {
        self.theme = self.shipped.clone();
    }
}
