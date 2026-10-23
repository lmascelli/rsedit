//! Overlays: faces put on spans of text by something other than the mode's
//! colouring -- a search marking its matches, a diagnostic, the emphasis of a
//! manual page. Unlike the other positions the editor keeps, they move with
//! the text at every edit: the two doors call the adjustments here.
//!
//! Design: `doc/architettura-editor.typ, <mark-overlay>`.
use crate::ui::Face;
use std::sync::Arc;

/// A span of text drawn in a face.
#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    /// Half-open character offsets into the buffer.
    pub start: usize,
    pub end: usize,
    pub face: Face,
    /// Higher wins where two overlap; equal priorities are drawn oldest first,
    /// so the later one covers the earlier.
    pub priority: i32,
    /// What made it. A producer replaces its own batch by category rather than
    /// by remembering every handle it was given -- which is what it wants when
    /// a search is re-run, and what still works after the module is reloaded.
    pub category: Arc<str>,
    /// Stable for the life of the overlay, so Lisp can hold one.
    pub id: usize,
}

impl Overlay {
    /// Returns whether this overlay covers POSITION.
    pub fn covers(&self, position: usize) -> bool {
        position >= self.start && position < self.end
    }
}

/// A buffer's overlays, sorted by start: an edit shifts the ones after it,
/// found by a binary search, and both adjustments keep the order.
#[derive(Clone, Debug, Default)]
pub struct OverlayTable {
    /// Sorted by `start`, then by `id` so the order is total and stable.
    overlays: Vec<Overlay>,
    /// The furthest `end` of any overlay here. An edit past it changes none
    /// of them -- the common case, since highlights are usually behind where
    /// the typing is -- and this makes it one comparison.
    furthest_end: usize,
    /// The next id to hand out. Never reused, so a stale handle in Lisp names
    /// nothing rather than naming whatever took its place.
    next_id: usize,
}

impl OverlayTable {
    /// Returns whether there are no overlays.
    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    /// Returns how many overlays there are.
    pub fn len(&self) -> usize {
        self.overlays.len()
    }

    /// Returns the overlays, sorted by start.
    pub fn iter(&self) -> impl Iterator<Item = &Overlay> {
        self.overlays.iter()
    }

    /// Adds an overlay and returns its id, or `None` for an empty span, which
    /// would draw nothing and never be noticed.
    pub fn add(
        &mut self,
        start: usize,
        end: usize,
        face: Face,
        priority: i32,
        category: Arc<str>,
    ) -> Option<usize> {
        let (start, end) = (start.min(end), start.max(end));
        if start == end {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let overlay = Overlay {
            start,
            end,
            face,
            priority,
            category,
            id,
        };
        // Inserted in place rather than pushed and re-sorted: the table is
        // sorted by definition, and a producer adding a thousand matches one
        // at a time would otherwise sort a thousand times.
        let at = self.overlays.partition_point(|existing| {
            (existing.start, existing.id) < (overlay.start, overlay.id)
        });
        self.overlays.insert(at, overlay);
        self.furthest_end = self.furthest_end.max(end);
        Some(id)
    }

    /// Removes the overlay with ID. True if there was one.
    pub fn remove(&mut self, id: usize) -> bool {
        let before = self.overlays.len();
        self.overlays.retain(|overlay| overlay.id != id);
        let removed = self.overlays.len() != before;
        if removed {
            self.recompute_furthest_end();
        }
        removed
    }

    /// Removes every overlay in CATEGORY, or all of them when it is `None`,
    /// and returns how many went.
    pub fn remove_category(&mut self, category: Option<&str>) -> usize {
        let before = self.overlays.len();
        match category {
            Some(category) => self
                .overlays
                .retain(|overlay| &*overlay.category != category),
            None => self.overlays.clear(),
        }
        let removed = before - self.overlays.len();
        if removed > 0 {
            self.recompute_furthest_end();
        }
        removed
    }

    /// Returns the overlays covering POSITION, highest priority last.
    pub fn at(&self, position: usize) -> Vec<&Overlay> {
        let mut found: Vec<&Overlay> = self
            .overlays
            .iter()
            .filter(|overlay| overlay.covers(position))
            .collect();
        found.sort_by_key(|overlay| (overlay.priority, overlay.id));
        found
    }

    /// Returns everything overlapping the half-open range `[from, to)`, in
    /// drawing order: lowest priority first, so a renderer that paints in
    /// order leaves the highest on top.
    pub fn overlapping(&self, from: usize, to: usize) -> Vec<&Overlay> {
        let mut found: Vec<&Overlay> = self
            .overlays
            .iter()
            .filter(|overlay| overlay.start < to && overlay.end > from)
            .collect();
        found.sort_by_key(|overlay| (overlay.priority, overlay.id));
        found
    }

    /// Moves the overlays for N characters inserted at AT.
    ///
    /// `start` moves when the insertion is at or before it, `end` only when
    /// it is strictly before it. So neither end grows: text inserted at an
    /// overlay's start or end stays outside it, and text inserted strictly
    /// inside is absorbed. A highlighted match stays the text that matched.
    pub fn adjust_for_insert(&mut self, at: usize, n: usize) {
        if n == 0 || self.overlays.is_empty() {
            return;
        }
        // Nothing reaches past here, so an insertion past here changes nothing.
        if at >= self.furthest_end {
            return;
        }
        // Everything from here on has `start >= at` and must shift entirely.
        // The ones before it matter only if they span the insertion point.
        let tail = self.overlays.partition_point(|overlay| overlay.start < at);
        for overlay in &mut self.overlays[..tail] {
            if overlay.end > at {
                overlay.end += n;
            }
        }
        for overlay in &mut self.overlays[tail..] {
            overlay.start += n;
            overlay.end += n;
        }
        self.furthest_end += n;
    }

    /// Moves the overlays for the deletion of `[from, to)`.
    ///
    /// Each end that was inside the deletion lands where it began, and an
    /// overlay left covering nothing is removed. Undoing the deletion does not
    /// bring it back: whatever made it makes it again if it still matters.
    pub fn adjust_for_delete(&mut self, from: usize, to: usize) {
        if from >= to || self.overlays.is_empty() {
            return;
        }
        if from >= self.furthest_end {
            return;
        }
        let shift = to - from;
        let moved = |position: usize| -> usize {
            if position <= from {
                position
            } else if position >= to {
                position - shift
            } else {
                from
            }
        };
        for overlay in &mut self.overlays {
            overlay.start = moved(overlay.start);
            overlay.end = moved(overlay.end);
        }
        self.overlays.retain(|overlay| overlay.start < overlay.end);
        self.recompute_furthest_end();
    }

    /// Works out `furthest_end` again from the overlays.
    fn recompute_furthest_end(&mut self) {
        self.furthest_end = self
            .overlays
            .iter()
            .map(|overlay| overlay.end)
            .max()
            .unwrap_or(0);
    }
}
