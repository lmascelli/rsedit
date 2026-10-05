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
    lisp::FuelMeter,
    text::search::{Isearch, Replace},
    ui::{Face, Style, Theme},
};
use std::collections::HashMap;
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
    /// Which run of each named background worker is the live one.
    ///
    /// # Why a number and not a list of live workers
    ///
    /// A worker is a job already sitting in the scheduler's list, and the
    /// scheduler is on another thread; there is no handle to reach in and
    /// remove one. What there is, is the job's own answer each turn to
    /// "should I run again" -- so stopping a worker means arranging for it to
    /// say no.
    ///
    /// A counter does that with no channel and no cancellation flag per
    /// worker. Each turn a job compares the number it was born with against
    /// the number here; defining a worker of the same name again, or stopping
    /// one, bumps it, and the old job retires the next time it wakes. That is
    /// also what makes re-evaluating a Lisp module safe: redefining a worker
    /// replaces it rather than adding a second copy of it.
    ///
    /// A name with no entry has never had a worker.
    workers: HashMap<String, WorkerSlot>,
}

/// What is known about one worker name.
///
/// `issued` only ever goes up, including when a worker is stopped -- that is
/// what makes the retirement stick. Reusing a number would bring a job that
/// had already been told to stop back to life the next time a worker of the
/// same name was defined.
#[derive(Clone, Copy, Debug, Default)]
struct WorkerSlot {
    issued: u64,
    running: bool,
}

impl Runtime {
    pub fn new(fuel: Arc<FuelMeter>) -> Self {
        Self {
            goal_column: None,
            isearch: None,
            replace: None,
            fuel,
            call_stack: Vec::new(),
            theme: Arc::new(Theme::default()),
            workers: HashMap::new(),
        }
    }

    // ------------------------------------------------------------------
    // Background workers
    // ------------------------------------------------------------------

    /// Retire whatever worker is running under NAME and hand out the number
    /// the replacement should be born with.
    ///
    /// One operation rather than a stop and a start, because the two cannot be
    /// allowed to interleave: between them a third caller would see a name
    /// with no live generation and start a worker that the second half then
    /// retired.
    pub fn next_worker_generation(&mut self, name: &str) -> u64 {
        let slot = self.workers.entry(name.to_string()).or_default();
        slot.issued += 1;
        slot.running = true;
        slot.issued
    }

    /// Retire whatever worker is running under NAME. True when there was one.
    ///
    /// The retirement is not immediate: the job finds out the next time it
    /// wakes, which is at most one turn away. Nothing waits for that -- a
    /// caller that did would be blocking the command thread on the worker
    /// thread, which is the arrangement this whole mechanism exists to avoid.
    pub fn stop_worker(&mut self, name: &str) -> bool {
        match self.workers.get_mut(name) {
            Some(slot) if slot.running => {
                slot.issued += 1;
                slot.running = false;
                true
            }
            _ => false,
        }
    }

    /// Record that the worker born as GENERATION of NAME has stopped of its
    /// own accord -- its program finished, or it spent its allowance.
    ///
    /// Ignored when the generation is not the live one, which is the case
    /// that matters: a worker that was replaced and is only now waking up to
    /// notice must not report the *replacement* as having stopped.
    pub fn retire_worker(&mut self, name: &str, generation: u64) {
        if let Some(slot) = self.workers.get_mut(name)
            && slot.issued == generation
        {
            slot.running = false;
        }
    }

    /// Whether GENERATION is still the live run of NAME.
    pub fn worker_is_current(&self, name: &str, generation: u64) -> bool {
        self.workers
            .get(name)
            .is_some_and(|slot| slot.running && slot.issued == generation)
    }

    /// Whether GENERATION is still the *newest* run of NAME -- true even
    /// after it has stopped.
    ///
    /// # Why this is not the same question as the one above
    ///
    /// [`Self::worker_is_current`] asks "should this job take another turn",
    /// and a job that has finished must be told no. This asks "does this job
    /// still speak for its name", and a job that has finished is exactly the
    /// one entitled to say so: the callback it owes is the announcement that
    /// it *has* finished, queued a moment before it retired itself.
    ///
    /// Answering that with `worker_is_current` would drop every one of those,
    /// which is the whole of what a caller waits for. The only thing that
    /// silences a finished job is a *newer* one under the same name, and a
    /// newer one is what bumps `issued`.
    pub fn worker_is_latest(&self, name: &str, generation: u64) -> bool {
        self.workers
            .get(name)
            .is_some_and(|slot| slot.issued == generation)
    }

    /// The names with a worker running, in no particular order.
    pub fn running_workers(&self) -> Vec<String> {
        self.workers
            .iter()
            .filter(|(_, slot)| slot.running)
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

    pub fn reset_theme(&mut self) {
        self.theme = Arc::new(Theme::default());
    }
}
