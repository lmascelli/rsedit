//! One frame, captured for a front end to draw: owned data, taken under the
//! editor's locks and drawn without them.
//!
//! Design: `doc/architettura-editor.typ, <snapshot>`.
use super::{RenderableWindowView, Separator, Theme, WindowId};

/// Everything a front end needs to draw one frame, owned outright: no lock,
/// no `Arc` into the editor's state and no borrow, so it can be drawn on
/// another thread, kept to compare with the next, or asserted on in a test.
///
/// Something new to draw is a field here, filled in by
/// [`EditorState::snapshot`], and never a lock the renderer takes. `Default`
/// is an empty frame, for the tests that care about a field or two.
///
/// [`EditorState::snapshot`]: crate::EditorState::snapshot
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameSnapshot {
    /// The tiled windows, in layout order, then the floating ones in the
    /// order they are drawn: later entries paint over earlier ones.
    pub views: Vec<RenderableWindowView>,

    /// The rules between windows side by side, in the order they are drawn.
    pub separators: Vec<Separator>,

    /// The echo area's text, or empty when there is nothing to show.
    pub echo_message: String,

    /// What the editor is part-way through reading -- a key sequence begun, a
    /// prefix argument being built -- or empty when it is waiting for nothing.
    pub pending_input: String,

    /// What a transient keymap is offering or asking, or empty when none is
    /// installed. Shown in place of [`Self::echo_message`]: they share one
    /// row, and this is what the editor is waiting on.
    pub prompt: String,

    /// How each face is drawn, as the theme stood at capture time.
    pub theme: std::sync::Arc<Theme>,

    /// The window with focus at capture time, on screen or not.
    pub focused_window_id: WindowId,

    /// The size the frame was composed for: a renderer that finds the
    /// terminal resized since knows the frame is stale.
    pub width: usize,
    pub height: usize,

    /// Whether colouring is still being worked out, so that this frame will
    /// be superseded with nothing typed. The renderer asks
    /// [`EditorState::next_redraw_in`] how long it may sleep.
    ///
    /// [`EditorState::next_redraw_in`]: crate::EditorState::next_redraw_in
    pub colouring_pending: bool,

    /// Text the editor owes to the system clipboard, or `None`. Taken from
    /// the editor by the capture, so it is on one frame only; a renderer
    /// with no clipboard to offer ignores it.
    pub clipboard: Option<String>,
}

impl FrameSnapshot {
    /// Returns the view that had focus, if it is on screen.
    pub fn focused_view(&self) -> Option<&RenderableWindowView> {
        self.views.iter().find(|v| v.is_focused)
    }
}
