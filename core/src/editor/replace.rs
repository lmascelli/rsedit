//! The verbs a replace is driven by: begin, see the match, replace it, skip
//! it, replace the rest, step back, end. Whatever asks -- keys, buttons, a
//! script -- calls one per answer; nothing here asks, or reads a key.
//!
//! Design: `doc/architettura-editor.typ, <sostituzione>`.
use super::*;
use crate::primitives::edits;
use crate::text::replace::{Replace, expand_replacement};
use crate::text::search::Pattern;

/// The category of the overlay that marks the match being offered: a category
/// rather than a remembered id, so that clearing removes all of the replace's
/// overlays, however the buffer has been edited underneath, and none is left
/// behind.
pub(crate) const REPLACE_OVERLAY: &str = "replace-match";

impl<B: BufferTrait> EditorState<B> {
    /// Begins replacing SOURCE with REPLACEMENT in the current buffer -- in
    /// the region when the mark is active, from point to the end otherwise --
    /// and finds the first match.
    ///
    /// Returns the error a bad pattern gave, or `None` for a replace begun.
    /// Whether it found anything is for [`Self::replace_match`] to answer.
    pub(crate) fn begin_replace(
        &self,
        source: &str,
        replacement: &str,
        regexp: bool,
        fold: bool,
        preserve_case: bool,
    ) -> Option<String> {
        let pattern = match Pattern::new(source, regexp, fold) {
            Ok(pattern) => pattern,
            Err(why) => return Some(why),
        };
        let buffer = self.get_current_buffer_name();
        let (origin, limit) = self.with_current_buffer(|buf| {
            match crate::buffer::mark::region_bounds(
                buf.mark,
                buf.text.cursor_pos_1d(),
                buf.text.len(),
            ) {
                Some((start, end)) => (start, end),
                None => (buf.text.cursor_pos_1d(), buf.text.len()),
            }
        });
        let mut session = Replace::new(
            buffer,
            pattern,
            replacement.to_string(),
            origin,
            limit,
            preserve_case,
        );
        self.with_current_buffer(|buf| session.seek(&buf.text));
        self.runtime_mut(|runtime| runtime.begin_replace(session));
        self.show_replace_match();
        None
    }

    /// Returns the match being offered: where it starts and ends, what it
    /// says, and what it would become.
    pub(crate) fn replace_match(&self) -> Option<(usize, usize, String, String)> {
        let (start, end) = self.runtime(|runtime| {
            let found = runtime.replace()?.found.as_ref()?;
            Some((found.start, found.end))
        })?;
        // Read before the session is asked what it becomes: the expansion
        // needs the text matched, which only the buffer has.
        let text = self.matched_text(start, end);
        let expansion = self.runtime(|runtime| runtime.replace()?.expansion(&text))?;
        Some((start, end, text, expansion))
    }

    /// Returns the characters of the current buffer from START to END.
    fn matched_text(&self, start: usize, end: usize) -> String {
        self.with_current_buffer(|buf| buf.text.slice(start, end))
    }

    /// Puts point on the match being offered and marks it, or clears the mark
    /// when nothing is left to offer. Every view wants both; one that draws
    /// its own removes the overlay.
    fn show_replace_match(&self) {
        let bounds = self.runtime(|runtime| {
            runtime
                .replace()
                .and_then(|session| session.found.as_ref())
                .map(|found| (found.start, found.end))
        });
        self.with_current_buffer_mut(|buf| {
            buf.overlays.remove_category(Some(REPLACE_OVERLAY));
            if let Some((start, end)) = bounds {
                buf.overlays
                    .add(start, end, Face::REPLACE_MATCH, 100, REPLACE_OVERLAY.into());
                let (line, column) = buf.text.cursor_1d_to_2d(start);
                buf.text.cursor_move(line, column);
            }
        });
    }

    /// Replaces the match being offered, and finds the next; false when there
    /// was nothing to replace.
    pub(crate) fn replace_this(&self) -> bool {
        let Some(found) = self.runtime(|runtime| {
            let session = runtime.replace()?;
            let found = session.found.clone()?;
            Some((found, session.replacement.clone(), session.preserve_case))
        }) else {
            return false;
        };
        let (found, replacement, preserve_case) = found;
        let matched = self.matched_text(found.start, found.end);
        let text = expand_replacement(&replacement, &found, &matched, preserve_case);
        let new_len = text.chars().count();

        // An undo entry of its own: stepping back is an undo and a search
        // again.
        // TODO(RIC-3): for `replace-string` and `replace-regexp` too, whose
        // replacements are then undone one by one.
        self.with_current_buffer_mut(|buf| {
            let _ = edits::delete_range(buf, found.start, found.end);
            let _ = edits::insert_text(buf, found.start, &text);
            buf.undo.boundary();
        });

        self.runtime_mut(|runtime| {
            if let Some(session) = runtime.replace_mut() {
                session.accept(new_len);
            }
        });
        self.seek_next_replace();
        true
    }

    /// Leaves the match being offered, and finds the next; false when there
    /// was none.
    pub(crate) fn replace_skip(&self) -> bool {
        let moved = self.runtime_mut(|runtime| match runtime.replace_mut() {
            Some(session) if session.found.is_some() => {
                session.decline();
                true
            }
            _ => false,
        });
        if moved {
            self.seek_next_replace();
        }
        moved
    }

    /// Replaces this match and every one after it, asking nothing, and
    /// returns how many.
    pub(crate) fn replace_rest(&self) -> usize {
        // Bounded by the buffer's length: a pattern and a replacement that grow
        // the text for ever -- `x` becoming `xx` -- would be an editor that
        // never came back. No honest replace gets near the bound.
        let ceiling = self.with_current_buffer(|buf| buf.text.len()) + 1;
        let mut count = 0;
        while self.replace_this() && count < ceiling {
            count += 1;
        }
        count
    }

    /// Takes back the last replacement and offers it again; false when there
    /// is none.
    ///
    /// Two halves that must agree: the text goes back through the undo
    /// history, and the scan through the session's record of where each
    /// replacement began. Neither can be worked out from the other.
    pub(crate) fn replace_back(&self) -> bool {
        let stepped = self.runtime_mut(|runtime| {
            runtime
                .replace_mut()
                .map(|session| session.unaccept())
                .unwrap_or(false)
        });
        if !stepped {
            return false;
        }
        self.with_current_buffer_mut(|buf| {
            let Buffer { text, undo, .. } = buf;
            // TODO(BUF-1): below the two doors, like every undo, so the buffer
            // is told here what they would have told it. See `edits::changed`.
            if let Some(applied) = undo.undo(text) {
                crate::primitives::edits::changed(buf, applied.from);
            }
        });
        self.seek_next_replace();
        true
    }

    /// Looks for the next match, and shows it.
    fn seek_next_replace(&self) {
        let buffer = self.runtime(|runtime| runtime.replace().map(|s| s.buffer.clone()));
        let Some(buffer) = buffer else { return };
        // The scan reads the buffer, and the session is under the runtime's
        // lock: the session is taken out, moved on against the text and put
        // back, rather than both being held.
        let mut session = match self.runtime_mut(|runtime| runtime.take_replace()) {
            Some(session) => session,
            None => return,
        };
        self.with_buffer(&buffer, |buf| session.seek(&buf.text));
        self.runtime_mut(|runtime| runtime.begin_replace(session));
        self.show_replace_match();
    }

    /// Ends the replace, clears its mark and, with RESTORE_POINT, puts point
    /// back where it began; returns how many matches were replaced.
    pub(crate) fn end_replace(&self, restore_point: bool) -> usize {
        // TODO(RIC-8): nothing calls this when another key takes down the map
        // of query-replace, and the replace stays open, its match marked.
        let Some(session) = self.runtime_mut(|runtime| runtime.take_replace()) else {
            return 0;
        };
        self.with_current_buffer_mut(|buf| {
            buf.overlays.remove_category(Some(REPLACE_OVERLAY));
            if restore_point {
                let at = session.origin.min(buf.text.len());
                let (line, column) = buf.text.cursor_1d_to_2d(at);
                buf.text.cursor_move(line, column);
            }
        });
        session.replaced
    }
}
