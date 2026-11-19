//! Markers from Lisp: a position in a buffer that the text carries along as it
//! is edited before it, named by a handle, with a side to take of text
//! inserted exactly at it. The table is [`crate::buffer::markers`].
//!
//! Design: `doc/architettura-editor.typ, <mark-overlay>`.
use super::*;

/// Returns EXP as a position, a number not below 0.
fn position<B: BufferTrait>(exp: Option<&ELispExp<B>>) -> Result<usize, EvalError<EditorState<B>>> {
    match exp {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => Ok(*n as usize),
        other => Err(EvalError::WrongArgumentType {
            expected: "a non-negative Number".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

/// Returns EXP as a marker's handle.
fn handle<B: BufferTrait>(exp: Option<&ELispExp<B>>) -> Result<u64, EvalError<EditorState<B>>> {
    match exp {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 => {
            Ok(*n as u64)
        }
        other => Err(EvalError::WrongArgumentType {
            expected: "a marker's handle".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

pub const MAKE_MARKER_DOC: &str = "(make-marker POSITION &optional ADVANCES): Put a marker at \
         POSITION in the current buffer, kept within it, and return a handle to it. The marker \
         moves with the text as it is edited before it. Text inserted exactly at it goes after \
         it, or before it when ADVANCES is not nil.\n\n\
         Example:\n\
         (setq my-start (make-marker (point)))";

primitive!(make_marker, args, _env, ctx, {
    if args.is_empty() || args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let at = position(args.first())?;
    let advances = args.get(1).is_some_and(ELispExp::is_truthy);
    let id = ctx.with_current_buffer_mut(|buf| {
        let at = at.min(buf.text.len());
        buf.markers.add_kind(at, advances)
    });
    Ok(ELispExp::number(id as f64))
});

pub const MARKER_POSITION_DOC: &str = "(marker-position MARKER): Return where MARKER is in the \
         current buffer, or nil when the buffer has no such marker.\n\n\
         Example:\n\
         (goto-char (marker-position my-start))";

primitive!(marker_position, args, _env, ctx, {
    exact_arity(args, 1)?;
    let id = handle(args.first())?;
    Ok(ctx
        .with_current_buffer(|buf| buf.markers.get(id))
        .map_or_else(ELispExp::nil, |at| ELispExp::number(at as f64)))
});

pub const SET_MARKER_DOC: &str = "(set-marker MARKER POSITION): Move MARKER to POSITION in the \
         current buffer, kept within it, and return where it went; nil, moving nothing, when the \
         buffer has no such marker.\n\n\
         Example:\n\
         (set-marker my-start (point-max))";

primitive!(set_marker, args, _env, ctx, {
    exact_arity(args, 2)?;
    let id = handle(args.first())?;
    let at = position(args.get(1))?;
    Ok(ctx
        .with_current_buffer_mut(|buf| {
            let at = at.min(buf.text.len());
            buf.markers.set(id, at).then_some(at)
        })
        .map_or_else(ELispExp::nil, |at| ELispExp::number(at as f64)))
});

pub const DELETE_MARKER_DOC: &str = "(delete-marker MARKER): Take MARKER out of the current \
         buffer, and return t; nil when the buffer has no such marker. A handle is never given \
         to another marker.\n\n\
         Example:\n\
         (delete-marker my-start)";

primitive!(delete_marker, args, _env, ctx, {
    exact_arity(args, 1)?;
    let id = handle(args.first())?;
    Ok(ELispExp::boolean(ctx.with_current_buffer_mut(|buf| {
        let had = buf.markers.get(id).is_some();
        buf.markers.remove(id);
        had
    })))
});

/// Registers the primitives of the markers.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("make-marker", make_marker, MAKE_MARKER_DOC);
    into.function("marker-position", marker_position, MARKER_POSITION_DOC);
    into.function("set-marker", set_marker, SET_MARKER_DOC);
    into.function("delete-marker", delete_marker, DELETE_MARKER_DOC);
}
