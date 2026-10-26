//! Overlays from Lisp: a face on a span of text, for what a mode's patterns
//! cannot say, as a search's matches or a diagnostic. An overlay is, with
//! virtual text, a position the buffer keeps and the two doors move; the
//! table is [`crate::buffer::overlay`].
//!
//! Design: `doc/architettura-editor.typ, <mark-overlay>`.
use super::*;
use crate::ui::Face;
use std::sync::Arc;

/// The category an overlay made without one belongs to.
const DEFAULT_CATEGORY: &str = "overlay";

/// Returns EXP as a buffer position, which must be a number not below 0.
fn position<B: BufferTrait>(exp: &ELispExp<B>) -> Result<usize, EvalError<EditorState<B>>> {
    match exp {
        ELispExp::Number(n) if n.is_finite() && *n >= 0.0 => Ok(*n as usize),
        other => Err(EvalError::WrongArgumentType {
            expected: "a non-negative Number".into(),
            got: other.clone(),
        }),
    }
}

pub const MAKE_OVERLAY_DOC: &str = "(make-overlay START END FACE &optional PRIORITY CATEGORY): \
         Draw the text between START and END of the current buffer in FACE, and return a handle \
         to the overlay, or nil when the span is empty. Of two that overlap, the one with the \
         higher PRIORITY shows, 0 unless given. CATEGORY is a symbol naming whatever made it, \
         `overlay' unless given, for `remove-overlays'.\n\n\
         Example:\n\
         (make-overlay (point) (+ (point) 10) 'error 5 'my-marks)";

primitive!(make_overlay, args, _env, ctx, {
    if args.len() < 3 || args.len() > 5 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }
    let start = position(&args[0])?;
    let end = position(&args[1])?;
    let face = match &args[2] {
        ELispExp::Symbol(name) | ELispExp::String(name) => Face::intern(name),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol naming a face".into(),
                got: other.clone(),
            });
        }
    };
    let priority = match args.get(3) {
        None => 0,
        Some(exp) if exp.is_nil() => 0,
        Some(ELispExp::Number(n)) if n.is_finite() => *n as i32,
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Number".into(),
                got: other.clone(),
            });
        }
    };
    let category: Arc<str> = match args.get(4) {
        None => Arc::from(DEFAULT_CATEGORY),
        Some(exp) if exp.is_nil() => Arc::from(DEFAULT_CATEGORY),
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) => Arc::from(name.as_str()),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol naming a category".into(),
                got: other.clone(),
            });
        }
    };
    // Clamped to the buffer, so a caller working from stale offsets marks
    // something wrong rather than nothing at all -- and never marks past the
    // end, where there is no text to draw over.
    let made = ctx.with_current_buffer_mut(|buf| {
        let limit = buf.text.len();
        buf.overlays
            .add(start.min(limit), end.min(limit), face, priority, category)
    });
    Ok(match made {
        Some(id) => ELispExp::number(id as f64),
        None => ELispExp::nil(),
    })
});

pub const DELETE_OVERLAY_DOC: &str = "(delete-overlay HANDLE): Remove the overlay HANDLE names \
         from the current buffer. Return t, or nil when there is none: an overlay goes by itself \
         when all its text is deleted, and a handle is never used again.";

primitive!(delete_overlay, args, _env, ctx, {
    let id = match args.first() {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => *n as usize,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "a handle from `make-overlay'".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    let removed = ctx.with_current_buffer_mut(|buf| buf.overlays.remove(id));
    Ok(if removed {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

pub const REMOVE_OVERLAYS_DOC: &str = "(remove-overlays &optional CATEGORY): Remove the overlays \
         of CATEGORY from the current buffer, or all of them, and return how many went.\n\n\
         Example:\n\
         (remove-overlays 'my-marks)";

primitive!(remove_overlays, args, _env, ctx, {
    let category = args::optional_name(args.first(), "Symbol naming a category")?;
    let removed =
        ctx.with_current_buffer_mut(|buf| buf.overlays.remove_category(category.as_deref()));
    Ok(ELispExp::number(removed as f64))
});

pub const OVERLAYS_AT_DOC: &str = "(overlays-at POSITION): Return the handles of the overlays \
         that cover POSITION in the current buffer, lowest priority first. An overlay covers its \
         start and not its end.\n\n\
         Example:\n\
         (overlays-at (point))";

primitive!(overlays_at, args, _env, ctx, {
    let position = position(args.first().unwrap_or(&ELispExp::Number(0.0)))?;
    let found = ctx.with_current_buffer(|buf| {
        buf.overlays
            .at(position)
            .into_iter()
            .map(|overlay| ELispExp::number(overlay.id as f64))
            .collect::<Vec<_>>()
    });
    Ok(ELispExp::proper_list(found))
});

pub const OVERLAY_FACE_DOC: &str = "(overlay-face HANDLE): Return the face the overlay HANDLE \
         names is drawn in, as a symbol, or nil when there is no such overlay.\n\n\
         Example:\n\
         (mapcar 'overlay-face (overlays-at (point)))";

primitive!(overlay_face, args, _env, ctx, {
    let id = match args.first() {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => *n as usize,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "a handle from `make-overlay'".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    let face = ctx.with_current_buffer(|buf| {
        buf.overlays
            .iter()
            .find(|overlay| overlay.id == id)
            .map(|overlay| overlay.face.name())
    });
    Ok(match face {
        Some(name) => ELispExp::symbol(name.to_string()),
        None => ELispExp::nil(),
    })
});

/// Registers the primitives of overlays.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("make-overlay", make_overlay, MAKE_OVERLAY_DOC);
    into.function("delete-overlay", delete_overlay, DELETE_OVERLAY_DOC);
    into.function("remove-overlays", remove_overlays, REMOVE_OVERLAYS_DOC);
    into.function("overlays-at", overlays_at, OVERLAYS_AT_DOC);
    into.function("overlay-face", overlay_face, OVERLAY_FACE_DOC);
}
