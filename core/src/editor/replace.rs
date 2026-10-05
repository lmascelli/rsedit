//! The verbs a replace is driven by.
//!
//! # Why this is a set of verbs and not a loop
//!
//! Because the thing asking is not part of the editor. A terminal asks with
//! single keys; something with a window asks with buttons; a script does not
//! ask at all. None of those can be written as a loop that reads an answer --
//! there is no blocking read here, and a button press is not a read in any
//! case -- so the loop is turned inside out. [`crate::text::search::Replace`] holds
//! the place in it, and whoever is asking calls one of these per answer.
//!
//! That split is the point, and it is the same one `*minibuffer-read-function*`
//! and `*completion-read-function*` already make: what a thing *is* lives in
//! Rust and cannot be taken away by a missing module, and how it is *presented*
//! is named by a variable anything can rebind.
//!
//! # What belongs on which side
//!
//! Moving point to the match is here, because where you are in a buffer is not
//! a matter of presentation -- every view wants it, and a view that had to do
//! it would be a view that could get it wrong. Marking the match is here for
//! the blunter reason that a match nobody can see is useless in every view, so
//! leaving it to each one means each one has the same bug until it doesn't. A
//! view that wants to draw its own removes the overlay; that is one call.
//!
//! What is *not* here: asking. Nothing in this file puts a question on screen
//! or reads a key.
use super::*;
use crate::primitives::edits;
use crate::text::search::{Pattern, Replace, expand_replacement};

/// The overlay category the current match is marked with.
///
/// A category rather than a remembered id, so that clearing is "remove
/// everything of mine" -- which is right even after something else has edited
/// the buffer underneath, and which cannot leak one overlay per session the
/// way a forgotten id would.
pub(crate) const REPLACE_OVERLAY: &str = "replace-match";

impl<B: BufferTrait> EditorState<B> {
    /// Begin a replace in the current buffer, and find the first match.
    ///
    /// The region when the mark is active, the whole buffer from point
    /// otherwise -- which is what everybody means by "replace in the region"
    /// without having to say so.
    ///
    /// Answers the error a bad pattern gave, or `None` for a session that
    /// started. Whether it *found* anything is a separate question:
    /// `replace_match` answers that.
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

    /// Whether a replace is running.
    pub(crate) fn replace_active(&self) -> bool {
        self.runtime(|runtime| runtime.replace().is_some())
    }

    /// The match being offered: where it is, what it says, and what it would
    /// become.
    pub(crate) fn replace_match(&self) -> Option<(usize, usize, String, String)> {
        let (start, end) = self.runtime(|runtime| {
            let found = runtime.replace()?.found.as_ref()?;
            Some((found.start, found.end))
        })?;
        // Read before the session is asked what it would become: the
        // expansion needs the matched text, and only the buffer has it.
        let text = self.matched_text(start, end);
        let expansion = self.runtime(|runtime| runtime.replace()?.expansion(&text))?;
        Some((start, end, text, expansion))
    }

    /// The characters between START and END of the current buffer.
    fn matched_text(&self, start: usize, end: usize) -> String {
        self.with_current_buffer(|buf| buf.text.slice(start, end))
    }

    /// How many have been replaced so far.
    pub(crate) fn replace_count(&self) -> usize {
        self.runtime(|runtime| runtime.replace().map_or(0, |session| session.replaced))
    }

    /// Put point on the current match and mark it, or clear the mark when
    /// there is nothing left to offer.
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

    /// Replace the match being offered, then find the next. False when there
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

        // Its own undo entry, which is what makes stepping back possible at
        // all: going back one replacement is an undo plus a re-search.
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

    /// Leave the match being offered and find the next.
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

    /// Replace this one and every one left, with nothing more asked.
    pub(crate) fn replace_rest(&self) -> usize {
        // Bounded by the buffer's length: a pattern and a replacement that
        // between them grow the text forever -- `x` becoming `xx` -- would
        // otherwise be an editor that never came back. The bound is generous
        // enough that no honest replace reaches it.
        let ceiling = self.with_current_buffer(|buf| buf.text.len()) + 1;
        let mut count = 0;
        while self.replace_this() && count < ceiling {
            count += 1;
        }
        count
    }

    /// Take back the last replacement and offer it again. False when there is
    /// none.
    ///
    /// Two halves that have to agree: the *text* goes back through the undo
    /// history, and the *scan* goes back through the session's own record of
    /// where each replacement began. Neither can be derived from the other --
    /// the buffer does not know what the scan was doing, and the session does
    /// not know how to put characters back.
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
            undo.undo(text);
        });
        self.seek_next_replace();
        true
    }

    /// Look for the next match and show it.
    fn seek_next_replace(&self) {
        let buffer = self.runtime(|runtime| runtime.replace().map(|s| s.buffer.clone()));
        let Some(buffer) = buffer else { return };
        // The scan reads the buffer and the session is behind another lock, so
        // the session is taken out, advanced against the text, and put back --
        // rather than holding both at once.
        let mut session = match self.runtime_mut(|runtime| runtime.take_replace()) {
            Some(session) => session,
            None => return,
        };
        self.with_buffer(&buffer, |buf| session.seek(&buf.text));
        self.runtime_mut(|runtime| runtime.begin_replace(session));
        self.show_replace_match();
    }

    /// End the session and clear what it was showing. Answers how many were
    /// replaced.
    pub(crate) fn end_replace(&self, restore_point: bool) -> usize {
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
