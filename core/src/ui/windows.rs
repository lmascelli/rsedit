//! The windows and how they share the frame: a window, its geometry, the
//! tree the tiled ones form and the floating ones over it, and how a split
//! divides its rectangle.
//!
//! Design: `doc/architettura-editor.typ, <finestre>`.
use crate::ELispExp;
use crate::buffer::BufferTrait;

/// Which way a split divides its rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// One window above the other.
    Horizontal,
    /// Side by side.
    Vertical,
}

/// A rectangle of cells: its top-left corner, from the frame's, and its size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: isize,
    pub y: isize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    /// Returns whether the cell at (X, Y) is inside this rectangle: from its
    /// corner, included, to its far edges, excluded.
    pub fn contains(&self, x: isize, y: isize) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as isize
            && y < self.y + self.height as isize
    }
}

/// A window's identity, apart from the numbers of lines and columns it sits
/// beside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub usize);

impl std::fmt::Display for WindowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<usize> for WindowId {
    fn from(id: usize) -> Self {
        WindowId(id)
    }
}

impl<B: BufferTrait> From<WindowId> for ELispExp<B> {
    fn from(id: WindowId) -> Self {
        ELispExp::number(id.0 as f64)
    }
}

/// Which window has the keyboard, and whether it is a tiled one: a window
/// without focus follows point when the keyboard is in a floating window.
#[derive(Clone, Copy, Debug)]
pub struct Focus {
    pub id: WindowId,
    /// False when the keyboard is in a floating window -- the minibuffer, a
    /// completion strip -- so that no tiled window is the one being typed in.
    pub tiled: bool,
}

/// A window: the buffer it shows, how far it is scrolled, and what the last
/// frame composed for it, which the commands and the mouse read between
/// frames.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: WindowId,

    /// The buffer it shows, by name.
    pub buffer_name: String,

    /// The first column and the first line it shows.
    pub scroll_x: usize,
    pub scroll_y: usize,

    /// How many rows of text it had when the last frame was composed, or 0
    /// if it has never been on screen: what a screenful is to a scrolling
    /// command.
    pub text_height: usize,

    /// The rectangle it was last drawn in, status line included, or a
    /// zero-sized one if it has never been on screen: what a click is tested
    /// against.
    pub rect: Rect,

    /// Whether it draws a status line along its bottom row: all windows but
    /// a strip, a few rows opened to be read and dismissed.
    pub show_mode_line: bool,

    /// Where point was when its view last followed point, given back to the
    /// buffer when it is focused again; `None` until it has been composed,
    /// as a window just made by a split.
    pub point: Option<usize>,

    /// How many columns it gave to its gutter when the last frame was
    /// composed, zero for none: what a click's column is counted from.
    pub gutter_width: usize,
}

impl Window {
    /// Shows BUFFER_NAME in this window, from its top, forgetting the scroll
    /// and the point that belonged to the buffer it showed.
    pub fn show(&mut self, buffer_name: &str) {
        buffer_name.clone_into(&mut self.buffer_name);
        self.scroll_x = 0;
        self.scroll_y = 0;
        self.point = None;
    }

    /// Returns window ID showing BUFFER_NAME, from its top, with a status
    /// line.
    pub fn new(id: impl Into<WindowId>, buffer_name: &str) -> Self {
        Self {
            id: id.into(),
            buffer_name: buffer_name.to_string(),
            scroll_x: 0,
            scroll_y: 0,
            text_height: 0,
            rect: Rect::default(),
            show_mode_line: true,
            point: None,
            gutter_width: 0,
        }
    }
}

/// How a split shares its space between its two children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Division {
    /// The first child takes this fraction of the space, the second the rest.
    Ratio(f32),

    /// The first child is this many rows, stacked, or columns, side by side,
    /// and keeps them when the frame is resized: the window split keeps its
    /// size, as with Emacs's `split-window-right SIZE`.
    FirstFixed(usize),

    /// The second child is this many rows or columns, and keeps them: the
    /// window added keeps its size, as a strip does.
    SecondFixed(usize),
}

impl Division {
    /// Returns how much of TOTAL the first child gets.
    fn first_share(&self, total: usize) -> usize {
        match self {
            Division::Ratio(ratio) => ((total as f32) * ratio).round() as usize,
            Division::FirstFixed(size) => (*size).min(total.saturating_sub(1)).max(total.min(1)),
            Division::SecondFixed(size) => total.saturating_sub(*size).max(total.min(1)),
        }
    }
}

/// The tiled windows, as a tree: a window, or a split of a rectangle into two
/// trees.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutNode {
    Leaf(Window),
    Split {
        orientation: Orientation,
        /// How the space is shared; `left` is the first child, above or to
        /// the left.
        division: Division,
        left: Box<LayoutNode>,
        right: Box<LayoutNode>,
    },
}

/// A window drawn over the tiled ones, in a rectangle of its own: a prompt, a
/// strip, a popup.
#[derive(Clone, Debug, PartialEq)]
pub struct FloatingWindow {
    pub window: Window,
    pub rect: Rect,
    pub has_border: bool,
    /// What its border is labelled with, the prompt for the minibuffer.
    pub title: Option<String>,
    /// The window focused when it opened, which has the focus back when it
    /// closes, another floating window included.
    pub previous_focused_window_id: WindowId,
}

/// The fewest rows a drag leaves a stacked window: a row of text and its
/// status line. A split of a small frame may still make one smaller.
pub const MIN_DRAGGED_HEIGHT: usize = 2;
/// The fewest columns a drag leaves a window side by side.
pub const MIN_DRAGGED_WIDTH: usize = 4;

/// Which half of a split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The upper one when stacked, the left one when side by side.
    First,
    /// The lower one, or the right one.
    Second,
}

/// The way from the root of a layout to one of its splits, which have no ids:
/// valid as long as the tree keeps its shape, as it does for a drag.
pub type SplitPath = Vec<Side>;

/// Returns how a split ORIENTATION, DIVISION divides RECT: a rectangle for
/// each child, and the rule between them when there is room for one. The one
/// arithmetic composition and the pointer both use.
pub fn split_rects(
    orientation: &Orientation,
    division: &Division,
    rect: Rect,
) -> (Rect, Rect, Option<Rect>) {
    match orientation {
        Orientation::Horizontal => {
            let first_height = division.first_share(rect.height);
            (
                Rect {
                    height: first_height,
                    ..rect
                },
                Rect {
                    y: rect.y + first_height as isize,
                    height: rect.height.saturating_sub(first_height),
                    ..rect
                },
                // Stacked windows are divided by the upper one's status line:
                // no rule.
                None,
            )
        }
        Orientation::Vertical => {
            let divided = rect.width >= MIN_WIDTH_FOR_SEPARATOR;
            let usable = rect.width.saturating_sub(divided as usize);
            let first_width = division.first_share(usable);
            (
                Rect {
                    width: first_width,
                    ..rect
                },
                Rect {
                    x: rect.x + first_width as isize + divided as isize,
                    width: usable.saturating_sub(first_width),
                    ..rect
                },
                divided.then_some(Rect {
                    x: rect.x + first_width as isize,
                    y: rect.y,
                    width: 1,
                    height: rect.height,
                }),
            )
        }
    }
}

impl LayoutNode {
    /// Calls VISIT on every window of this tree, in layout order.
    pub fn each_window_mut(&mut self, visit: &mut impl FnMut(&mut Window)) {
        match self {
            LayoutNode::Leaf(window) => visit(window),
            LayoutNode::Split { left, right, .. } => {
                left.each_window_mut(visit);
                right.each_window_mut(visit);
            }
        }
    }

    /// Returns window ID, if it is in this tree.
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        match self {
            LayoutNode::Leaf(window) => (window.id == id).then_some(window),
            LayoutNode::Split { left, right, .. } => left.window(id).or_else(|| right.window(id)),
        }
    }

    /// Returns window ID to change, if it is in this tree.
    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        match self {
            LayoutNode::Leaf(window) => {
                if window.id == id {
                    Some(window)
                } else {
                    None
                }
            }
            LayoutNode::Split { left, right, .. } => {
                left.window_mut(id).or_else(|| right.window_mut(id))
            }
        }
    }

    /// Returns the ids of every window, left to right and top to bottom: the
    /// order `other-window` goes round in.
    pub fn window_ids(&self) -> Vec<WindowId> {
        let mut ids = Vec::new();
        self.collect_window_ids(&mut ids);
        ids
    }

    fn collect_window_ids(&self, out: &mut Vec<WindowId>) {
        match self {
            LayoutNode::Leaf(window) => out.push(window.id),
            LayoutNode::Split { left, right, .. } => {
                left.collect_window_ids(out);
                right.collect_window_ids(out);
            }
        }
    }

    /// Makes window ID a split ORIENTATION, DIVISION of itself, first, and
    /// NEW_WINDOW, and returns true; false when no window has that id. The
    /// tree has no sizes, so no window is too small to split.
    pub fn split_window(
        &mut self,
        id: WindowId,
        orientation: Orientation,
        new_window: Window,
        division: Division,
    ) -> bool {
        match self {
            LayoutNode::Leaf(window) if window.id == id => {
                let existing = std::mem::replace(window, placeholder_window());
                *self = LayoutNode::Split {
                    orientation,
                    division,
                    left: Box::new(LayoutNode::Leaf(existing)),
                    right: Box::new(LayoutNode::Leaf(new_window)),
                };
                true
            }
            LayoutNode::Leaf(_) => false,
            LayoutNode::Split { left, right, .. } => {
                left.split_window(id, orientation, new_window.clone(), division)
                    || right.split_window(id, orientation, new_window, division)
            }
        }
    }

    /// Removes window ID, its sibling taking the place of their split, and
    /// returns true; false when no window has that id, or when it is the only
    /// one.
    pub fn remove_window(&mut self, id: WindowId) -> bool {
        let LayoutNode::Split { left, right, .. } = self else {
            // The only window: it stays.
            return false;
        };
        let is_target = |node: &LayoutNode| matches!(node, LayoutNode::Leaf(w) if w.id == id);

        if is_target(left) || is_target(right) {
            // The sibling takes the split's place, and its space.
            let survivor = if is_target(left) { right } else { left };
            *self = std::mem::replace(&mut **survivor, LayoutNode::Leaf(placeholder_window()));
            return true;
        }
        left.remove_window(id) || right.remove_window(id)
    }

    /// Makes window ID the whole tree, and returns true; false when no window
    /// has that id.
    pub fn keep_only(&mut self, id: impl Into<WindowId>) -> bool {
        let Some(window) = self.window(id.into()).cloned() else {
            return false;
        };
        *self = LayoutNode::Leaf(window);
        true
    }

    /// Returns the path to the split whose rule is under (X, Y), in a tree
    /// drawn in RECT, and the split's orientation. Worked out with
    /// [`split_rects`], since a rule is stored nowhere.
    pub fn separator_at(&self, rect: Rect, x: isize, y: isize) -> Option<(SplitPath, Orientation)> {
        let LayoutNode::Split {
            orientation,
            division,
            left,
            right,
        } = self
        else {
            return None;
        };
        let (first, second, rule) = split_rects(orientation, division, rect);
        if rule.is_some_and(|rule| rule.contains(x, y)) {
            return Some((Vec::new(), *orientation));
        }
        let (child, side, child_rect) = if first.contains(x, y) {
            (left, Side::First, first)
        } else if second.contains(x, y) {
            (right, Side::Second, second)
        } else {
            return None;
        };
        child.separator_at(child_rect, x, y).map(|(mut path, at)| {
            path.insert(0, side);
            (path, at)
        })
    }

    /// Returns the path from here to window ID.
    pub fn path_to_window(&self, id: WindowId) -> Option<SplitPath> {
        match self {
            LayoutNode::Leaf(win) => (win.id == id).then(Vec::new),
            LayoutNode::Split { left, right, .. } => left
                .path_to_window(id)
                .map(|mut path| {
                    path.insert(0, Side::First);
                    path
                })
                .or_else(|| {
                    right.path_to_window(id).map(|mut path| {
                        path.insert(0, Side::Second);
                        path
                    })
                }),
        }
    }

    /// Returns which way the split at PATH divides, if there is one there.
    pub fn orientation_at(&self, path: &[Side]) -> Option<Orientation> {
        let LayoutNode::Split {
            orientation,
            left,
            right,
            ..
        } = self
        else {
            return None;
        };
        match path.split_first() {
            None => Some(*orientation),
            Some((Side::First, rest)) => left.orientation_at(rest),
            Some((Side::Second, rest)) => right.orientation_at(rest),
        }
    }

    /// Returns the path to the split the status line of window ID divides:
    /// the nearest stacked split it is in the upper half of. None for the
    /// bottom window, above the echo area.
    pub fn mode_line_divider(&self, id: WindowId) -> Option<SplitPath> {
        let path = self.path_to_window(id)?;
        (0..path.len()).rev().find_map(|cut| {
            (path[cut] == Side::First
                && self.orientation_at(&path[..cut]) == Some(Orientation::Horizontal))
            .then(|| path[..cut].to_vec())
        })
    }

    /// Moves the boundary of the split at PATH by DELTA cells, towards the
    /// second child when positive, and returns whether it moved. RECT is the
    /// rectangle this tree was last drawn in.
    pub fn resize_split(&mut self, path: &[Side], rect: Rect, delta: isize) -> bool {
        let LayoutNode::Split {
            orientation,
            division,
            left,
            right,
        } = self
        else {
            return false;
        };
        if let Some((side, rest)) = path.split_first() {
            let (first, second, _) = split_rects(orientation, division, rect);
            return match side {
                Side::First => left.resize_split(rest, first, delta),
                Side::Second => right.resize_split(rest, second, delta),
            };
        }
        let (usable, floor) = match orientation {
            Orientation::Horizontal => (rect.height, MIN_DRAGGED_HEIGHT),
            Orientation::Vertical => (
                rect.width
                    .saturating_sub((rect.width >= MIN_WIDTH_FOR_SEPARATOR) as usize),
                MIN_DRAGGED_WIDTH,
            ),
        };
        // Too small for two windows a drag may leave: left as it is.
        if usable < floor * 2 {
            return false;
        }
        let current = division.first_share(usable) as isize;
        let wanted = (current + delta).clamp(floor as isize, (usable - floor) as isize);
        if wanted == current {
            return false;
        }
        // The kind of division is kept: a strip dragged still wants a size,
        // not a share.
        *division = match division {
            Division::Ratio(_) => Division::Ratio(wanted as f32 / usable as f32),
            Division::FirstFixed(_) => Division::FirstFixed(wanted as usize),
            Division::SecondFixed(_) => Division::SecondFixed(usable - wanted as usize),
        };
        true
    }

    /// Returns the window whose last drawn rectangle, [`Window::rect`],
    /// contains (X, Y).
    pub fn window_at(&self, x: isize, y: isize) -> Option<&Window> {
        match self {
            LayoutNode::Leaf(win) => win.rect.contains(x, y).then_some(win),
            LayoutNode::Split { left, right, .. } => {
                left.window_at(x, y).or_else(|| right.window_at(x, y))
            }
        }
    }

    /// Returns whether window ID is in this tree.
    pub fn contains_window(&self, id: WindowId) -> bool {
        match self {
            LayoutNode::Leaf(win) => win.id == id,
            LayoutNode::Split { left, right, .. } => {
                left.contains_window(id) || right.contains_window(id)
            }
        }
    }
}

/// Returns a window nothing reads, left by `std::mem::replace` in place of a
/// subtree being moved, and gone before the move returns.
fn placeholder_window() -> Window {
    Window::new(usize::MAX, "")
}

/// The narrowest a rectangle can be and still give a column to a rule: one
/// for each window and one for the rule.
const MIN_WIDTH_FOR_SEPARATOR: usize = 3;
