//! What the editor can be asked about itself: what a key runs, where a
//! command is bound, what arguments it collects, and the next key sequence,
//! read for a page to describe. The answers are the editor's own, and the
//! pages that show them are `help.lisp`.
//!
//! Design: `doc/architettura-editor.typ, <aiuto>`.
use super::*;
use crate::editor::KEY_CAPTURE_FUNCTION;
use crate::managers::BindingSource;

/// Returns a binding as Lisp sees it, `(KEYS TARGET SOURCE)`, with TARGET
/// turned from the syntax a keymap stores into data: a `Form` is no list to
/// `car`, and the command inside it could not be reached.
fn binding_form<B: BufferTrait>(
    keys: String,
    target: ELispExp<B>,
    source: BindingSource,
) -> ELispExp<B> {
    ELispExp::proper_list(vec![
        ELispExp::string(keys),
        risp::form_to_data(&target),
        ELispExp::string(source.name().to_string()),
    ])
}

fn key_argument<B: BufferTrait>(args: &[ELispExp<B>]) -> Option<Vec<KeyEvent>> {
    match args.first() {
        Some(ELispExp::String(text)) => parse_key_sequence(text),
        _ => None,
    }
}

pub const KEY_BINDING_DOC: &str = "(key-binding KEYS): Return what pressing KEYS would run in the \
         current buffer, as (KEYS TARGET SOURCE), or nil for keys bound to nothing or the start \
         of a longer sequence. SOURCE names the keymap: \"transient\", \"local\", \"mode\" or \
         \"global\". The answer comes from the lookup a key press makes.\n\n\
         Example:\n\
         (key-binding \"C-x C-f\")   ; (\"C-x C-f\" (find-file) \"global\")";

primitive!(key_binding, args, _env, ctx, {
    exact_arity(args, 1)?;
    let Some(keys) = key_argument(args) else {
        ctx.log_diagnostic(&format!("key-binding: bad key sequence {:?}", args[0]));
        return Ok(ELispExp::nil());
    };
    Ok(match ctx.key_binding(&keys) {
        Some(binding) => binding_form(binding.described(), binding.target, binding.source),
        None => ELispExp::nil(),
    })
});

pub const KEYS_WITH_PREFIX_DOC: &str = "(keys-with-prefix KEYS): Return every binding in effect \
         that begins with KEYS and is longer, sorted, each as (KEYS TARGET SOURCE). Empty when \
         KEYS begins none; the empty string gives every binding.\n\n\
         Example:\n\
         (keys-with-prefix \"C-x r\")";

primitive!(keys_with_prefix, args, _env, ctx, {
    exact_arity(args, 1)?;
    let ELispExp::String(text) = &args[0] else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args[0].clone(),
        });
    };
    let prefix = if text.trim().is_empty() {
        Vec::new()
    } else {
        match parse_key_sequence(text) {
            Some(keys) => keys,
            None => {
                ctx.log_diagnostic(&format!("keys-with-prefix: bad key sequence {text}"));
                return Ok(ELispExp::nil());
            }
        }
    };
    let found: Vec<ELispExp<B>> = ctx
        .key_bindings()
        .into_iter()
        .filter(|binding| binding.keys.len() > prefix.len() && binding.keys.starts_with(&prefix))
        .map(|binding| binding_form(binding.described(), binding.target, binding.source))
        .collect();
    Ok(ELispExp::proper_list(found))
});

pub const KEYMAP_BINDINGS_DOC: &str = "(keymap-bindings): Return every binding in effect in the \
         current buffer, sorted by key, each as (KEYS TARGET SOURCE) under the keymap that wins \
         it; a binding another keymap hides is left out.";

primitive!(keymap_bindings, _args, _env, ctx, {
    Ok(ELispExp::proper_list(
        ctx.key_bindings()
            .into_iter()
            .map(|binding| binding_form(binding.described(), binding.target, binding.source))
            .collect(),
    ))
});

pub const WHERE_IS_DOC: &str = "(where-is COMMAND): Return the keys that run COMMAND in the \
         current buffer, sorted, as strings; empty when none does. Only a binding that names \
         COMMAND and gives it no arguments of its own counts.\n\n\
         Example:\n\
         (where-is 'find-file)   ; (\"C-x C-f\")";

primitive!(where_is, args, _env, ctx, {
    exact_arity(args, 1)?;
    let name = match &args[0] {
        ELispExp::Symbol(name) | ELispExp::String(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: other.clone(),
            });
        }
    };
    // A binding is what `define-key` was handed: a symbol, a form, or the
    // list a quoted `'(quit)` evaluates to. Key resolution runs the three
    // alike, so here all three name the command.
    let names_command = |target: &ELispExp<B>| match target {
        ELispExp::Symbol(bound) => bound.as_str() == name,
        ELispExp::Form(items) => match items.as_slice() {
            [ELispExp::Symbol(bound)] => bound.as_str() == name,
            _ => false,
        },
        other if !other.is_nil() => {
            let items: Vec<ELispExp<B>> = other.iter().collect();
            match items.as_slice() {
                [ELispExp::Symbol(bound)] => bound.as_str() == name,
                _ => false,
            }
        }
        _ => false,
    };
    let found: Vec<ELispExp<B>> = ctx
        .key_bindings()
        .into_iter()
        .filter(|binding| names_command(&binding.target))
        .map(|binding| ELispExp::string(binding.described()))
        .collect();
    Ok(ELispExp::proper_list(found))
});

pub const COMMAND_SPECS_DOC: &str = "(command-specs NAME): Return the argument codes the command \
         NAME was registered with, as strings such as \"fFind file: \", or nil when it takes none \
         or is no command.\n\n\
         Example:\n\
         (command-specs \"find-file\")";

primitive!(command_specs, args, _env, ctx, {
    exact_arity(args, 1)?;
    let name = match &args[0] {
        ELispExp::Symbol(name) | ELispExp::String(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: other.clone(),
            });
        }
    };
    Ok(match ctx.command_specs(&name) {
        Some(specs) => ELispExp::proper_list(
            specs
                .into_iter()
                .map(|spec| ELispExp::string(spec.code()))
                .collect(),
        ),
        None => ELispExp::nil(),
    })
});

pub const READ_KEY_SEQUENCE_DOC: &str = "(read-key-sequence FUNCTION): Hand the next key sequence \
         to FUNCTION instead of running it, and return t. FUNCTION is called as (FUNCTION KEYS \
         TARGET SOURCE): KEYS written as a binding writes them, TARGET the form they would run, \
         quoted, and SOURCE the keymap as `key-binding' names it, \"prefix-argument\" for C-u, or \
         nil for keys bound to nothing. A prefix waits for the rest of its sequence. This sets \
         `*key-capture-function*', and setting that to nil cancels it.\n\n\
         Example:\n\
         (read-key-sequence (lambda (keys target source) (message \"%s runs %s\" keys target)))";

primitive!(read_key_sequence, args, env, _ctx, {
    exact_arity(args, 1)?;
    if args[0].is_nil() {
        return Err(EvalError::WrongArgumentType {
            expected: "a function to call with the key sequence".into(),
            got: args[0].clone(),
        });
    }
    // The root, not here: this is read on a *later* keystroke, and a binding
    // made in the calling function's scope is gone by then -- silently, with
    // the capture simply never happening.
    env.set_root_variable(KEY_CAPTURE_FUNCTION.into(), args[0].clone());
    Ok(ELispExp::t())
});

pub const HELP_MACRO_DOC_DOC: &str = "(help--macro-doc SYMBOL): Return (ARGLIST DOC) for the \
         macro SYMBOL names, its parameters as strings and its docstring or nil, or nil when it \
         names no macro.";

primitive!(help_macro_doc, args, env, _ctx, {
    exact_arity(args, 1)?;
    let name = match &args[0] {
        ELispExp::Symbol(name) | ELispExp::String(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: other.clone(),
            });
        }
    };
    let Some(ELispExp::Lambda(lambda)) = env.get_macro(&name) else {
        return Ok(ELispExp::nil());
    };
    let mut arglist: Vec<ELispExp<B>> = lambda
        .params
        .iter()
        .map(|param| ELispExp::string(param.clone()))
        .collect();
    if !lambda.optionals.is_empty() {
        arglist.push(ELispExp::string("&optional".into()));
        arglist.extend(
            lambda
                .optionals
                .iter()
                .map(|param| ELispExp::string(param.clone())),
        );
    }
    if let Some(rest) = &lambda.rest {
        arglist.push(ELispExp::string("&rest".into()));
        arglist.push(ELispExp::string(rest.clone()));
    }
    // `defmacro' leaves the docstring as the first form of the body, where
    // `defun' keeps it apart; a string followed by more body is one.
    let doc = match (&lambda.doc, lambda.body.first()) {
        (Some(doc), _) => Some(doc.to_string()),
        (None, Some(ELispExp::String(text))) if lambda.body.len() > 1 => Some(text.to_string()),
        _ => None,
    };
    Ok(ELispExp::proper_list(vec![
        ELispExp::proper_list(arglist),
        doc.map_or_else(ELispExp::nil, ELispExp::string),
    ]))
});

/// Registers the primitives the help commands are built on.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("key-binding", key_binding, KEY_BINDING_DOC);
    into.function("keys-with-prefix", keys_with_prefix, KEYS_WITH_PREFIX_DOC);
    into.function("keymap-bindings", keymap_bindings, KEYMAP_BINDINGS_DOC);
    into.function("where-is", where_is, WHERE_IS_DOC);
    into.function("command-specs", command_specs, COMMAND_SPECS_DOC);
    into.function("help--macro-doc", help_macro_doc, HELP_MACRO_DOC_DOC);
    into.function(
        "read-key-sequence",
        read_key_sequence,
        READ_KEY_SEQUENCE_DOC,
    );
}
