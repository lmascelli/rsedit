//! Binding faces to styles from Lisp.
use super::*;
use crate::ui::{Color, Face, Style};

/// Read a colour argument: a string Lisp writes colours in, or nil for "leave
/// the terminal's own choice alone".
fn colour_arg<B: BufferTrait>(
    arg: Option<&ELispExp<B>>,
) -> Result<Option<Color>, EvalError<EditorState<B>>> {
    match arg {
        None => Ok(None),
        Some(exp) if exp.is_nil() => Ok(None),
        // An empty string reads as "leave it alone" so that the interactive
        // form is usable: `M-x set-face` prompts for two colours, and pressing
        // Enter at a prompt has to mean "skip this one" rather than fail.
        Some(ELispExp::String(text)) if text.is_empty() => Ok(None),
        Some(ELispExp::String(text)) => Color::parse(text).map(Some).ok_or_else(|| {
            EvalError::RuntimeMessage(format!(
                "Not a colour: {text:?}. Use \"#rrggbb\", \"#rgb\", or a colour name like \
                 \"blue\" or \"bright-blue\""
            ))
        }),
        Some(other) => Err(EvalError::WrongArgumentType {
            expected: "String or nil".into(),
            got: other.clone(),
        }),
    }
}

/// Read the attribute list: a list of names of attributes to turn on.
fn attributes_arg<B: BufferTrait>(
    arg: Option<&ELispExp<B>>,
    style: &mut Style,
) -> Result<(), EvalError<EditorState<B>>> {
    let Some(exp) = arg else { return Ok(()) };
    if exp.is_nil() {
        return Ok(());
    }
    for item in exp.iter() {
        let name = match &item {
            ELispExp::String(name) => name.to_string(),
            ELispExp::Symbol(name) => name.to_string(),
            other => {
                return Err(EvalError::WrongArgumentType {
                    expected: "String or Symbol".into(),
                    got: other.clone(),
                });
            }
        };
        // Reported rather than ignored: a mistyped attribute that silently
        // does nothing is a theme that quietly renders wrong.
        if !style.set_attribute(&name, true) {
            return Err(EvalError::RuntimeMessage(format!(
                "Unknown face attribute: {name:?}. Known: bold, italic, underline, reverse"
            )));
        }
    }
    Ok(())
}

fn face_arg<B: BufferTrait>(arg: Option<&ELispExp<B>>) -> Result<Face, EvalError<EditorState<B>>> {
    let name = match arg {
        Some(ELispExp::String(name)) => name.to_string(),
        Some(ELispExp::Symbol(name)) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "String or Symbol naming a face".into(),
                got: other.cloned().unwrap_or_else(ELispExp::nil),
            });
        }
    };
    // Interned rather than looked up: naming a face is how one comes to exist,
    // now that the set is open. A misspelt name is therefore not an error --
    // it defines an inert face, which `list-faces` shows, so the mistake is
    // visible rather than silently falling back to the default.
    Ok(Face::intern(&name))
}

pub const SET_FACE_DOC: &str = "(set-face FACE &optional FOREGROUND BACKGROUND ATTRIBUTES): Draw \
         the face called FACE with FOREGROUND and BACKGROUND -- \"#rrggbb\", \"#rgb\" or a name \
         `list-colors' gives, and nil or \"\" for the terminal's own -- and ATTRIBUTES, a list of \
         \"bold\", \"italic\", \"underline\" and \"reverse\". What FACE was is replaced whole, \
         and a face no one has named is made. Return nil.\n\n\
         Example:\n\
         (set-face 'comment \"bright-black\" nil '(\"italic\"))";

primitive!(set_face, args, _env, ctx, {
    let face = face_arg(args.first())?;
    let mut style = Style {
        fg: colour_arg(args.get(1))?,
        bg: colour_arg(args.get(2))?,
        ..Style::plain()
    };
    attributes_arg(args.get(3), &mut style)?;
    ctx.set_face_style(face, style);
    Ok(ELispExp::nil())
});

pub const FACE_STYLE_DOC: &str = "(face-style FACE): Return how FACE is drawn now, as (FOREGROUND \
         BACKGROUND ATTRIBUTE...): each colour written \"#rrggbb\", or nil where the face leaves \
         it to the terminal, then the names of the attributes that are on.\n\n\
         Example:\n\
         (face-style 'region)";

primitive!(face_style, args, _env, ctx, {
    let style = ctx.face_style(face_arg(args.first())?);
    // Written back exactly as `set-face` would take it, so a theme can be read
    // out of a running editor and pasted into a config file.
    let colour = |c: Option<Color>| match c {
        Some(color) => ELispExp::string(color.to_text()),
        None => ELispExp::nil(),
    };
    let mut items = vec![colour(style.fg), colour(style.bg)];
    items.extend(
        style
            .attribute_names()
            .into_iter()
            .map(|name| ELispExp::string(name.to_string())),
    );
    Ok(ELispExp::proper_list(items))
});

pub const LIST_FACES_DOC: &str = "(list-faces): Return the names of every face, as strings: the \
         editor's own first, then those named since, by `set-face', `add-syntax-rule' or a theme.";

primitive!(list_faces, _args, _env, _ctx, {
    Ok(ELispExp::proper_list(
        Face::all()
            .into_iter()
            .map(|face| ELispExp::string(face.name().to_string()))
            .collect(),
    ))
});

pub const LIST_COLORS_DOC: &str = "(list-colors): Return the sixteen colour names `set-face' \
         takes, as strings. Any other colour is written \"#rrggbb\" or \"#rgb\".";

primitive!(list_colors, _args, _env, _ctx, {
    Ok(ELispExp::proper_list(
        crate::ui::NAMED_COLORS
            .iter()
            .map(|(name, _)| ELispExp::string(name.to_string()))
            .collect(),
    ))
});

pub const RESET_FACES_DOC: &str = "(reset-faces): Put every face back as the editor ships it: the \
         ones it styles itself as they were, and any other with no style at all. Return nil.";

primitive!(reset_faces, _args, _env, ctx, {
    ctx.reset_theme();
    Ok(ELispExp::nil())
});

/// Register this module's primitives: faces and the colours they are drawn in.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // ---------------------------------------------------------------
    // Faces and the theme
    // ---------------------------------------------------------------
    into.command(
        "set-face",
        set_face,
        &["s:Face: ", "s:Foreground: ", "s:Background: "],
        SET_FACE_DOC,
    );
    into.function("face-style", face_style, FACE_STYLE_DOC);
    into.function("reset-faces", reset_faces, RESET_FACES_DOC);
    into.function("list-faces", list_faces, LIST_FACES_DOC);
    into.function("list-colors", list_colors, LIST_COLORS_DOC);
}
