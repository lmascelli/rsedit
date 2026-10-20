//! The things a running editor carries that belong to no document: the
//! execution budget, the call stack, the theme, where vertical movement is
//! aiming, and the search in progress.
//!
//! # What this compartment is, honestly
//!
//! The other five group fields that are *one fact in several pieces* -- a
//! window's layout and its focus, a buffer table and the name of the current
//! one. These five are not that. No operation here touches two of them, and
//! putting them together closes no window and removes no ordering.
//!
//! What it buys is smaller and still worth having: the facade stops carrying
//! five unrelated locks in its field list, and a reader looking for "what is
//! the editor's own state, as opposed to the document's" has one place to
//! look. The grouping is by *lifetime* -- all of this is per-session, none of
//! it is per-buffer -- rather than by a shared invariant, and it should not be
//! read as claiming one.
//!
//! The theme sits here for want of a better home. It is really a UI concern
//! and will likely move when there is a UI compartment to move it to.
//!
//! # The fuel meter keeps its `Arc`
//!
//! [`FuelMeter`] does its own synchronisation -- the live budget is a
//! thread-local, and the atomic behind it is read only when a scope opens --
//! so it needs no lock of its own and gets none. It is held as an `Arc` and
//! handed out by clone, so a caller that wants to open a metered scope does
//! not have to keep this compartment's lock open while it runs.
use crate::{
    text::search::{Isearch, Replace},
    ui::{Face, Style, Theme},
    work::WorkId,
};
use risp::FuelMeter;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct Runtime {
    /// Column that repeated vertical movement is aiming for.
    goal_column: Option<usize>,
    /// The incremental search currently running, between one keystroke and the
    /// next.
    isearch: Option<Isearch>,
    /// The replace in progress, between one answer and the next.
    ///
    /// Beside the search rather than folded into it: the two are the same
    /// *kind* of thing -- a question being asked of the buffer across several
    /// commands -- and neither is ever asked while the other is.
    replace: Option<Replace>,
    fuel: Arc<FuelMeter>,
    /// Innermost call last. Frozen at the state of the most recent uncaught
    /// error until something clears it.
    call_stack: Vec<String>,
    theme: Arc<Theme>,
    /// How every face looks as the editor ships it: the compiled-in defaults,
    /// and the styles modules gave their own faces with `define-face`. What
    /// `reset-faces` puts back, since the defaults alone would leave those
    /// faces plain.
    shipped: Arc<Theme>,
    /// The newest job Lisp started under each name, so that
    /// `running-workers` can tell Lisp's work from the editor's own in the
    /// WorkManager's list: a name is Lisp's while the job running under it
    /// is the one recorded here.
    ///
    /// Only that. Which job a queued callback may still speak for is the
    /// WorkManager's registry's to say, since it changes in one step with the
    /// work it describes: see `WorkManager::is_latest`. Between them they took
    /// the place of a generation counter per name, which did both.
    lisp_jobs: HashMap<String, WorkId>,
}

impl Runtime {
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

    /// Job ID was started from Lisp, under NAME.
    pub fn adopt_lisp_job(&mut self, name: &str, id: WorkId) {
        self.lisp_jobs.insert(name.to_string(), id);
    }

    /// The names under which a job Lisp started is among RUNNING, the jobs
    /// the WorkManager lists, in no particular order.
    ///
    /// Two starts under one name on two threads at once can record the one
    /// that lost: then the name is missing from this answer until the next
    /// start under it. Nothing else depends on it.
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

    /// The meter, cloned out.
    ///
    /// Cloned rather than lent so that opening a metered scope -- which lasts
    /// for a whole command -- does not hold this compartment's lock for the
    /// same span. An `Arc` clone is one atomic increment; a lock held across a
    /// command would be a lock held across the interpreter.
    pub fn fuel(&self) -> Arc<FuelMeter> {
        self.fuel.clone()
    }

    // ------------------------------------------------------------------
    // Where vertical movement is aiming
    // ------------------------------------------------------------------

    pub fn goal_column(&self) -> Option<usize> {
        self.goal_column
    }

    pub fn set_goal_column(&mut self, col: Option<usize>) {
        self.goal_column = col;
    }

    // ------------------------------------------------------------------
    // The search in progress
    // ------------------------------------------------------------------

    pub fn begin_isearch(&mut self, session: Isearch) {
        self.isearch = Some(session);
    }

    pub fn take_isearch(&mut self) -> Option<Isearch> {
        self.isearch.take()
    }

    pub fn isearch_active(&self) -> bool {
        self.isearch.is_some()
    }

    // ------------------------------------------------------------------
    // The replace in progress
    // ------------------------------------------------------------------

    pub fn begin_replace(&mut self, session: Replace) {
        self.replace = Some(session);
    }

    /// The session, to be read. `None` when none is running.
    pub fn replace(&self) -> Option<&Replace> {
        self.replace.as_ref()
    }

    /// The session, to be advanced.
    pub fn replace_mut(&mut self) -> Option<&mut Replace> {
        self.replace.as_mut()
    }

    pub fn take_replace(&mut self) -> Option<Replace> {
        self.replace.take()
    }

    // ------------------------------------------------------------------
    // The call stack
    // ------------------------------------------------------------------

    pub fn push_call_frame(&mut self, frame: &str) {
        self.call_stack.push(frame.to_string());
    }

    pub fn pop_call_frame(&mut self) {
        self.call_stack.pop();
    }

    pub fn call_frame_depth(&self) -> usize {
        self.call_stack.len()
    }

    pub fn truncate_call_frames(&mut self, depth: usize) {
        self.call_stack.truncate(depth);
    }

    /// The captured stack, innermost first.
    pub fn backtrace(&self) -> Vec<String> {
        let mut frames = self.call_stack.clone();
        frames.reverse();
        frames
    }

    pub fn clear_backtrace(&mut self) {
        self.call_stack.clear();
    }

    // ------------------------------------------------------------------
    // The theme
    // ------------------------------------------------------------------

    pub fn theme(&self) -> Arc<Theme> {
        self.theme.clone()
    }

    pub fn face_style(&self, face: Face) -> Style {
        self.theme.style(face)
    }

    pub fn set_face_style(&mut self, face: Face, style: Style) {
        Arc::make_mut(&mut self.theme).set(face, style);
    }

    /// Make STYLE what FACE ships with, and draw FACE in it from now on.
    pub fn define_face_style(&mut self, face: Face, style: Style) {
        Arc::make_mut(&mut self.shipped).set(face, style);
        Arc::make_mut(&mut self.theme).set(face, style);
    }

    pub fn reset_theme(&mut self) {
        self.theme = self.shipped.clone();
    }
}
