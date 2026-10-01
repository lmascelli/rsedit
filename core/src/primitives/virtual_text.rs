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
         CATEGORY): Show TEXT at POSITION without putting it in the buffer, and return a handle \
         to it. Returns nil for empty TEXT.\n\n\
         The text is drawn in front of the character at POSITION, pushing the rest of the line \
         to the right. It is not in the buffer: nothing is modified, nothing is added to the \
         undo history, and saving writes the file as it was. Point cannot be put inside it, and \
         moving across the position it sits at steps over it in one go -- it is not text.\n\n\
         To put something at the end of a line, give the position of that line's newline. \
         `(line-end-position)' is that.\n\n\
         FACE is how it is drawn, defaulting to `comment' -- which is the right default because \
         the one thing the reader must never have to wonder is whether what they are looking at \
         is in the file. PRIORITY orders two at the same position, higher first; equal \
         priorities are drawn oldest first.\n\n\
         CATEGORY is a symbol naming whatever made it, and is how a producer replaces its own \
         batch later -- see `clear-virtual-text'. It defaults to `virtual-text'. The editor \
         attaches no meaning to the name: a language server's hints, a preview and a note are \
         all just categories, and one that the editor understood would be one it had to be \
         taught about the next kind of.\n\n\
         **It moves with the text.** Typing before it carries it along; deleting the text it \
         sits in front of leaves it where that text began. It is never dropped, unlike an \
         overlay, which goes when everything it covered does -- a position has no width to \
         lose. A producer whose text has ended up somewhere wrong says so by clearing its \
         category and placing it again.\n\n\
         Example:\n\
         (make-virtual-text (point) \": int\" 'comment 0 'lsp-hints)";

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
    let category = match args.get(4) {
        None => DEFAULT_CATEGORY.to_string(),
        Some(exp) if exp.is_nil() => DEFAULT_CATEGORY.to_string(),
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) => name.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol naming a category".into(),
                got: other.clone(),
            });
        }
    };
    let id = ctx.with_current_buffer_mut(|buf| {
        // Clamped to the buffer, so a caller working from stale offsets puts
        // its text at the end rather than nowhere.
        let at = at.min(buf.text.len());
        buf.virtual_text
            .insert(at, text.as_str(), face, priority, &category)
    });
    Ok(ELispExp::number(id as f64))
});

pub const CLEAR_VIRTUAL_TEXT_DOC: &str = "(clear-virtual-text &optional CATEGORY): Remove virtual \
         text from the current buffer -- everything in CATEGORY, or all of it when CATEGORY is \
         omitted. Returns how many entries went.\n\n\
         The useful half of the interface, and the reason a category exists. A producer that has \
         new answers -- a language server that has re-analysed the file, a preview following \
         what is being typed -- clears its own batch and places the new one, without keeping a \
         handle for each entry and without disturbing anybody else's.\n\n\
         That still works after the module that made them has been reloaded, which loose handles \
         do not.\n\n\
         Example:\n\
         (clear-virtual-text 'lsp-hints)";

primitive!(clear_virtual_text, args, _env, ctx, {
    let category = match args.first() {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) => Some(name.to_string()),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol naming a category, or nil".into(),
                got: other.clone(),
            });
        }
    };
    let gone = ctx
        .with_current_buffer_mut(|buf| buf.virtual_text.clear(category.as_deref()));
    Ok(ELispExp::number(gone as f64))
});

pub const DELETE_VIRTUAL_TEXT_DOC: &str = "(delete-virtual-text HANDLE): Remove the one entry \
         HANDLE names. Returns t if there was one, nil otherwise.\n\n\
         For a producer with exactly one thing to take back. A producer with a set of them wants \
         `clear-virtual-text' and a category instead, which is one call rather than a list of \
         handles to keep correct.";

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
    Ok(ELispExp::boolean(
        ctx.with_current_buffer_mut(|buf| buf.virtual_text.remove(id)),
    ))
});

pub const VIRTUAL_TEXT_AT_DOC: &str = "(virtual-text-at POSITION): The virtual text shown at \
         POSITION, as a list of (TEXT FACE CATEGORY HANDLE) in the order it is drawn. Empty when \
         there is none.\n\n\
         For a module that wants to say something about what is shown there -- or for a test, \
         which cannot read the screen.";

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
