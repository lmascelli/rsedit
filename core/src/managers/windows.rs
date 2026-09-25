//! Every window the frame has, and the only code allowed to change them.

use crate::ui::{Division, FloatingWindow, LayoutNode, Orientation, Rect, SplitPath, Window, WindowId};

pub struct Windows {
    root: LayoutNode,
    floating: Vec<FloatingWindow>,
    focused: WindowId,
    next_id: usize,
    drag: Option<MouseDrag>,
}

impl Windows {
    /// Split the focused window, and return the new window's id.
    ///
    /// Focus stays where it was then typing continues in the window you were already in.
    /// Split the focused window, giving the two halves DIVISION.
    pub(crate) fn split_focused_window(
        &self,
        orientation: Orientation,
        division: Division,
    ) -> Option<WindowId> {
        // The new window shows the same buffer, scrolled the same way, so a
        // split looks like what it is: one view becoming two of the same
        // thing rather than a jump somewhere else.
        let existing = self.layout.window(self.focused)?.clone();
        let new_window = Window {
            id: self.next_id,
            ..existing
        };
        layout
            .split_window(focused, orientation, new_window, division)
            .then_some(new_id)
    }

    /// Open a full-width window of exactly HEIGHT rows at the bottom of the
    /// frame, showing BUFFER, and return its id.
    ///
    /// # What makes this different from a split
    ///
    /// It divides the *whole frame* rather than one window, so it appears below
    /// everything and every window above it gives up a share of the space. That
    /// is what a strip is: a thing the frame has, not a thing one window was
    /// cut in half to make.
    ///
    /// # Focus does not move
    ///
    /// Deliberately, and it is the property everything else rests on. A strip
    /// is shown *while something else is being typed into* -- completions
    /// beneath a prompt being the case this was built for -- and focus moving
    /// would make the strip's buffer current, so the next keystroke would be
    /// typed into the list of suggestions instead of into the prompt.
    pub fn open_bottom_window(&mut self, buffer: &str, height: usize) -> WindowId {
        let window = Window {
            show_mode_line: false,
            ..Window::new(self.next_id, buffer)
        };
        let existing = std::mem::replace(&self.root, LayoutNode::Leaf(window.clone()));
        self.root = LayoutNode::Split {
            orientation: Orientation::Horizontal,
            division: Division::SecondFixed(height),
            left: Box::new(existing),
            right: Box::new(LayoutNode::Leaf(window)),
        };
        self.next_id
    }
}

/// What a held mouse button is in the middle of doing.
#[derive(Clone, Debug)]
pub(crate) enum MouseDrag {
    /// Extending a selection inside one window.
    ///
    /// `at` is where the pointer was last seen, in frame cells. Carried
    /// because a drag that has left the window keeps going while the pointer
    /// sits still, and a pointer sitting still sends no events -- so the only
    /// record of where it is, is this one.
    Text {
        window: WindowId,
        at: (isize, isize),
    },
    /// Moving the boundary of one split. `last` is the position along the axis
    /// the boundary moves in, so each event can ask how far it has come since
    /// the one before it.
    Divider {
        path: SplitPath,
        orientation: Orientation,
        last: isize,
    },
}

/// How often a drag held outside its window scrolls it.
///
/// Fast enough to feel continuous, slow enough that a line is still a unit you
/// can stop on.
pub const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(60);

pub const MOUSE_MODE: &str = "mouse-mode";

/// Whether the editor is reading the mouse, as Lisp currently defines it.
pub fn mouse_mode<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.get_variable(MOUSE_MODE)
        .is_some_and(|value| !value.is_nil())
}

/// What the pointer is over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Inside a tiled window's text. LINE and COLUMN are buffer coordinates
    /// with the window's scroll already added; COLUMN may be past the end of
    /// its line, which the command clamps against the buffer rather than the
    /// geometry -- the screen has no opinion about how long a line is.
    Text {
        window: WindowId,
        line: usize,
        column: usize,
    },
    /// A tiled window's status line.
    ///
    /// Named by its window rather than by the split it divides, because a
    /// *click* on one does nothing and only a drag needs to know: which split
    /// a status line belongs to is a question about the tree, and asking it on
    /// every pointer move would be work for nothing.
    ModeLine { window: WindowId },
    /// The rule drawn between two windows side by side, and the split it
    /// divides.
    Separator {
        path: SplitPath,
        orientation: Orientation,
    },
    /// A floating window -- a prompt, a completion strip.
    ///
    /// Reported rather than ignored so that a click on one is *swallowed*. A
    /// float is drawn over a tiled window, so falling through would move point
    /// in a buffer the pointer is not actually over and the user cannot see.
    Floating { window: WindowId },
}

/// A command form with numeric arguments, built rather than parsed.
///
/// Built, because the arguments are numbers the editor just worked out: going
/// through the parser would mean formatting them into text for it to read back.
fn mouse_form<B: BufferTrait>(name: &str, args: &[f64]) -> ELispExp<B> {
    let mut items = vec![ELispExp::symbol(name.into())];
    items.extend(args.iter().copied().map(ELispExp::number));
    ELispExp::form(items)
}

pub const WINDOW_SEPARATOR: &str = "window-separator";
pub const DEFAULT_WINDOW_SEPARATOR: char = '\u{2502}';

fn window_separator<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> char {
    match env.get_variable(WINDOW_SEPARATOR) {
        Some(ELispExp::String(text)) => text.chars().next().unwrap_or(' '),
        Some(value) if value.is_nil() => ' ',
        _ => DEFAULT_WINDOW_SEPARATOR,
    }
}
