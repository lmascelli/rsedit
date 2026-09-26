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
    search::Isearch,
    ui::{Face, Style, Theme},
};
use std::sync::Arc;

pub struct Runtime {
    /// Column that repeated vertical movement is aiming for.
    goal_column: Option<usize>,
    /// The incremental search currently running, between one keystroke and the
    /// next.
    isearch: Option<Isearch>,
    fuel: Arc<FuelMeter>,
    /// Innermost call last. Frozen at the state of the most recent uncaught
    /// error until something clears it.
    call_stack: Vec<String>,
    theme: Arc<Theme>,
}

impl Runtime {
    pub fn new(fuel: Arc<FuelMeter>) -> Self {
        Self {
            goal_column: None,
            isearch: None,
            fuel,
            call_stack: Vec::new(),
            theme: Arc::new(Theme::default()),
        }
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
