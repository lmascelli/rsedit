//! Asking the user a question whose answer decides what happens next. A
//! question returns at once; the answer arrives later, as a call to the
//! callback it was given.
//!
//! Design: `doc/architettura-editor.typ, <domande>`.
use super::*;
use crate::input::{Keymap, OnUnbound, TransientKeymap};
use crate::primitives::args;
use risp::{Lambda, call_callable, eval};

fn nil_or<B: BufferTrait>(value: Option<&ELispExp<B>>) -> ELispExp<B> {
    value.cloned().unwrap_or_else(ELispExp::nil)
}

/// Returns the call that answers a one-key question: it takes the map down,
/// then runs CALLBACK.
fn answer_form<B: BufferTrait>(callback: &ELispExp<B>) -> ELispExp<B> {
    ELispExp::form(vec![
        ELispExp::symbol("ask--answer".into()),
        args::quoted(callback.clone()),
    ])
}

/// Puts a question on screen that only the listed keys can answer.
///
/// `OnUnbound::Refuse` is what makes it a question rather than an offer: a key
/// that is not an answer does nothing at all, and says nothing, so the
/// question stays up and the message under it is not overwritten by a
/// complaint about the key.
pub(super) fn ask_with_keys<B: BufferTrait>(
    ctx: &EditorState<B>,
    message: String,
    keys: Vec<(&str, ELispExp<B>)>,
) {
    let mut keymap = Keymap::new();
    for (key, form) in keys {
        if let Some(event) = super::parse_key(key) {
            keymap.insert_key(event, form);
        }
    }
    ctx.set_transient_keymap(TransientKeymap {
        keymap,
        on_unbound: OnUnbound::Refuse,
        message,
    });
}

pub const Y_OR_N_DOC: &str = "(y-or-n PROMPT ON-YES &optional ON-NO): Ask PROMPT, to be answered \
         with y or n, and return at once; ON-YES or ON-NO is called, with no arguments, when the \
         key comes. No other key answers.\n\n\
         Example:\n\
         (y-or-n \"Delete this line?\" 'kill-whole-line)";

primitive!(y_or_n, args, _env, ctx, {
    let prompt = args::text(args.first())?;
    let on_yes = nil_or(args.get(1));
    let on_no = nil_or(args.get(2));
    ask_with_keys(
        ctx,
        format!("{prompt} (y or n)"),
        vec![("y", answer_form(&on_yes)), ("n", answer_form(&on_no))],
    );
    Ok(ELispExp::t())
});

pub const YES_OR_NO_DOC: &str = "(yes-or-no PROMPT ON-YES &optional ON-NO): Ask PROMPT, to be \
         answered by typing yes or no and Return, and return at once; ON-YES or ON-NO is called, \
         with no arguments, when the answer comes. Anything else asks again, and closing the \
         question counts as no.\n\n\
         Example:\n\
         (yes-or-no \"Discard your changes?\" 'revert-buffer)";

primitive!(yes_or_no, args, env, ctx, {
    let prompt = args::text(args.first())?;
    let on_yes = nil_or(args.get(1));
    let on_no = nil_or(args.get(2));

    // A one-argument lambda rather than a form, because the minibuffer calls
    // its confirm callback *with* what was typed -- so this has to be
    // something callable, and a form is not.
    let confirm = ELispExp::lambda(Lambda {
        params: vec!["answer".to_string()],
        optionals: Vec::new(),
        rest: None,
        body: vec![ELispExp::form(vec![
            ELispExp::symbol("ask--typed".into()),
            ELispExp::symbol("answer".into()),
            args::quoted(ELispExp::string(prompt.clone())),
            args::quoted(on_yes.clone()),
            args::quoted(on_no.clone()),
        ])],
        env: env.clone(),
        doc: None,
    });

    let read = ELispExp::form(vec![
        ELispExp::symbol("minibuffer-read".into()),
        ELispExp::string(format!("{prompt} (yes or no)")),
        args::quoted(confirm.clone()),
        ELispExp::nil(),
        // Cancelling is not an answer, and treating it as one would be the
        // wrong one: `no' is the safe reading of "I did not mean to be here".
        args::quoted(on_no.clone()),
    ]);
    eval(&read, env.clone(), ctx)
});

const ASK_ANSWER_DOC: &str = "(ask--answer CALLBACK): Take the question's keymap down and run \
         CALLBACK. Not called directly -- it is what `y-or-n' binds its keys to.";

primitive!(ask_answer, args, env, ctx, {
    // First, so that the keymap is gone before the callback runs: a callback
    // that asks the next question installs a map of its own, and taking one
    // down afterwards would take that one down with it.
    ctx.clear_transient_keymap();
    let callback = nil_or(args.first());
    if callback.is_truthy() {
        call_callable(&callback, &[], env.clone(), ctx)?;
    }
    Ok(ELispExp::nil())
});

const ASK_TYPED_DOC: &str = "(ask--typed ANSWER PROMPT ON-YES ON-NO): Dispatch a typed yes-or-no \
         answer, asking again if it was neither. Not called directly -- it is what `yes-or-no' \
         hands the minibuffer.";

primitive!(ask_typed, args, env, ctx, {
    let answer = match args.first() {
        Some(ELispExp::String(text)) => text.trim().to_lowercase(),
        _ => String::new(),
    };
    let prompt = nil_or(args.get(1));
    let on_yes = nil_or(args.get(2));
    let on_no = nil_or(args.get(3));

    let chosen = match answer.as_str() {
        "yes" => &on_yes,
        "no" => &on_no,
        _ => {
            // Asked again rather than defaulted. A question worth spelling the
            // answer to is one where guessing which was meant is the whole
            // thing being avoided.
            ctx.set_echo_message("Please answer `yes' or `no'.");
            return eval(
                &ELispExp::form(vec![
                    ELispExp::symbol("yes-or-no".into()),
                    args::quoted(prompt.clone()),
                    args::quoted(on_yes.clone()),
                    args::quoted(on_no.clone()),
                ]),
                env.clone(),
                ctx,
            );
        }
    };
    if chosen.is_truthy() {
        call_callable(chosen, &[], env.clone(), ctx)?;
    }
    Ok(ELispExp::nil())
});

/// Registers the primitives that ask a question.
///
/// Called by [`super::install_primitives`].
///
/// None of these is a command: a question is asked *by* something, and there
/// is nothing for `M-x yes-or-no` to ask about.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("y-or-n", y_or_n, Y_OR_N_DOC);
    into.function("yes-or-no", yes_or_no, YES_OR_NO_DOC);
    into.function("ask--answer", ask_answer, ASK_ANSWER_DOC);
    into.function("ask--typed", ask_typed, ASK_TYPED_DOC);
}
