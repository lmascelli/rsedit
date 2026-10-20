//! Showing text in a window that is not in the buffer.
//!
//! The table is in [`crate::buffer::virtual_text`] and the drawing is in
//! [`crate::ui::layout`]; this is the door Lisp comes through.
//!
//! Nothing here knows what the text is *for*. An inlay hint from a language
//! server, a preview of what a command would insert, a note beside a line --
//! all three are a string at a position with a category naming whoever put it
//! there, and the editor's whole involvement is letting that producer replace
//! its own batch in one call.
use super::*;
use crate::ui::Face;

/// The category virtual text made without one belongs to.
const DEFAULT_CATEGORY: &str = "virtual-text";

pub const MAKE_VIRTUAL_TEXT_DOC: &str = "(make-virtual-text POSITION TEXT &optional FACE PRIORITY \
         CATEGORY): Show TEXT in front of the character at POSITION in the current buffer, \
         without putting it in the buffer, and return a handle to it, or nil when TEXT is empty. \
         FACE is `comment' unless given. PRIORITY orders several at one position, highest first, \
         0 unless given. CATEGORY is a symbol naming whatever made it, `virtual-text' unless \
         given, for `clear-virtual-text'.\n\n\
         Example:\n\
         (make-virtual-text (point) \": int\" nil 0 'my-hints)";

primitive!(make_virtual_text, args, _env, ctx, {
    if args.len() < 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let at = match &args[0] {
        ELispExp::Number(n) if n.is_finite() && *n >= 0.0 => *n as usize,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "a non-negative Number".into(),
                got: other.clone(),
            });
        }
    };
    let ELispExp::String(text) = &args[1] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[1].clone(),
        });
    };
    // Nothing, rather than an entry that draws nothing: a zero-width piece
    // would be a row the composer walked past for no reason, on every frame.
    if text.is_empty() {
        return Ok(ELispExp::nil());
    }
    let face = match args.get(2) {
        None => Face::intern("comment"),
        Some(exp) if exp.is_nil() => Face::intern("comment"),
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) => Face::intern(name),
        Some(other) => {
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
    let category = args::optional_name(args.get(4), "Symbol naming a category")?
        .unwrap_or_else(|| DEFAULT_CATEGORY.to_string());
    let id = ctx.with_current_buffer_mut(|buf| {
        // Clamped to the buffer, so a caller working from stale offsets puts
        // its text at the end rather than nowhere.
        let at = at.min(buf.text.len());
        buf.virtual_text
            .insert(at, text.as_str(), face, priority, &category)
    });
    Ok(ELispExp::number(id as f64))
});

pub const CLEAR_VIRTUAL_TEXT_DOC: &str = "(clear-virtual-text &optional CATEGORY): Remove the \
         virtual text of CATEGORY from the current buffer, or all of it, and return how many \
         went.\n\n\
         Example:\n\
         (clear-virtual-text 'my-hints)";

primitive!(clear_virtual_text, args, _env, ctx, {
    let category = args::optional_name(args.first(), "Symbol naming a category, or nil")?;
    let gone = ctx.with_current_buffer_mut(|buf| buf.virtual_text.clear(category.as_deref()));
    Ok(ELispExp::number(gone as f64))
});

pub const DELETE_VIRTUAL_TEXT_DOC: &str = "(delete-virtual-text HANDLE): Remove the one piece of \
         virtual text HANDLE names from the current buffer. Return t, or nil when there is none.";

primitive!(delete_virtual_text, args, _env, ctx, {
    let ELispExp::Number(id) = args.first().unwrap_or(&ELispExp::Number(-1.0)) else {
        return Err(EvalError::WrongArgumentType {
            expected: "a handle from make-virtual-text".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    if !id.is_finite() || *id < 0.0 {
        return Ok(ELispExp::nil());
    }
    let id = *id as usize;
    Ok(ELispExp::boolean(ctx.with_current_buffer_mut(|buf| {
        buf.virtual_text.remove(id)
    })))
});

pub const VIRTUAL_TEXT_AT_DOC: &str = "(virtual-text-at POSITION): Return the virtual text shown \
         at POSITION in the current buffer, as a list of (TEXT FACE CATEGORY HANDLE) in the order \
         it is drawn.\n\n\
         Example:\n\
         (virtual-text-at (point))";

primitive!(virtual_text_at, args, _env, ctx, {
    let at = match args.first() {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 0.0 => *n as usize,
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "a non-negative Number".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    Ok(ctx.with_current_buffer(|buf| {
        ELispExp::proper_list(
            buf.virtual_text
                .at(at)
                .map(|entry| {
                    ELispExp::proper_list(vec![
                        ELispExp::string(entry.text.to_string()),
                        ELispExp::symbol(entry.face.name().to_string()),
                        ELispExp::symbol(entry.category.to_string()),
                        ELispExp::number(entry.id as f64),
                    ])
                })
                .collect(),
        )
    }))
});

/// Register this module's primitives: text shown in the view that is not in the buffer.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Text shown in a window that is not in the buffer: a hint, a preview.
    // See `primitives::virtual_text`, and `ui::layout` for how a row that is
    // wider on screen than it is in the buffer is drawn and clicked on.
    into.function(
        "make-virtual-text",
        make_virtual_text,
        MAKE_VIRTUAL_TEXT_DOC,
    );
    into.function(
        "clear-virtual-text",
        clear_virtual_text,
        CLEAR_VIRTUAL_TEXT_DOC,
    );
    into.function(
        "delete-virtual-text",
        delete_virtual_text,
        DELETE_VIRTUAL_TEXT_DOC,
    );
    into.function("virtual-text-at", virtual_text_at, VIRTUAL_TEXT_AT_DOC);
}
