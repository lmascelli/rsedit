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
    Face::from_name(&name).ok_or_else(|| {
        let known: Vec<&str> = Face::ALL.iter().map(Face::name).collect();
        EvalError::RuntimeMessage(format!(
            "Unknown face: {name:?}. Known faces: {}",
            known.join(", ")
        ))
    })
}

pub const SET_FACE_DOC: &str = "(set-face FACE &optional FOREGROUND BACKGROUND ATTRIBUTES): Bind \
         FACE to a style, replacing whatever it was bound to.\n\n\
         FACE is a face name, as a string or symbol -- see `list-faces'. \
         FOREGROUND and BACKGROUND are colours: a name (\"red\", \
         \"bright-blue\") or an exact colour (\"#rrggbb\", \"#rgb\"). nil or \
         \"\" for either leaves that side alone. ATTRIBUTES is a list of \
         \"bold\", \"italic\", \"underline\" and \"reverse\".\n\n\
         A colour is a request, not an instruction: a display that cannot show \
         it draws the nearest thing it has, which on a sixteen-colour terminal \
         is the user's own palette. Names are shorthand for conventional \
         colours -- see `list-colors'.\n\n\
         Example:\n\
         (set-face 'region \"#f8f8f2\" \"#3a5fcd\")\n\
         (set-face 'region nil nil '(\"reverse\")) ; the default: no colour to clash\n\
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

pub const FACE_STYLE_DOC: &str = "(face-style FACE): Return how FACE is currently drawn, as a list \
         (FOREGROUND BACKGROUND . ATTRIBUTES) in the form `set-face' takes \
         back as a hex string, or nil when the face leaves that side \
         alone.\n\n\
         Example:\n\
         (face-style 'region) => (nil nil \"reverse\")";

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

pub const LIST_FACES_DOC: &str = "(list-faces): Return the names of every face the editor knows, as \
         a list of strings. These are the names `set-face' and \
         `add-syntax-rule' accept.";

primitive!(list_faces, _args, _env, _ctx, {
    Ok(ELispExp::proper_list(
        Face::ALL
            .iter()
            .map(|face| ELispExp::string(face.name().to_string()))
            .collect(),
    ))
});

pub const LIST_COLORS_DOC: &str = "(list-colors): Return the colour names `set-face' accepts, as a \
         list of strings. Each is shorthand for a conventional colour; any \
         other colour is written as \"#rrggbb\" or \"#rgb\".";

primitive!(list_colors, _args, _env, _ctx, {
    Ok(ELispExp::proper_list(
        crate::ui::NAMED_COLORS
            .iter()
            .map(|(name, _)| ELispExp::string(name.to_string()))
            .collect(),
    ))
});
