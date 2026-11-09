//! The two doors every change to a buffer's text goes through, the third that
//! undo and redo replay the history through, and the primitives that insert
//! and delete characters.
//!
//! Design: `doc/architettura-editor.typ, <due-porte>`.
use super::*;
use crate::buffer::{Buffer, Mark, Touched, undo};
use crate::primitives::args::movement_count;
use std::cell::Cell;

thread_local! {
    /// Whether a door has refused an edit on this thread since the last
    /// [`take_refusal`]: what the end of a command reads, to say the refusal
    /// whichever command it was, rather than each command remembering to.
    static REFUSED: Cell<bool> = const { Cell::new(false) };
}

/// Records that a door refused an edit, and returns false, what the door
/// answers.
fn refuse() -> bool {
    REFUSED.with(|refused| refused.set(true));
    false
}

/// Returns whether a door has refused an edit on this thread since the last
/// call, and forgets it.
///
/// `run_command_form` asks before a command, to start clean, and after it, to
/// say "Buffer is read-only" for a command that tried to change a read-only
/// buffer and said nothing: a kill, a verb, a comment. One that said it
/// itself, with [`edited`], says it twice with the same words.
pub(crate) fn take_refusal() -> bool {
    REFUSED.with(|refused| refused.replace(false))
}

/// Deletes the text between FROM and TO, in either order and clamped to the
/// text, and records it so that it can be undone.
///
/// Returns false, and changes nothing, in a read-only buffer: the caller says
/// so, with [`edited`]. An empty range is not a refusal.
#[must_use]
pub(crate) fn delete_range<B: BufferTrait>(buf: &mut Buffer<B>, from: usize, to: usize) -> bool {
    if buf.read_only {
        return refuse();
    }
    let start = from.min(to);
    let end = from.max(to).min(buf.text.len());
    if start >= end {
        // Nothing to remove, which is not a refusal: a command that asked to
        // delete an empty range got what it wanted.
        return true;
    }
    let point = buf.text.cursor_pos_1d();
    let whole_buffer = start == 0 && end == buf.text.len();
    // Read before the deletion, the only time it can be: the whole buffer in
    // one pass, where `slice` pays a lookup per character.
    let removed: String = if whole_buffer {
        buf.text.to_string()
    } else {
        buf.text.slice(start, end)
    };
    buf.undo.record_delete(start, removed, point);
    if let Some(touched) = buf.touched.as_mut() {
        touched.note(&buf.text, start, end);
    }
    buf.mark = deactivated(buf.mark);
    buf.overlays.adjust_for_delete(start, end);
    buf.virtual_text.adjust_for_delete(start, end);
    buf.markers.adjust_for_delete(start, end);
    if whole_buffer {
        // One step rather than a deletion per character: see
        // `BufferTrait::clear`.
        buf.text.clear();
    } else {
        // From the far end backwards: one move of the gap for the range.
        undo::apply_delete(&mut buf.text, start, end);
    }
    buf.is_modified = true;
    changed(buf, start);
    true
}

/// Inserts CONTENT at offset AT, clamped to the text, and records it so that
/// it can be undone.
///
/// Returns false, and changes nothing, in a read-only buffer.
#[must_use]
pub(crate) fn insert_text<B: BufferTrait>(buf: &mut Buffer<B>, at: usize, content: &str) -> bool {
    if buf.read_only {
        return refuse();
    }
    if content.is_empty() {
        return true;
    }
    let at = at.min(buf.text.len());
    let point = buf.text.cursor_pos_1d();
    let inserted = content.chars().count();
    buf.undo.record_insert(at, inserted, point);
    if let Some(touched) = buf.touched.as_mut() {
        touched.note(&buf.text, at, at);
    }
    buf.mark = deactivated(buf.mark);
    // Before the text moves, each by its table's own rule (`buffer::overlay`,
    // `buffer::virtual_text`).
    buf.overlays.adjust_for_insert(at, inserted);
    buf.virtual_text.adjust_for_insert(at, inserted);
    buf.markers.adjust_for_insert(at, inserted);
    undo::apply_insert(&mut buf.text, at, content);
    buf.is_modified = true;
    changed(buf, at);
    true
}

/// Tells BUF that its text changed from offset AT: bumps the version,
/// invalidates the colouring and the scan checkpoints from AT's line down,
/// and emits the buffer's `buffer-changed`.
///
/// The three doors call it.
pub(crate) fn changed<B: BufferTrait>(buf: &mut Buffer<B>, at: usize) {
    buf.version = buf.version.wrapping_add(1);
    let line = buf.text.cursor_1d_to_2d(at.min(buf.text.len())).0;
    buf.syntax.invalidate_from(buf.version, line);
    buf.syntax.truncate(buf.text.line_count());
    // The other lexer's cache, from the same line: two caches that disagreed
    // about where the edit was would each describe a different text.
    buf.scan.invalidate_from(buf.version, line);
    buf.changed.emit();
}

/// Which way [`replay`] goes through the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Replay {
    Undo,
    Redo,
}

/// What [`replay`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Replayed {
    /// The buffer is read-only, and nothing changed: the caller says so.
    Refused,
    /// There was nothing to undo, or to redo.
    Nothing,
    /// A group was replayed: where point goes, and where the text changed.
    Applied(undo::Applied),
}

/// The third door: undoes or redoes the latest group of BUF's history, doing
/// for each change what the two doors do for an edit, except recording it --
/// the history writes its own inverse. Refuses a read-only buffer, moves the
/// overlays, the virtual text and the markers with the text, tells what is
/// watching the edits, ends a selection, and tells the buffer with
/// [`changed`].
pub(crate) fn replay<B: BufferTrait>(buf: &mut Buffer<B>, way: Replay) -> Replayed {
    if buf.read_only {
        refuse();
        return Replayed::Refused;
    }
    // Split borrow: the history writes the text, and each change it is about
    // to make moves what is anchored to it, all fields of the one buffer.
    let Buffer {
        text,
        undo,
        overlays,
        virtual_text,
        markers,
        touched,
        ..
    } = buf;
    // The length of the text as each step finds it, which the history does
    // not say: what is watching the edits measures from the end too.
    let mut length = text.len();
    let mut follow = |step: undo::Step| match step {
        undo::Step::Insert { at, len } => {
            overlays.adjust_for_insert(at, len);
            virtual_text.adjust_for_insert(at, len);
            markers.adjust_for_insert(at, len);
            if let Some(touched) = touched.as_mut() {
                touched.note_unread(at, at, length);
            }
            length += len;
        }
        undo::Step::Delete { from, to } => {
            overlays.adjust_for_delete(from, to);
            virtual_text.adjust_for_delete(from, to);
            markers.adjust_for_delete(from, to);
            if let Some(touched) = touched.as_mut() {
                touched.note_unread(from, to, length);
            }
            length -= to.saturating_sub(from).min(length);
        }
    };
    let applied = match way {
        Replay::Undo => undo.undo(text, &mut follow),
        Replay::Redo => undo.redo(text, &mut follow),
    };
    let Some(applied) = applied else {
        return Replayed::Nothing;
    };
    buf.mark = deactivated(buf.mark);
    buf.is_modified = true;
    changed(buf, applied.from);
    Replayed::Applied(applied)
}

/// Returns MARK, if there is one, no longer active.
///
/// Every edit passes here, so an edit ends a selection, as in Emacs. The
/// position stays, for `exchange-point-and-mark`.
fn deactivated(mark: Option<Mark>) -> Option<Mark> {
    mark.map(|mark| Mark {
        active: false,
        ..mark
    })
}

/// Returns what an editing primitive answers, nil, having said "Buffer is
/// read-only" in the echo area when HAPPENED is false.
///
/// Called once the buffer's closure has returned: the echo area is a
/// compartment of its own, never taken under a buffer's lock.
pub(crate) fn edited<B: BufferTrait>(ctx: &EditorState<B>, happened: bool) -> ELispExp<B> {
    if !happened {
        ctx.set_echo_message("Buffer is read-only");
    }
    ELispExp::nil()
}

/// Returns TEXT with every CRLF and every lone CR made a LF, the only line
/// break the buffer counts; borrowed when TEXT has no CR in it.
///
/// On every system: the clipboard of Windows hands over CRLF, and a buffer
/// counts its lines on `\n` there too.
fn normalize_line_endings(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Inserts CONTENT at point, through [`insert_text`].
#[must_use]
pub(crate) fn insert_at_point<B: BufferTrait>(buf: &mut Buffer<B>, content: &str) -> bool {
    let at = buf.text.cursor_pos_1d();
    insert_text(buf, at, content)
}

pub const SELF_INSERT_DOC: &str = "(self-insert STRING &optional N): Insert the first character \
         of STRING at point, N times, 1 by default. It is what each printable key runs, with its \
         own character and the prefix argument as N: C-u 3 x inserts xxx. N less than 0 is an \
         error, as in Emacs.\n\n\
         Example:\n\
         (self-insert \"a\")\n\
         (self-insert \"-\" 40)";

primitive!(self_insert, args, _env, ctx, {
    let times = match args.get(1) {
        None => 1,
        Some(ELispExp::Number(n)) if *n < 0.0 => {
            return Err(EvalError::RuntimeMessage(format!(
                "Negative repetition argument {n}"
            )));
        }
        Some(ELispExp::Number(n)) => args::width(*n)?,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.clone(),
            });
        }
    };
    if let Some(ELispExp::String(s)) = args.first() {
        if let Some(c) = s.chars().next() {
            // One insertion of N characters: one edit, one undo step.
            let text: String = std::iter::repeat_n(c, times).collect();
            let happened = ctx.with_current_buffer_mut(|buf| insert_at_point(buf, &text));
            return Ok(edited(ctx, happened));
        }
        Ok(ELispExp::symbol("nil".into()))
    } else {
        Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        })
    }
});

pub const INSERT_DOC: &str = "(insert &rest STRINGS): Insert STRINGS at point, one after another, \
         and return nil. Anything that is not a string goes in as `format' writes it with %s. The \
         whole insertion is one edit and one undo step; a read-only buffer refuses it and says \
         so.\n\n\
         Example:\n\
         (insert \"total: \" 42 \"\\n\")";

primitive!(insert, args, _env, ctx, {
    // Joined first, so that `(insert a b c)` is one edit: one undo step, and
    // one change for the highlighter to colour.
    let mut text = String::new();
    for arg in args {
        match arg {
            ELispExp::String(s) => text.push_str(s),
            other => text.push_str(&risp::lisp_display(other)),
        }
    }
    let happened = ctx.with_current_buffer_mut(|buf| insert_at_point(buf, &text));
    Ok(edited(ctx, happened))
});

pub const DELETE_REGION_DOC: &str = "(delete-region FROM TO): Delete the text between FROM and TO \
         without putting it on the kill ring, and return t if the buffer changed.\n\n\
         Example:\n\
         (delete-region (point-min) (point-max))  ; empty the buffer";

primitive!(delete_region, args, _env, ctx, {
    let (from, to) = match (args.first(), args.get(1)) {
        (Some(ELispExp::Number(from)), Some(ELispExp::Number(to))) => {
            (*from as usize, *to as usize)
        }
        _ => {
            return Err(EvalError::WrongArgumentType {
                expected: "two positions".into(),
                got: args.first().cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    let changed = ctx.with_current_buffer_mut(|buf| {
        let end = from.max(to).min(buf.text.len());
        let start = from.min(to).min(end);
        start < end && delete_range(buf, start, end)
    });
    Ok(ELispExp::boolean(changed))
});

pub const INSERT_PASTED_TEXT_DOC: &str = "(insert-pasted-text STRING): Insert STRING at point as \
         one paste, which is what the terminal's paste runs: one command and one undo step, \
         without `post-self-insert-hook', so nothing in it is paired. A read-only buffer refuses \
         it.\n\n\
         Example:\n\
         (insert-pasted-text \"fn main() {}\")";

primitive!(insert_pasted_text, args, _env, ctx, {
    let Some(ELispExp::String(text)) = args.first() else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    // One call, like any insertion: what makes it a paste is the caller. The
    // line endings are made plain at this door, which every paste comes
    // through; `insert` keeps a CR written on purpose.
    let text = normalize_line_endings(text);
    let happened = ctx.with_current_buffer_mut(|buf| insert_at_point(buf, &text));
    Ok(edited(ctx, happened))
});

pub const NEWLINE_DOC: &str = "(newline &optional N): Insert N newlines at point, 1 by \
         default, and nothing else: what Return runs. Run from a key, it is followed by \
         `post-self-insert-hook', as a typed character is, so that a minor mode such as \
         `electric-indent-mode' can indent the line it opens; called from Lisp, it runs no hook. \
         N less than 0 is an error.\n\n\
         Example:\n\
         (newline 2)";

primitive!(newline, args, _env, ctx, {
    let times = match args.first() {
        None => 1,
        Some(exp) if exp.is_nil() => 1,
        Some(ELispExp::Number(n)) if *n < 0.0 => {
            return Err(EvalError::RuntimeMessage(format!(
                "Negative repetition argument {n}"
            )));
        }
        Some(ELispExp::Number(n)) => args::width(*n)?,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.clone(),
            });
        }
    };
    let text = "\n".repeat(times);
    let happened = ctx.with_current_buffer_mut(|buf| insert_at_point(buf, &text));
    Ok(edited(ctx, happened))
});

pub const INSERT_NEWLINE_DOC: &str = "(insert-newline): Insert a newline at point, without \
         indenting the new line: unlike `newline', it is never followed by \
         `post-self-insert-hook', whatever minor modes are on.\n\n\
         Example:\n\
         (define-key nil \"<ret>\" 'insert-newline)";

primitive!(insert_newline, _args, _env, ctx, {
    let happened = ctx.with_current_buffer_mut(|buf| insert_at_point(buf, "\n"));
    Ok(edited(ctx, happened))
});

pub const DELETE_BACKWARD_CHAR_DOC: &str = "(delete-backward-char): Delete the character before \
         point. It takes no count.";

primitive!(delete_backward_char, _args, _env, ctx, {
    let happened = ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        delete_range(buf, point.saturating_sub(1), point)
    });
    Ok(edited(ctx, happened))
});

pub const DELETE_CHAR_DOC: &str = "(delete-char &optional N): Delete the N characters after \
         point, 1 by default, across line ends, without putting them on the kill ring; with N \
         negative, the characters before point.";

primitive!(delete_char, args, _env, ctx, {
    let count = movement_count(args)?;
    let happened = ctx.with_current_buffer_mut(|buf| {
        let point = buf.text.cursor_pos_1d();
        // Saturating: a count past either end of the buffer means to that end,
        // and an arithmetic that wrapped would delete the wrong way.
        let reach = count.unsigned_abs();
        if count < 0 {
            delete_range(buf, point.saturating_sub(reach), point)
        } else {
            delete_range(buf, point, point.saturating_add(reach))
        }
    });
    Ok(edited(ctx, happened))
});

pub const TRACK_CHANGES_BEGIN_DOC: &str = "(track-changes-begin): Start watching what the edits \
         to the current buffer reach, from now, forgetting what was watched before. Watching keeps \
         the text of the stretch edited, not a copy of the buffer: see `track-changes-end'.\n\n\
         Example:\n\
         (track-changes-begin)";

primitive!(track_changes_begin, args, _env, ctx, {
    exact_arity(args, 0)?;
    ctx.with_current_buffer_mut(|buf| buf.touched = Some(Touched::new(buf.text.len())));
    Ok(ELispExp::nil())
});

pub const TRACK_CHANGES_END_DOC: &str = "(track-changes-end): Stop watching the current buffer, \
         and return what the edits since `track-changes-begin' changed, as (START REMOVED \
         INSERTED): from START, REMOVED characters became the string INSERTED. Return nil when \
         nothing was edited, or nothing was watched. It is what `string-diff' says of the text \
         before and after, within the stretch the edits reached: a word typed and deleted again \
         is (START 0 \"\"). Undo and redo count as edits; one that reached past the stretch makes \
         the whole stretch the change.\n\n\
         Example:\n\
         (track-changes-begin)\n\
         (insert \"abc\")\n\
         (track-changes-end)   ; (0 0 \"abc\") in an empty buffer";

primitive!(track_changes_end, args, _env, ctx, {
    exact_arity(args, 0)?;
    let change = ctx.with_current_buffer_mut(|buf| buf.touched.take()?.change(&buf.text));
    Ok(match change {
        None => ELispExp::nil(),
        Some(change) => ELispExp::proper_list(vec![
            ELispExp::Number(change.start as f64),
            ELispExp::Number(change.removed as f64),
            ELispExp::string(change.inserted),
        ]),
    })
});

/// Registers the primitives that insert and delete characters.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("delete-region", delete_region, DELETE_REGION_DOC);
    // Functions, not commands: no argument spec collects arbitrary text, and
    // a printable key is bound to a form that carries its own character,
    // `(self-insert "x")`.
    into.function("self-insert", self_insert, SELF_INSERT_DOC);
    into.function("insert", insert, INSERT_DOC);
    into.command(
        "insert-pasted-text",
        insert_pasted_text,
        &[],
        INSERT_PASTED_TEXT_DOC,
    );
    into.command("newline", newline, &["p"], NEWLINE_DOC);
    into.command("insert-newline", insert_newline, &[], INSERT_NEWLINE_DOC);
    into.command(
        "delete-backward-char",
        delete_backward_char,
        &[],
        DELETE_BACKWARD_CHAR_DOC,
    );
    into.command("delete-char", delete_char, &["p"], DELETE_CHAR_DOC);
    into.function(
        "track-changes-begin",
        track_changes_begin,
        TRACK_CHANGES_BEGIN_DOC,
    );
    into.function(
        "track-changes-end",
        track_changes_end,
        TRACK_CHANGES_END_DOC,
    );
}
