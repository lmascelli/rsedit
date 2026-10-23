//! Every window the frame has -- the tiled tree, the floats over it, which one
//! has focus, the next id -- and what the mouse is in the middle of doing: one
//! fact in several pieces, and the only code that changes it. A window names a
//! buffer and knows nothing else of it: what an operation needs from the
//! buffer is passed in as a number.
//!
//! Design: `doc/architettura-editor.typ, <compartimenti>`.
use crate::ui::{
    Division, FloatingWindow, LayoutNode, Orientation, Rect, Side, SplitPath, Window, WindowId,
};
use std::time::{Duration, Instant};

/// How often a drag held outside its window scrolls it.
///
/// Fast enough to feel continuous, slow enough that a line is still a unit you
/// can stop on.
pub const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(60);

/// How long after a click a second one in the same cell counts as a double: a
/// little under Emacs' `double-click-time`.
pub const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);

/// Whether a press is on its own or the second of a pair: two is as far as
/// counting goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickCount {
    Single,
    Double,
}

/// What the pointer is over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Inside a tiled window's text. LINE is a buffer line, scroll included.
    /// COLUMN is a *screen* column, scroll included, which `editor/mouse.rs`
    /// turns into a buffer column with
    /// [`Row::to_buffer`](crate::ui::layout::Row::to_buffer): a row showing
    /// virtual text is wider than its line, and the buffer is not this
    /// compartment's. COLUMN may lie past the end of the row; the command
    /// clamps it against the buffer.
    Text {
        window: WindowId,
        line: usize,
        column: usize,
    },
    /// A tiled window's status line, named by its window: only a drag needs
    /// the split it divides, and asks the tree then.
    ModeLine { window: WindowId },
    /// The rule drawn between two windows side by side, and the split it
    /// divides.
    Separator {
        path: SplitPath,
        orientation: Orientation,
    },
    /// A floating window -- a prompt, a completion strip -- reported so that a
    /// click on it is swallowed rather than falling through to the window
    /// drawn under it.
    Floating { window: WindowId },
}

/// What a held mouse button is in the middle of doing: where a drag started
/// decides what the events after it mean, wherever the pointer has gone since.
#[derive(Clone, Debug)]
pub enum MouseDrag {
    /// Extending a selection inside one window. `at` is where the pointer
    /// was last seen, in frame cells: a pointer held still outside the window
    /// sends no events, and the drag keeps scrolling from there.
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

/// What a scroll did, and what it leaves the caller to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scrolled {
    /// The view was already as far as it goes.
    No,
    /// It moved. `drag_point_to` names a line when point would otherwise have
    /// been left outside the new view -- moving it is the caller's job, since
    /// that is a change to a buffer and nothing here touches one.
    Yes { drag_point_to: Option<usize> },
}

/// What [`Windows::remove`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRemoved {
    /// Nothing: there was no such window, or it was the last one -- a frame
    /// with no windows has nowhere to draw a cursor.
    No,
    /// Gone, and focus was somewhere else all along.
    Yes,
    /// Gone, and it held focus. Focus has moved to the named survivor here;
    /// making its buffer current is the caller's half of that.
    Refocus(WindowId),
}

/// Two lines of overlap, as Emacs keeps: a screenful with nothing in common
/// with the last one gives the reader nothing to place themselves by.
const NEXT_SCREEN_CONTEXT_LINES: usize = 2;

/// The window tree, the floats, focus, the next id, and the mouse's state.
pub struct Windows {
    root: LayoutNode,
    floating: Vec<FloatingWindow>,
    focused: WindowId,
    /// The id the next window will get.
    next_id: usize,
    /// What a held button is doing, if anything.
    drag: Option<MouseDrag>,
    /// When and where the last left button went down, for telling the second
    /// click of a pair: a terminal reports presses, not double clicks.
    /// Cleared once a pair is complete, so that three presses are a pair and
    /// then a single.
    last_click: Option<(Instant, isize, isize)>,
}

impl Default for Windows {
    fn default() -> Self {
        Self {
            root: LayoutNode::Leaf(Window::new(WindowId(0), "*scratch*")),
            floating: Vec::new(),
            focused: WindowId(0),
            next_id: 1,
            drag: None,
            last_click: None,
        }
    }
}

/// Returns which column of WIN's text the screen column X is over: zero in
/// the gutter, where a click means the start of the line, and to the left of
/// the window. One function for the click and the drag, which must agree.
fn text_column(win: &Window, x: isize) -> usize {
    (x - win.rect.x - win.gutter_width as isize).max(0) as usize
}

impl Windows {
    // ------------------------------------------------------------------
    // The pieces, for the operations that still live on the facade
    // ------------------------------------------------------------------

    /// Returns the tree of tiled windows.
    pub fn root(&self) -> &LayoutNode {
        &self.root
    }

    /// Returns the tree of tiled windows, to change.
    pub fn root_mut(&mut self) -> &mut LayoutNode {
        &mut self.root
    }

    /// Returns the floating windows, oldest first.
    pub fn floating(&self) -> &[FloatingWindow] {
        &self.floating
    }

    /// Returns the floating windows, to change.
    pub fn floating_mut(&mut self) -> &mut Vec<FloatingWindow> {
        &mut self.floating
    }

    /// Returns the window with focus.
    pub fn focused(&self) -> WindowId {
        self.focused
    }

    /// Gives window ID focus.
    pub fn set_focused(&mut self, id: WindowId) {
        self.focused = id;
    }

    /// Takes the id the next window gets.
    pub fn next_id(&mut self) -> WindowId {
        let id = self.next_id;
        self.next_id += 1;
        WindowId(id)
    }

    /// Returns what a held button is doing.
    pub fn drag(&self) -> Option<MouseDrag> {
        self.drag.clone()
    }

    /// Records what a held button is doing.
    pub fn set_drag(&mut self, drag: Option<MouseDrag>) {
        self.drag = drag;
    }

    /// Takes what a held button was doing, leaving nothing.
    pub fn take_drag(&mut self) -> Option<MouseDrag> {
        self.drag.take()
    }

    /// Points every window showing OLD at NEW, tiled and floating: a window
    /// holds the name of what it shows, and any of them may show the buffer
    /// renamed.
    pub fn rename_buffer(&mut self, old: &str, new: &str) {
        self.root.each_window_mut(&mut |window| {
            if window.buffer_name == old {
                new.clone_into(&mut window.buffer_name);
            }
        });
        for float in self.floating.iter_mut() {
            if float.window.buffer_name == old {
                new.clone_into(&mut float.window.buffer_name);
            }
        }
    }

    /// Records a left button going down at (X, Y), and returns whether it is
    /// the second of a pair: in the same cell, within `DOUBLE_CLICK_INTERVAL`.
    /// The same cell and not a neighbour, since a click just beside the last
    /// one moves the cursor along.
    pub fn register_click(&mut self, x: isize, y: isize) -> ClickCount {
        let now = Instant::now();
        let doubled = self.last_click.is_some_and(|(when, last_x, last_y)| {
            last_x == x && last_y == y && now.duration_since(when) <= DOUBLE_CLICK_INTERVAL
        });
        // Forgotten on a double so that a third press starts a fresh pair
        // rather than completing a second one with the press before it.
        self.last_click = if doubled { None } else { Some((now, x, y)) };
        if doubled {
            ClickCount::Double
        } else {
            ClickCount::Single
        }
    }

    // ------------------------------------------------------------------
    // Asking
    // ------------------------------------------------------------------

    /// Returns how many tiled windows the frame holds.
    pub fn count(&self) -> usize {
        self.root.window_ids().len()
    }

    /// Returns the buffer the focused tiled window shows, which is not always
    /// the current one: a floating prompt takes the current buffer, not a
    /// tiled window.
    pub fn focused_buffer(&self) -> Option<String> {
        self.root
            .window(self.focused)
            .map(|window| window.buffer_name.clone())
    }

    /// Returns what window ID shows, tiled or floating: focusing a prompt,
    /// which is a float, makes its buffer current like any other window's.
    pub fn buffer_of(&self, id: WindowId) -> Option<String> {
        if let Some(window) = self.root.window(id) {
            return Some(window.buffer_name.clone());
        }
        self.floating
            .iter()
            .find(|float| float.window.id == id)
            .map(|float| float.window.buffer_name.clone())
    }

    /// Returns the rectangle the tiled windows were last laid out in, rebuilt
    /// from their own: the caller of `compute_tiled_views` decides it, and
    /// the rules between windows, which belong to splits, need it.
    pub fn bounds(&self) -> Rect {
        // Every tiled window sits inside it, so its bounds are theirs put
        // together -- which is what the last frame decided, whatever the
        // terminal has done since.
        let mut bounds: Option<Rect> = None;
        for rect in self
            .root
            .window_ids()
            .into_iter()
            .filter_map(|id| self.root.window(id).map(|win| win.rect))
        {
            bounds = Some(match bounds {
                None => rect,
                Some(so_far) => {
                    let x = so_far.x.min(rect.x);
                    let y = so_far.y.min(rect.y);
                    Rect {
                        x,
                        y,
                        width: ((so_far.x + so_far.width as isize)
                            .max(rect.x + rect.width as isize)
                            - x) as usize,
                        height: ((so_far.y + so_far.height as isize)
                            .max(rect.y + rect.height as isize)
                            - y) as usize,
                    }
                }
            });
        }
        bounds.unwrap_or_default()
    }

    /// Returns what the pointer at (X, Y) is over, in cells of the last frame;
    /// `None` over the echo area, or a gap no window claims.
    pub fn hit_test(&self, x: isize, y: isize) -> Option<Hit> {
        // Floats first and in reverse, because later ones paint over earlier
        // ones and the last drawn is the one the pointer is really on.
        if let Some(float) = self
            .floating
            .iter()
            .rev()
            .find(|float| float.rect.contains(x, y))
        {
            return Some(Hit::Floating {
                window: float.window.id,
            });
        }
        // A rule occupies a column no window claims, so this can be asked
        // before them without stealing anything.
        if let Some((path, orientation)) = self.root.separator_at(self.bounds(), x, y) {
            return Some(Hit::Separator { path, orientation });
        }
        let win = self.root.window_at(x, y)?;
        let row = (y - win.rect.y) as usize;
        if row >= win.text_height {
            return Some(Hit::ModeLine { window: win.id });
        }
        Some(Hit::Text {
            window: win.id,
            line: win.scroll_y + row,
            column: win.scroll_x + text_column(win, x),
        })
    }

    /// Returns the line and the *screen* column of WINDOW that the cell
    /// (X, Y) is on, as `Hit::Text` gives them, clamped to what the window
    /// shows: this answers a drag, and a drag past an edge selects to the
    /// edge. Scrolling after the pointer is [`Windows::drag_scroll_step`]'s.
    pub fn position_in(&self, window: WindowId, x: isize, y: isize) -> Option<(usize, usize)> {
        let win = self.root.window(window)?;
        let rows = win.text_height.max(1);
        let row = (y - win.rect.y).clamp(0, rows as isize - 1) as usize;
        Some((win.scroll_y + row, win.scroll_x + text_column(win, x)))
    }

    /// Returns which way a drag that has left WINDOW wants the view to move,
    /// if it has left it: vertically only, since off a side clamping already
    /// selects to the ends of the lines.
    pub fn drag_scroll_step(&self, window: WindowId, y: isize) -> Option<isize> {
        let win = self.root.window(window)?;
        let top = win.rect.y;
        let bottom = top + win.text_height.max(1) as isize;
        // Below the text: the status line and anything under it.
        if y >= bottom {
            return Some(1);
        }
        if y < top {
            return Some(-1);
        }
        // The top row of a window at the top of the frame counts as above it:
        // a terminal cannot report row -1, so a drag off the top arrives as
        // row 0. At the start of the buffer, `scroll_by` has nothing to do.
        (y == top && top == 0).then_some(-1)
    }

    /// Returns how long until a drag that has left its window scrolls again,
    /// or `None` when none is waiting to: a pointer held still sends nothing,
    /// and the selection has to keep going.
    pub fn drag_scroll_in(&self) -> Option<Duration> {
        let MouseDrag::Text { window, at: (_, y) } = self.drag()? else {
            return None;
        };
        self.drag_scroll_step(window, y)
            .map(|_| DRAG_SCROLL_INTERVAL)
    }

    // ------------------------------------------------------------------
    // Changing
    // ------------------------------------------------------------------

    /// Splits the focused window, the halves shared as DIVISION says, and
    /// returns the new window's id. Focus stays where it was, as in Emacs.
    pub fn split_focused(
        &mut self,
        orientation: Orientation,
        division: Division,
    ) -> Option<WindowId> {
        let focused = self.focused;
        // The new window shows the same buffer, scrolled the same way: one
        // view becomes two.
        let existing = self.root.window(focused)?.clone();
        let new_id = self.next_id();
        let new_window = Window {
            id: new_id,
            ..existing
        };
        self.root
            .split_window(focused, orientation, new_window, division)
            .then_some(new_id)
    }

    /// Opens a window of exactly HEIGHT rows across the bottom of the whole
    /// frame, showing BUFFER, and returns its id. Focus does not move: a strip
    /// is shown while something else is typed into, as the completions under
    /// a prompt are.
    pub fn open_bottom(&mut self, buffer: &str, height: usize) -> WindowId {
        let id = self.next_id();
        let window = Window {
            // No status line: HEIGHT rows of what the caller wants shown.
            show_mode_line: false,
            ..Window::new(id, buffer)
        };
        let existing = std::mem::replace(&mut self.root, LayoutNode::Leaf(window.clone()));
        self.root = LayoutNode::Split {
            orientation: Orientation::Horizontal,
            division: Division::SecondFixed(height),
            left: Box::new(existing),
            right: Box::new(LayoutNode::Leaf(window)),
        };
        id
    }

    /// Closes every tiled window but the focused one; false, changing
    /// nothing, when focus is not on a tiled window.
    pub fn delete_others(&mut self) -> bool {
        self.root.keep_only(self.focused)
    }

    /// Removes window ID, and says what focus must now do. Focus moves only
    /// when ID had it: closing a strip must not change the current buffer
    /// under whatever opened the strip.
    pub fn remove(&mut self, id: WindowId) -> WindowRemoved {
        if !self.root.remove_window(id) {
            return WindowRemoved::No;
        }
        match (self.focused == id)
            .then(|| self.root.window_ids().first().copied())
            .flatten()
        {
            // Focus names a window that is gone, so it moves now; making the
            // survivor's buffer current is the caller's.
            Some(survivor) => {
                self.focused = survivor;
                WindowRemoved::Refocus(survivor)
            }
            None => WindowRemoved::Yes,
        }
    }

    /// Moves focus COUNT windows on, round [`Windows::all_ids`] and backwards
    /// for a negative COUNT, and returns where it landed, for the caller to
    /// make its buffer current.
    pub fn focus_other(&mut self, count: isize) -> Option<WindowId> {
        let ids = self.all_ids();
        if ids.len() < 2 {
            return None;
        }
        // Every removal moves focus, so it is always in the list: starting
        // from the beginning is only a recovery.
        let here = ids.iter().position(|id| *id == self.focused).unwrap_or(0) as isize;
        // `rem_euclid` rather than `%`, so a negative count wraps round to the
        // end instead of producing a negative index.
        let there = (here + count).rem_euclid(ids.len() as isize) as usize;
        let landed = ids[there];
        self.focused = landed;
        Some(landed)
    }

    /// Returns every window: the tiled ones in layout order, then the floats,
    /// oldest first. The floats are in it so that focus can get back to one it
    /// has left; [`Windows::count`] counts only the tiled windows, which are
    /// what divide the frame.
    pub fn all_ids(&self) -> Vec<WindowId> {
        let mut ids = self.root.window_ids();
        ids.extend(self.floating.iter().map(|float| float.window.id));
        ids
    }

    /// Points the focused window at NAME; making it current is the caller's.
    pub fn show_in_focused(&mut self, name: &str) {
        if let Some(window) = self.root.window_mut(self.focused) {
            window.show(name);
        }
    }

    /// Moves the focused window's view by AMOUNT screenfuls, two lines
    /// overlapping, and says where point must go if the view has left it: the
    /// view moves and point follows, as with `C-v` in Emacs.
    pub fn scroll_focused(
        &mut self,
        amount: isize,
        line_count: usize,
        point_line: usize,
    ) -> Scrolled {
        let Some(win) = self.root.window_mut(self.focused) else {
            return Scrolled::No;
        };
        // A window never drawn has no height yet: one row keeps the arithmetic
        // sane.
        let height = win.text_height.max(1);
        // The floor is for a window of one or two rows, which the overlap
        // would otherwise fill.
        let step = height.saturating_sub(NEXT_SCREEN_CONTEXT_LINES).max(1) as isize;
        // Far enough that the last line sits on the bottom row, and no
        // further: past that is a screen of nothing.
        let furthest = line_count.saturating_sub(height) as isize;
        let target = (win.scroll_y as isize + amount * step).clamp(0, furthest);
        if target == win.scroll_y as isize {
            return Scrolled::No;
        }
        win.scroll_y = target as usize;
        let top = target as usize;
        let bottom = top + height - 1;
        // Point moves only if it fell outside; inside, it keeps its line.
        let last_line = line_count.saturating_sub(1);
        let drag_point_to = if point_line < top {
            Some(top)
        } else if point_line > bottom {
            Some(bottom.min(last_line))
        } else {
            None
        };
        Scrolled::Yes { drag_point_to }
    }

    /// Scrolls the focused window so that point's line sits WHERE_TO of the
    /// way down it: 0.0 the top row, 0.5 the middle, 1.0 the bottom. Point
    /// does not move; the text does. Returns false when nothing changed.
    pub fn recenter_focused(
        &mut self,
        where_to: f64,
        line_count: usize,
        point_line: usize,
    ) -> bool {
        let Some(win) = self.root.window_mut(self.focused) else {
            return false;
        };
        let height = win.text_height.max(1);
        let above = ((height - 1) as f64 * where_to.clamp(0.0, 1.0)).round() as usize;
        // Clamped at the top only: near the end of a file, `C-l` still
        // centres.
        let target = point_line
            .saturating_sub(above)
            .min(line_count.saturating_sub(1));
        if target == win.scroll_y {
            return false;
        }
        win.scroll_y = target;
        true
    }

    /// Scrolls WINDOW by LINES, towards the end when positive, without moving
    /// focus. In the focused window point follows the view, as in
    /// [`Windows::scroll_focused`]; in another it stays, since point is the
    /// buffer's and another window may be showing the buffer.
    pub fn scroll_by(
        &mut self,
        window: WindowId,
        lines: isize,
        line_count: usize,
        point_line: usize,
    ) -> Scrolled {
        let focused = self.focused == window;
        let Some(win) = self.root.window_mut(window) else {
            return Scrolled::No;
        };
        let height = win.text_height.max(1);
        // The last line may reach the top row: further than `scroll_focused`
        // goes, since a wheel looks at the end of a file where a screenful key
        // reads through it.
        let furthest = line_count.saturating_sub(1) as isize;
        let target = (win.scroll_y as isize + lines).clamp(0, furthest);
        if target == win.scroll_y as isize {
            return Scrolled::No;
        }
        win.scroll_y = target as usize;
        if !focused {
            return Scrolled::Yes {
                drag_point_to: None,
            };
        }
        let top = target as usize;
        let bottom = top + height - 1;
        let last_line = line_count.saturating_sub(1);
        let drag_point_to = if point_line < top {
            Some(top)
        } else if point_line > bottom {
            Some(bottom.min(last_line))
        } else {
            None
        };
        Scrolled::Yes { drag_point_to }
    }

    /// Moves the boundary of the split at PATH by DELTA cells, and returns
    /// false when it could not move.
    pub fn resize_split(&mut self, path: &[Side], delta: isize) -> bool {
        let rect = self.bounds();
        self.root.resize_split(path, rect, delta)
    }
}
