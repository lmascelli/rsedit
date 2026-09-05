//! One frame's worth of state, captured for the UI to draw.
//!
//! # Why this exists
//!
//! Rendering used to reach into `EditorState` field by field: it took
//! `layout_root`, then `focused_window_id`, then `buffers` -- releasing and
//! re-acquiring `buffers` three more times for the floating windows -- and then,
//! after all of that had finished and every lock had been dropped, it read
//! `echo_message`. Six-plus acquisitions across five locks, with gaps between
//! them.
//!
//! That is not a hypothetical problem. `BackgroundScheduler` already runs on its
//! own thread with a clone of `EditorState`, and the `(spawn ...)` special form
//! evaluates on more. Anything they mutate between two of those acquisitions
//! produces a frame assembled from two different instants: a floating window
//! whose `lines` were extracted under one acquisition and whose cursor position
//! was computed under the next, reporting a cursor at a position that no longer
//! exists in the text just captured.
//!
//! So capture is now one operation. Every lock is taken once, in a fixed order,
//! held for the whole capture -- which is pure in-memory work -- and released
//! before a single byte reaches the terminal. Drawing then touches no shared
//! state at all.
//!
//! # The rule for adding to it
//!
//! A new renderable feature adds a **field to this struct**, populated inside
//! [`EditorState::snapshot`]. It does not add a lock read to the renderer. The
//! mode-line, the region, syntax faces and window decorations are all coming,
//! and each one that follows this rule costs nothing when the UI event loop
//! eventually moves to its own thread; each one that does not is another torn
//! frame to find later.
use super::RenderableWindowView;

/// Everything the UI needs to draw one frame, owned outright.
///
/// Contains no locks, no `Arc`s into editor state and no borrows, so it can be
/// handed to a renderer on another thread, kept for comparison against the next
/// frame, or asserted on in a test without a terminal anywhere in sight.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSnapshot {
    /// Tiled windows first, in layout order, then floating windows in the order
    /// they should be drawn -- later entries paint over earlier ones.
    pub views: Vec<RenderableWindowView>,
    /// Text for the echo area, or empty when there is nothing to show.
    pub echo_message: String,
    /// Which window had focus at capture time. `views` already carries
    /// `is_focused` per window; this is here for renderers that need to know
    /// even when the focused window is not currently visible.
    pub focused_window_id: usize,
    /// Frame size the capture was composed for. A renderer that finds the
    /// terminal has since been resized knows this snapshot is stale rather than
    /// drawing a mis-sized frame.
    pub width: usize,
    pub height: usize,
}

impl FrameSnapshot {
    /// The view that had focus, if it is on screen.
    pub fn focused_view(&self) -> Option<&RenderableWindowView> {
        self.views.iter().find(|v| v.is_focused)
    }
}
