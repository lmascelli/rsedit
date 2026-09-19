//! Putting a face on a particular span of text.
//!
//! A mode's colouring is a set of patterns: it can say "a word in this list is
//! a keyword" and cannot say "*this* span, because of something that happened".
//! A search marking its matches, a diagnostic arriving from elsewhere, a manual
//! page whose emphasis was in the bytes rather than in the words -- none of
//! those are expressible as a rule over the text.
//!
//! See [`crate::buffer::overlay`] for what makes them work: they are the only
//! stored positions in the editor, and the two doors move them.
use super::*;
use crate::ui::Face;
use std::sync::Arc;

/// The category an overlay made without one belongs to.
const DEFAULT_CATEGORY: &str = "overlay";

/// Read an optional buffer position argument.
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
         Draw the text between START and END in FACE, and return a handle to it. Returns nil if \
         the span is empty.\n\n\
         An overlay says something about one particular span, which a syntax rule cannot: rules \
         are patterns over the text, and \"this match, here\" is not a pattern. It is drawn over \
         the mode's colouring and under the region, so a diagnostic is visible on a keyword and \
         a selection is visible over everything.\n\n\
         PRIORITY orders overlaps, higher winning; equal priorities are drawn oldest first. \
         CATEGORY is a symbol naming whatever made it, and is how a producer replaces its own \
         batch later -- see `remove-overlays'. It defaults to `overlay'.\n\n\
         **Overlays move with the text.** Typing before one carries it along; typing at either \
         end leaves the new text outside it, so a highlighted match stays exactly the text that \
         matched; typing inside it is absorbed. Deleting everything an overlay covered removes \
         it, and undoing that deletion does not bring it back -- whatever made it will make it \
         again if it still matters.\n\n\
         Example:\n\
         (make-overlay 10 20 'error 5 'diagnostics)";

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
    let made = ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        let limit = buf.text.len();
        buf.overlays
            .add(start.min(limit), end.min(limit), face, priority, category)
    });
    Ok(match made {
        Some(id) => ELispExp::number(id as f64),
        None => ELispExp::nil(),
    })
});

pub const DELETE_OVERLAY_DOC: &str = "(delete-overlay ID): Remove the overlay with handle ID from \
         the current buffer. Returns t if there was one, nil otherwise.\n\n\
         nil rather than an error, because an overlay may have gone on its own: deleting all the \
         text it covered removes it. A handle is never reused, so a stale one names nothing \
         rather than naming whatever took its place.\n\n\
         To remove a whole batch at once -- every match of the last search, say -- use \
         `remove-overlays' with the category instead.\n\n\
         Example:\n\
         (delete-overlay my-overlay)";

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
    let removed = ctx.mutate_buffer(ctx.get_current_buffer(), |buf| buf.overlays.remove(id));
    Ok(if removed {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

pub const REMOVE_OVERLAYS_DOC: &str = "(remove-overlays &optional CATEGORY): Remove every overlay \
         in CATEGORY from the current buffer, or all of them when CATEGORY is omitted. Returns \
         how many went.\n\n\
         This is what a producer actually wants. Re-running a search means \"forget every match I \
         marked last time\", which is one call rather than a list of handles to keep -- and it \
         still works after the module that made them has been reloaded, which loose handles do \
         not survive.\n\n\
         Example:\n\
         (remove-overlays 'isearch)\n\
         (remove-overlays)            ; every overlay in this buffer";

primitive!(remove_overlays, args, _env, ctx, {
    let category = match args.first() {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) => Some(name.to_string()),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol naming a category".into(),
                got: other.clone(),
            });
        }
    };
    let removed = ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        buf.overlays.remove_category(category.as_deref())
    });
    Ok(ELispExp::number(removed as f64))
});

pub const OVERLAYS_AT_DOC: &str = "(overlays-at POSITION): The handles of the overlays covering \
         POSITION in the current buffer, lowest priority first.\n\n\
         An overlay covers its START and not its END, the way a region does, so the two ends of \
         adjoining overlays do not both claim the same character.\n\n\
         Mostly for finding out what is there -- a test, or a command reporting the diagnostic \
         under the cursor.\n\n\
         Example:\n\
         (overlays-at (point))";

primitive!(overlays_at, args, _env, ctx, {
    let position = position(args.first().unwrap_or(&ELispExp::Number(0.0)))?;
    let found = ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
        buf.overlays
            .at(position)
            .into_iter()
            .map(|overlay| ELispExp::number(overlay.id as f64))
            .collect::<Vec<_>>()
    });
    Ok(ELispExp::proper_list(found))
});

pub const OVERLAY_FACE_DOC: &str = "(overlay-face ID): The face the overlay with handle ID is \
         drawn in, as a symbol, or nil if there is no such overlay.\n\n\
         Example:\n\
         (overlay-face (car (overlays-at (point))))";

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
    let face = ctx.mutate_buffer(ctx.get_current_buffer(), |buf| {
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
