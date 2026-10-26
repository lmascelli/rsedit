//! The two doors every change to a buffer's text goes through, and the
//! primitives that insert and delete characters.
//!
//! Design: `doc/architettura-editor.typ, <due-porte>`.
use super::*;
use crate::buffer::{Buffer, Mark, undo};
use crate::primitives::args::repeat_count;

/// Deletes the text between FROM and TO, in either order and clamped to the
/// text, and records it so that it can be undone.
///
/// Returns false, and changes nothing, in a read-only buffer: the caller says
/// so, with [`edited`]. An empty range is not a refusal.
#[must_use]
pub(crate) fn delete_range<B: BufferTrait>(buf: &mut Buffer<B>, from: usize, to: usize) -> bool {
    if buf.read_only {
        return false;
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
    buf.mark = deactivated(buf.mark);
    buf.overlays.adjust_for_delete(start, end);
    buf.virtual_text.adjust_for_delete(start, end);
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
        return false;
    }
    if content.is_empty() {
        return true;
    }
    let at = at.min(buf.text.len());
    let point = buf.text.cursor_pos_1d();
    let inserted = content.chars().count();
    buf.undo.record_insert(at, inserted, point);
    buf.mark = deactivated(buf.mark);
    // Before the text moves, each by its table's own rule (`buffer::overlay`,
    // `buffer::virtual_text`).
    buf.overlays.adjust_for_insert(at, inserted);
    buf.virtual_text.adjust_for_insert(at, inserted);
    undo::apply_insert(&mut buf.text, at, content);
    buf.is_modified = true;
    changed(buf, at);
    true
}

/// Tells BUF that its text changed from offset AT: bumps the version,
/// invalidates the colouring and the scan checkpoints from AT's line down,
/// and emits the buffer's `buffer-changed`.
///
/// The two doors call it, and so do the callers of undo and redo, which change
/// the text below the doors and must say so themselves.
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
#[cfg(not(target_os = "windows"))]
fn normalize_line_endings(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

// TODO(CMD-7): on Windows a paste keeps its CRs, with no reason written, and
// the tests in `paste_tests` that expect lines fail there.
/// Returns TEXT as it is.
#[cfg(target_os = "windows")]
fn normalize_line_endings(text: &str) -> std::borrow::Cow<'_, str> {
    std::borrow::Cow::Borrowed(text)
}

/// Inserts CONTENT at point, through [`insert_text`].
#[must_use]
pub(crate) fn insert_at_point<B: BufferTrait>(buf: &mut Buffer<B>, content: &str) -> bool {
    let at = buf.text.cursor_pos_1d();
    insert_text(buf, at, content)
}

pub const SELF_INSERT_DOC: &str = "(self-insert STRING): Insert the first character of STRING at \
         point. It is what each printable key runs, with its own character.\n\n\
         Example:\n\
         (self-insert \"a\")";

// TODO(CMD-5): the prefix argument is not read, so `C-u 3 x` inserts one x.
primitive!(self_insert, args, _env, ctx, {
    if let Some(ELispExp::String(s)) = args.first() {
        if let Some(c) = s.chars().next() {
            let happened = ctx.with_current_buffer_mut(|buf| {
                let mut utf8 = [0u8; 4];
                insert_at_point(buf, c.encode_utf8(&mut utf8))
            });
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

pub const INSERT_NEWLINE_DOC: &str = "(insert-newline): Insert a newline at point, without \
         indenting the new line.\n\n\
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
         point, 1 by default, across line ends, without putting them on the kill ring.";

primitive!(delete_char, args, _env, ctx, {
    let count = repeat_count(args)?;
    let happened = ctx.with_current_buffer_mut(|buf| {
        let from = buf.text.cursor_pos_1d();
        delete_range(buf, from, from + count)
    });
    Ok(edited(ctx, happened))
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
    into.command("insert-newline", insert_newline, &[], INSERT_NEWLINE_DOC);
    into.command(
        "delete-backward-char",
        delete_backward_char,
        &[],
        DELETE_BACKWARD_CHAR_DOC,
    );
    into.command("delete-char", delete_char, &["p"], DELETE_CHAR_DOC);
}
