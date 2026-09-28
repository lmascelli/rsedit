//! What the editor can be asked about itself: what a key does, where a
//! command is bound, what arguments it collects.
//!
//! # The split
//!
//! Everything here answers a question and draws nothing. `describe-key` is a
//! page of text in a buffer, and that lives in `help.lisp`, where it can be
//! replaced by a window with buttons without any of the answers changing.
//!
//! The answers themselves cannot be replaced, and are in Rust for the same
//! reason `query-replace`'s loop is: a help command that a missing module
//! could quietly make wrong is worse than no help command, because it is
//! believed.
//!
//! # Why none of this re-reads the keymaps itself
//!
//! Which map wins is a rule of the editor, written once in [`Modes`] and run
//! on every keystroke. Everything below asks *that*, through the facade, so a
//! reported binding and a pressed one cannot disagree -- see
//! [`crate::editor::keys`], where the capture that `describe-key` is built on
//! also diverts at the end of the real resolution rather than in front of it.
use super::*;
use crate::editor::KEY_CAPTURE_FUNCTION;
use crate::managers::BindingSource;

/// A binding as Lisp sees one: (KEYS TARGET SOURCE).
fn binding_form<B: BufferTrait>(
    keys: String,
    target: ELispExp<B>,
    source: BindingSource,
) -> ELispExp<B> {
    ELispExp::proper_list(vec![
        ELispExp::string(keys),
        target,
        ELispExp::string(source.name().to_string()),
    ])
}

fn key_argument<B: BufferTrait>(args: &[ELispExp<B>]) -> Option<Vec<KeyEvent>> {
    match args.first() {
        Some(ELispExp::String(text)) => parse_key_sequence(text),
        _ => None,
    }
}

pub const KEY_BINDING_DOC: &str = "(key-binding KEYS): What pressing KEYS would run in the \
         current buffer, as (KEYS TARGET SOURCE), or nil when it would run nothing.\n\n\
         SOURCE is \"transient\", \"mode\" or \"global\" -- which map the binding came from, and \
         so whether it is in force everywhere or only here.\n\n\
         nil for a sequence that is only the *start* of a binding: `C-x' runs nothing on its \
         own. Ask `keys-with-prefix' to tell a prefix from a key bound to nothing at all.\n\n\
         The answer comes from the same walk key resolution does, in the buffer as it is now, so \
         it is what would happen rather than what a keymap says in isolation.\n\n\
         Example:\n\
         (key-binding \"C-x C-f\") => (\"C-x C-f\" (find-file) \"global\")\n\
         (key-binding \"C-x\") => nil";

primitive!(key_binding, args, _env, ctx, {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let Some(keys) = key_argument(args) else {
        ctx.log_diagnostic(&format!("key-binding: bad key sequence {:?}", args[0]));
        return Ok(ELispExp::nil());
    };
    Ok(match ctx.key_binding(&keys) {
        Some(binding) => binding_form(binding.described(), binding.target, binding.source),
        None => ELispExp::nil(),
    })
});

pub const KEYS_WITH_PREFIX_DOC: &str = "(keys-with-prefix KEYS): Every binding in effect that \
         begins with KEYS and is longer, as a list of (KEYS TARGET SOURCE), sorted.\n\n\
         What continues a half-typed sequence, and so what a `C-x' is *for*. An empty list means \
         KEYS leads nowhere, which -- together with `key-binding' answering nil -- is how a \
         prefix is told apart from a key bound to nothing.\n\n\
         Called with the empty string it lists everything, which is `keymap-bindings'.\n\n\
         Example:\n\
         (keys-with-prefix \"C-x\") => ((\"C-x C-f\" (find-file) \"global\") ...)";

primitive!(keys_with_prefix, args, _env, ctx, {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
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

pub const KEYMAP_BINDINGS_DOC: &str = "(keymap-bindings): Every binding in effect in the current \
         buffer, as a list of (KEYS TARGET SOURCE), sorted by key sequence.\n\n\
         Each sequence appears once, under the map that wins it: a key the mode rebinds is \
         listed as the mode's, and the global binding it hides is not listed at all. What is \
         wanted here is what the keys do, and a binding that cannot happen is not that.\n\n\
         Example:\n\
         (length (keymap-bindings)) => 214";

primitive!(keymap_bindings, _args, _env, ctx, {
    Ok(ELispExp::proper_list(
        ctx.key_bindings()
            .into_iter()
            .map(|binding| binding_form(binding.described(), binding.target, binding.source))
            .collect(),
    ))
});

pub const WHERE_IS_DOC: &str = "(where-is COMMAND): Every key sequence that runs COMMAND in the \
         current buffer, as a sorted list of strings. Empty when it has no binding.\n\n\
         A binding counts when it names COMMAND and supplies no arguments of its own -- `\\'quit' \
         or `(quit)'. A binding that hands the command an argument, as every self-inserting key \
         does, is a different offer and is not listed: what `where-is' is for is telling somebody \
         which keys to press to get what `M-x COMMAND' would give them.\n\n\
         Example:\n\
         (where-is 'find-file) => (\"C-x C-f\")";

primitive!(where_is, args, _env, ctx, {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    let name = match &args[0] {
        ELispExp::Symbol(name) | ELispExp::String(name) => name.to_string(),
        other => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol".into(),
                got: other.clone(),
            });
        }
    };
    // A binding is stored as whatever `define-key` was handed: a bare
    // symbol, a source form, or the *data* list a quoted `'(quit)` evaluates
    // to -- which is a cons list and not a `Form` at all. All three mean the
    // same thing to key resolution, so all three have to mean the same thing
    // here; matching only one shape is how a `where-is` comes to answer
    // "nowhere" about a key somebody is looking at.
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

pub const COMMAND_SPECS_DOC: &str = "(command-specs NAME): The argument specs NAME was \
         registered with, as a list of code strings, or nil when it takes none.\n\n\
         The codes are the ones `defcommand' was written with -- \"sPrompt: \" and the rest -- so \
         what comes back can be read straight back into one.\n\n\
         nil also for a name that is no command at all; `commandp' is the question that tells \
         those apart.\n\n\
         Example:\n\
         (command-specs \"find-file\") => (\"fFind file: \")\n\
         (command-specs \"next-line\") => nil";

primitive!(command_specs, args, _env, ctx, {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
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

pub const READ_KEY_SEQUENCE_DOC: &str = "(read-key-sequence FUNCTION): Have the next key \
         sequence handed to FUNCTION instead of being run. Returns t.\n\n\
         FUNCTION is called as (FUNCTION KEYS TARGET SOURCE): KEYS spelt the way a binding is \
         written, TARGET the form that key would have run -- quoted, so it can be shown rather \
         than done -- and SOURCE the map it came from (\"transient\", \"mode\", \"global\"), or \
         \"prefix-argument\" for `C-u', or nil when the keys are bound to nothing.\n\n\
         A prefix is read to the end: asking about `C-x' waits for the key after it, exactly as \
         the editor does, and the half-typed sequence shows in the echo area meanwhile.\n\n\
         What makes this truthful is where it diverts. The keys are resolved by the editor's own \
         resolution -- this buffer, this mode, the transient map first -- and only the *running* \
         is replaced. A reader that walked the keymaps itself would be a second copy of the \
         precedence rules, and would go on reporting the old ones after they changed.\n\n\
         Reading is armed by setting `*key-capture-function*', which is also how it is cancelled: \
         (setq *key-capture-function* nil).\n\n\
         Example:\n\
         (read-key-sequence (lambda (keys target source)\n\
                              (message \"%s runs %s (%s)\" keys target source)))";

primitive!(read_key_sequence, args, env, ctx, {
    if args.len() != 1 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 1,
            got: args.len(),
        });
    }
    if args[0].is_nil() {
        return Err(EvalError::WrongArgumentType {
            expected: "a function to call with the key sequence".into(),
            got: args[0].clone(),
        });
    }
    let _ = ctx;
    env.set_variable(KEY_CAPTURE_FUNCTION.into(), args[0].clone());
    Ok(ELispExp::t())
});
