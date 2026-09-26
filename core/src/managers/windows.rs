//! Every window the frame has, and the only code allowed to change them.
//!
//! # Why this is a compartment and not five fields
//!
//! The layout, the floating windows, which one has focus, what the next id
//! will be and what a held mouse button is doing are one fact in five pieces.
//! Held as five locks they can be observed mid-change -- focus naming a window
//! the layout has already removed -- and every operation touching two of them
//! had to take them in an order recorded somewhere else. Held as one, that
//! order is not a rule anybody can break: there is nothing left to order.
//!
//! Three things in the previous arrangement stop being possible. `hit_test`
//! took the floating windows and then the layout, the reverse of the canonical
//! order, and needed a comment explaining why that was safe. Splitting read
//! the focused id, then the next id, then took the layout -- three
//! acquisitions with gaps, across which focus could in principle change. And
//! the bounds of the tiled area had to be a free function rather than a
//! method, purely so a caller already holding the layout could ask without
//! taking a second read lock on it.
//!
//! # What is deliberately not here
//!
//! Buffers. A window names a buffer and that is all it knows: nothing in this
//! file reads a buffer's text, its point or its length. Where a window
//! operation needs one of those -- scrolling needs a line count, recentring
//! needs the line point is on -- the *number* is passed in. That keeps the two
//! compartments' locks from ever being held together by anything here, and it
//! is what makes these operations testable without an editor.
use crate::ui::{
    Division, FloatingWindow, LayoutNode, Orientation, Rect, Side, SplitPath, Window, WindowId,
};
use std::time::Duration;

/// How often a drag held outside its window scrolls it.
///
/// Fast enough to feel continuous, slow enough that a line is still a unit you
/// can stop on.
pub const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(60);

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

/// What a held mouse button is in the middle of doing.
///
/// # Why this is remembered at all
///
/// A drag is the one mouse gesture that is not a single event. Where it
/// started decides what the events after it mean, and the pointer may by then
/// be somewhere that would answer differently -- over another window, or off
/// the frame entirely. Reading the position afresh on each event would make a
/// selection jump buffers halfway through, and a window resize stop the moment
/// the pointer overshot the rule it was dragging.
#[derive(Clone, Debug)]
pub enum MouseDrag {
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

/// What a scroll did, and what it leaves the caller to do.
///
/// Two variants rather than `Option<Option<usize>>`, which says the same thing
/// and reads wrong six months later.
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
pub enum Removed {
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

pub struct Windows {
    root: LayoutNode,
    floating: Vec<FloatingWindow>,
    focused: WindowId,
    /// The id the next window will get.
    ///
    /// A plain `usize` rather than an atomic: it lives under the same lock as
    /// everything else here, so the atomicity it used to need is the lock's
    /// job now.
    next_id: usize,
    drag: Option<MouseDrag>,
}

impl Default for Windows {
    fn default() -> Self {
        Self {
            root: LayoutNode::Leaf(Window::new(WindowId(0), "*scratch*")),
            floating: Vec::new(),
            focused: WindowId(0),
            next_id: 1,
            drag: None,
        }
    }
}

impl Windows {
    // ------------------------------------------------------------------
    // The pieces, for the operations that still live on the facade
    // ------------------------------------------------------------------

    pub fn root(&self) -> &LayoutNode {
        &self.root
    }

    pub fn root_mut(&mut self) -> &mut LayoutNode {
        &mut self.root
    }

    pub fn floating(&self) -> &[FloatingWindow] {
        &self.floating
    }

    pub fn floating_mut(&mut self) -> &mut Vec<FloatingWindow> {
        &mut self.floating
    }

    pub fn focused(&self) -> WindowId {
        self.focused
    }

    pub fn set_focused(&mut self, id: WindowId) {
        self.focused = id;
    }

    /// The id the next window gets, taken.
    pub fn next_id(&mut self) -> WindowId {
        let id = self.next_id;
        self.next_id += 1;
        WindowId(id)
    }

    pub fn drag(&self) -> Option<MouseDrag> {
        self.drag.clone()
    }

    pub fn set_drag(&mut self, drag: Option<MouseDrag>) {
        self.drag = drag;
    }

    pub fn take_drag(&mut self) -> Option<MouseDrag> {
        self.drag.take()
    }

    // ------------------------------------------------------------------
    // Asking
    // ------------------------------------------------------------------

    /// How many tiled windows the frame holds.
    pub fn count(&self) -> usize {
        self.root.window_ids().len()
    }

    /// The buffer shown by the focused window, which is not always the current
    /// buffer: a floating prompt takes the current buffer without taking a
    /// tiled window's place.
    pub fn focused_buffer(&self) -> Option<String> {
        self.root
            .window(self.focused)
            .map(|window| window.buffer_name.clone())
    }

    /// What window ID is showing, whether it is tiled or floating.
    ///
    /// Both, because a minibuffer prompt is a floating window and giving it
    /// focus has to make its buffer current in exactly the same way -- that is
    /// what every prompt in the editor depends on.
    pub fn buffer_of(&self, id: WindowId) -> Option<String> {
        if let Some(window) = self.root.window(id) {
            return Some(window.buffer_name.clone());
        }
        self.floating
            .iter()
            .find(|float| float.window.id == id)
            .map(|float| float.window.buffer_name.clone())
    }

    /// The rectangle the tiled windows were last laid out in.
    ///
    /// Rebuilt from what they recorded rather than stored: it is the frame less
    /// the echo area, and the one place that decides it is the caller of
    /// `compute_tiled_views`. The separator walk needs it because a rule
    /// belongs to a *split* rather than to a window, so there is no window's
    /// own rect to start from.
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

    /// What the pointer at (X, Y) is over, in cells of the last frame.
    ///
    /// `None` when it is over nothing -- the echo area, or a gap no window
    /// claims.
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
            column: win.scroll_x + (x - win.rect.x) as usize,
        })
    }

    /// Where in WINDOW's buffer the cell (X, Y) is, clamped to what the window
    /// is showing.
    ///
    /// Clamped rather than refused because this answers a *drag*, and a drag
    /// that leaves the window is a perfectly ordinary way to select to its
    /// edge. Dragging beyond an edge selects to that edge and stops; it does
    /// not scroll the window after the pointer, which `drag_scroll_step` does.
    pub fn position_in(&self, window: WindowId, x: isize, y: isize) -> Option<(usize, usize)> {
        let win = self.root.window(window)?;
        let rows = win.text_height.max(1);
        let row = (y - win.rect.y).clamp(0, rows as isize - 1) as usize;
        let column = (x - win.rect.x).max(0) as usize;
        Some((win.scroll_y + row, win.scroll_x + column))
    }

    /// Which way a drag that has left its window wants the view to move, if it
    /// has left it at all.
    ///
    /// Vertically only. Dragging off the side of a window is a request to
    /// select to the end of the lines you are over, which clamping already
    /// gives; dragging off the top or bottom is a request for lines that are
    /// not on screen, which nothing but scrolling can answer.
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
        // The top row of a window flush with the top of the frame counts as
        // being above it, because there is no row above it to be on: a
        // terminal cannot report row -1, so a drag off the top of the screen
        // arrives clamped to row 0 and would otherwise read as "inside".
        //
        // It costs nothing when the view is already at the start of the
        // buffer, because then there is nothing to scroll to and `scroll_by`
        // says so. It only acts when there is text above, which is the only
        // time anybody drags there.
        (y == top && top == 0).then_some(-1)
    }

    /// How long until a drag that has left its window should scroll again, or
    /// `None` when none is waiting to.
    ///
    /// # Why this is a timer and not an event
    ///
    /// A pointer held still outside a window sends nothing at all, and that is
    /// exactly the position somebody selecting a long passage leaves it in.
    /// Scrolling only on movement would mean the selection stopped the moment
    /// they stopped jiggling the mouse, which reads as the editor having lost
    /// interest.
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

    /// Split the focused window, giving the two halves DIVISION, and return
    /// the new window's id.
    ///
    /// Focus stays where it was, as it does in Emacs: `C-x 2` then typing
    /// continues in the window you were already in.
    pub fn split_focused(
        &mut self,
        orientation: Orientation,
        division: Division,
    ) -> Option<WindowId> {
        let focused = self.focused;
        // The new window shows the same buffer, scrolled the same way, so a
        // split looks like what it is: one view becoming two of the same
        // thing rather than a jump somewhere else.
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
    pub fn open_bottom(&mut self, buffer: &str, height: usize) -> WindowId {
        let id = self.next_id();
        let window = Window {
            // No status line: the height asked for is the height of what the
            // caller wanted shown, and spending a row of it saying
            // "*Completions*" would make six mean five.
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

    /// Close every window but the focused one.
    pub fn delete_others(&mut self) -> bool {
        self.root.keep_only(self.focused)
    }

    /// Remove a window, and say what focus must now do about it.
    ///
    /// Focus moves only when the window that went *was* the focused one.
    /// Closing a strip must not move focus, which would change the current
    /// buffer out from under whatever asked for the strip in the first place.
    pub fn remove(&mut self, id: WindowId) -> Removed {
        if !self.root.remove_window(id) {
            return Removed::No;
        }
        match (self.focused == id)
            .then(|| self.root.window_ids().first().copied())
            .flatten()
        {
            // Focus is pointing at a window that no longer exists, so it has
            // to move before anything tries to draw a cursor in it. The move
            // is the caller's because it drags the current buffer along, and
            // buffers are not this compartment's to touch.
            Some(survivor) => {
                self.focused = survivor;
                Removed::Refocus(survivor)
            }
            None => Removed::Yes,
        }
    }

    /// Move focus COUNT windows on, wrapping round, and answer which window it
    /// landed on so the caller can make its buffer current.
    ///
    /// A negative count goes the other way, which is what lets one command
    /// serve `C-x o` and a reversed `C-x o` alike.
    pub fn focus_other(&mut self, count: isize) -> Option<WindowId> {
        let ids = self.root.window_ids();
        if ids.len() < 2 {
            return None;
        }
        let here = ids.iter().position(|id| *id == self.focused).unwrap_or(0) as isize;
        // `rem_euclid` rather than `%`, so a negative count wraps round to the
        // end instead of producing a negative index.
        let there = (here + count).rem_euclid(ids.len() as isize) as usize;
        let landed = ids[there];
        self.focused = landed;
        Some(landed)
    }

    /// Point the focused window at BUFFER. The caller makes it current.
    pub fn show_in_focused(&mut self, name: &str) {
        if let Some(window) = self.root.window_mut(self.focused) {
            window.show(name);
        }
    }

    /// Move the focused window's view by AMOUNT screenfuls.
    ///
    /// # Why point moves second, and elsewhere
    ///
    /// Scrolling and moving point are different things, and Emacs keeps them
    /// different: `C-v` moves the *view*, and point comes along only because it
    /// has to stay somewhere visible. Moving point first and letting the
    /// renderer's cursor-following do the scrolling would look similar and be
    /// wrong in the case that matters -- point would land at the window's edge
    /// rather than keeping its place on the screen.
    pub fn scroll_focused(
        &mut self,
        amount: isize,
        line_count: usize,
        point_line: usize,
    ) -> Scrolled {
        let Some(win) = self.root.window_mut(self.focused) else {
            return Scrolled::No;
        };
        // A window that has never been drawn has no height to scroll by: the
        // layout has not run, so nothing has worked one out. Treating it as one
        // row keeps every calculation below sane.
        let height = win.text_height.max(1);
        // The floor matters for a genuinely tiny window -- one or two rows --
        // where the overlap would otherwise be the whole of it and the key
        // would do nothing at all.
        let step = height.saturating_sub(NEXT_SCREEN_CONTEXT_LINES).max(1) as isize;
        // Far enough that the last line sits on the bottom row, and no
        // further. Past that the window fills with the blank space after the
        // end of the buffer -- a screen showing nothing at all, which the user
        // then has to scroll back out of by hand.
        let furthest = line_count.saturating_sub(height) as isize;
        let target = (win.scroll_y as isize + amount * step).clamp(0, furthest);
        if target == win.scroll_y as isize {
            return Scrolled::No;
        }
        win.scroll_y = target as usize;
        let top = target as usize;
        let bottom = top + height - 1;
        // Point only if it fell outside. Inside the new view it keeps the line
        // it was on, which is what makes two screenfuls of reading leave the
        // cursor where the eye left it.
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

    /// Scroll the focused window so that the line point is on sits `where_to`
    /// of the way down it: 0.0 the top row, 0.5 the middle, 1.0 the bottom.
    ///
    /// Point does not move. This is the opposite of `scroll_focused`, which
    /// moves the view and drags point along only when it would otherwise fall
    /// off the screen: here the cursor is the fixed thing and the text slides
    /// under it, which is what makes `C-l` a way of *looking* rather than a
    /// way of moving.
    ///
    /// False when nothing changed, so a caller can tell a no-op from a scroll.
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
        // Clamped at the top, and *not* at the bottom. Near the end of a file
        // there are not enough lines left to fill the window, and refusing to
        // scroll past that would make `C-l` do nothing for the last screenful
        // -- exactly where centring is most wanted.
        let target = point_line
            .saturating_sub(above)
            .min(line_count.saturating_sub(1));
        if target == win.scroll_y {
            return false;
        }
        win.scroll_y = target;
        true
    }

    /// Scroll WINDOW by LINES, towards the end of the buffer when positive.
    ///
    /// Focus does not move.
    ///
    /// # Why point comes along
    ///
    /// For the same reason it does in [`Windows::scroll_focused`], and it is
    /// the same rule deliberately: a view scrolled far enough that point is no
    /// longer in it leaves the cursor somewhere the reader cannot see, and the
    /// next arrow key snaps the view back to wherever that was. Two ways of
    /// scrolling that disagreed about this was the confusing part -- `C-v`
    /// took the cursor with it and the wheel did not.
    ///
    /// Only for the *focused* window. Scrolling a window you are not typing in
    /// must not move point, because point belongs to the buffer rather than to
    /// the window, and another window may be showing the same buffer -- and
    /// because nothing is about to snap back there anyway.
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
        // The last line may sit on the top row and no further. Past that the
        // window fills with the nothing after the end of the buffer, which the
        // reader then has to scroll back out of by hand.
        //
        // Further than `scroll_focused` allows, and on purpose: a wheel is a
        // way of *looking* at the end of a file, where a screenful key is a
        // way of reading through it.
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

    /// Move the boundary of the split at PATH by DELTA cells.
    pub fn resize_split(&mut self, path: &[Side], delta: isize) -> bool {
        let rect = self.bounds();
        self.root.resize_split(path, rect, delta)
    }
}
