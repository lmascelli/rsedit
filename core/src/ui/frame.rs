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
use super::{RenderableWindowView, Separator, Theme};

/// Everything the UI needs to draw one frame, owned outright.
///
/// Contains no locks, no `Arc`s into editor state and no borrows, so it can be
/// handed to a renderer on another thread, kept for comparison against the next
/// frame, or asserted on in a test without a terminal anywhere in sight.
/// `Default` is an empty frame: no windows, nothing to say, no size.
///
/// It exists for the callers that care about two fields and have to write the
/// other nine -- chiefly the renderer's tests, which draw one separator or one
/// line of text. Without it, every field added here edits every one of them,
/// and the edit is always the same: repeat the empty value. That churn is not
/// free -- it is a diff to review that says nothing, in a file where a real
/// change would then be easy to miss.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameSnapshot {
    /// Tiled windows first, in layout order, then floating windows in the order
    /// they should be drawn -- later entries paint over earlier ones.
    pub views: Vec<RenderableWindowView>,
    /// The rules between windows sitting side by side, in draw order.
    pub separators: Vec<Separator>,
    /// Text for the echo area, or empty when there is nothing to show.
    pub echo_message: String,
    /// Input the editor is part-way through reading -- a key sequence begun, a
    /// prefix argument being built -- or empty when it is waiting for nothing.
    ///
    /// A field of its own rather than an echo message, because the two are
    /// different kinds of thing. A message reports something that *happened*,
    /// and so has a natural expiry; this reports what is *true right now*, and
    /// expiring it would leave the editor waiting for a key with nothing on
    /// screen to say so.
    pub pending_input: String,
    /// What a transient keymap is offering or asking, or empty when none is
    /// installed -- `[o]` while `C-x o o o` is live, a question while one is
    /// being asked.
    ///
    /// A field of its own rather than an echo message, for the reason
    /// `pending_input` is one: a message reports something that *happened* and
    /// expires, while this reports what is *true right now*. An offer that
    /// vanished after five seconds while the key still worked would be worse
    /// than one never shown.
    ///
    /// Not folded into `pending_input` either, though it is the same kind of
    /// thing: that one appends a trailing hyphen to say a sequence is part-way
    /// typed, and a standing offer is not part-way anything.
    ///
    /// Shown *instead of* `echo_message`: the two compete for one row, and
    /// this is the one the editor is waiting on.
    pub prompt: String,
    /// How each face should be drawn, as it stood at capture time.
    ///
    /// Carried in the frame rather than looked up by the renderer for the same
    /// reason everything else here is: drawing touches no shared state. It also
    /// means a frame is self-describing -- a snapshot kept for comparison still
    /// knows the colours it was composed under.
    pub theme: std::sync::Arc<Theme>,
    /// Which window had focus at capture time. `views` already carries
    /// `is_focused` per window; this is here for renderers that need to know
    /// even when the focused window is not currently visible.
    pub focused_window_id: usize,
    /// Frame size the capture was composed for. A renderer that finds the
    /// terminal has since been resized knows this snapshot is stale rather than
    /// drawing a mis-sized frame.
    pub width: usize,
    pub height: usize,
    /// Whether colouring is still being worked out, so this frame will be
    /// superseded without anybody touching the keyboard.
    ///
    /// # Why the frame has to say this
    ///
    /// Syntax highlighting is computed on another thread, a chunk at a time.
    /// Every other thing that changes the screen is caused by the user, so a
    /// renderer can draw and then block until the next event and be right. This
    /// one is not: the colour arrives on its own, and a renderer blocked on
    /// input sleeps straight through it. The file stays plain until some key is
    /// pressed, which looks like highlighting that only runs on changes -- when
    /// in fact it ran on time and nothing redrew.
    ///
    /// So the frame reports it, and the renderer asks
    /// [`EditorState::next_redraw_in`] how long it may sleep. False is the
    /// normal state and costs nothing: when there is no colouring left to do
    /// and no message about to expire, the renderer blocks indefinitely, as it
    /// did before.
    pub colouring_pending: bool,
}

impl FrameSnapshot {
    /// The view that had focus, if it is on screen.
    pub fn focused_view(&self) -> Option<&RenderableWindowView> {
        self.views.iter().find(|v| v.is_focused)
    }
}
