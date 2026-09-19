//! Faces put on text by something other than the mode's own colouring.
//!
//! # What this is for
//!
//! A buffer's colouring comes from its mode's syntax rules, which are regular
//! expressions over the text: they can say "a word in this list is a keyword"
//! and cannot say "*this* span, here, because of something that happened". A
//! search wanting to mark every match, a diagnostic arriving from elsewhere, a
//! manual page whose formatting was in the bytes rather than in the words --
//! none of those are expressible as a pattern.
//!
//! An overlay is that: a span of text, a face, and a note of what made it.
//!
//! # The one hard part
//!
//! Everything else in this editor that knows a position either recomputes it
//! every frame (syntax spans, the region) or is invalidated the moment the text
//! changes. The mark is *deactivated* by every edit, and [`crate::buffer::mark`]
//! says why in as many words: an active mark can never have had an edit under
//! it, so it never needs adjusting for one.
//!
//! Overlays are the first thing in the editor that must survive an edit. That
//! is the whole of what makes them interesting, and [`OverlayTable::adjust_for_insert`]
//! and [`OverlayTable::adjust_for_delete`] are where it happens -- called from
//! the same two doors that undo and the version stamp hang off, for the same
//! reason: a position that moved without the overlays moving with it is a
//! highlight drawn over the wrong words, minutes later, with nothing to say
//! why.
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
    /// Whether this overlay covers `position`.
    pub fn covers(&self, position: usize) -> bool {
        position >= self.start && position < self.end
    }
}

/// A buffer's overlays, kept in order.
///
/// # Why sorted
///
/// Sorted by `start`, always. Every edit has to shift the overlays after it,
/// and sorted order is what makes that a binary search and a walk of the tail
/// rather than a walk of everything. The two adjustments below are monotonic,
/// so the order survives them and never has to be restored.
#[derive(Clone, Debug, Default)]
pub struct OverlayTable {
    /// Sorted by `start`, then by `id` so the order is total and stable.
    overlays: Vec<Overlay>,
    /// The furthest `end` of any overlay here.
    ///
    /// An edit past this point cannot affect any of them, which is the common
    /// case -- highlights are usually behind where you are typing -- and this
    /// turns it into one comparison instead of a search.
    furthest_end: usize,
    /// The next id to hand out. Never reused, so a stale handle in Lisp names
    /// nothing rather than naming whatever took its place.
    next_id: usize,
}

impl OverlayTable {
    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    pub fn len(&self) -> usize {
        self.overlays.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Overlay> {
        self.overlays.iter()
    }

    /// Add an overlay and return its id.
    ///
    /// An empty span is refused: an overlay over no characters draws nothing,
    /// and keeping one means a buffer slowly filling with entries that cannot
    /// be seen and so are never noticed.
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

    /// Remove the overlay with `id`. True if there was one.
    pub fn remove(&mut self, id: usize) -> bool {
        let before = self.overlays.len();
        self.overlays.retain(|overlay| overlay.id != id);
        let removed = self.overlays.len() != before;
        if removed {
            self.recompute_furthest_end();
        }
        removed
    }

    /// Remove every overlay in `category`, or all of them when it is `None`.
    /// Returns how many went.
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

    /// The overlays covering `position`, highest priority last.
    pub fn at(&self, position: usize) -> Vec<&Overlay> {
        let mut found: Vec<&Overlay> = self
            .overlays
            .iter()
            .filter(|overlay| overlay.covers(position))
            .collect();
        found.sort_by_key(|overlay| (overlay.priority, overlay.id));
        found
    }

    /// Everything overlapping the half-open range `[from, to)`, in draw order:
    /// lowest priority first, so a renderer that paints in order leaves the
    /// highest on top.
    pub fn overlapping(&self, from: usize, to: usize) -> Vec<&Overlay> {
        let mut found: Vec<&Overlay> = self
            .overlays
            .iter()
            .filter(|overlay| overlay.start < to && overlay.end > from)
            .collect();
        found.sort_by_key(|overlay| (overlay.priority, overlay.id));
        found
    }

    /// Move the overlays for `n` characters inserted at `at`.
    ///
    /// # The rule, and why it is asymmetric
    ///
    /// `start` moves when the insertion is at or before it; `end` moves only
    /// when the insertion is strictly before it. That one difference is the
    /// whole of "neither end grows":
    ///
    /// - inserting *at* an overlay's start moves the overlay along, so the new
    ///   text ends up outside it, before it;
    /// - inserting *at* its end leaves it alone, so the new text ends up
    ///   outside it, after it;
    /// - inserting strictly inside moves only `end`, so the overlay absorbs
    ///   the text -- which is right, since text typed into the middle of a
    ///   highlighted region is part of that region.
    ///
    /// A highlighted search match therefore stays exactly the text that
    /// matched, however much is typed either side of it.
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

    /// Move the overlays for the deletion of `[from, to)`.
    ///
    /// Each endpoint maps through the same function -- before the deletion it
    /// stands, after it moves back, inside it collapses to where the deletion
    /// began -- and an overlay left covering nothing is removed. An overlay
    /// over no characters highlights nothing, so keeping it would only fill
    /// the buffer with entries that cannot be seen.
    ///
    /// The consequence to know about: undoing that deletion does not bring the
    /// overlay back. Whatever made it will make it again if it still matters.
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

    fn recompute_furthest_end(&mut self) {
        self.furthest_end = self
            .overlays
            .iter()
            .map(|overlay| overlay.end)
            .max()
            .unwrap_or(0);
    }
}
