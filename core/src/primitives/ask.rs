//! Asking the user a question whose answer decides what happens next.
//!
//! # Why every one of these takes a callback
//!
//! The editor never blocks. A command runs to completion and returns; the
//! answer to a question arrives later, in another command, when the user
//! presses a key. So a question cannot *return* an answer -- there is nothing
//! to return it to by the time it exists. What it can do is say what to do
//! with each answer, which is what these take.
//!
//! In Lisp that reads as a `cond` split across two lambdas; in Rust it is a
//! `match` written as two calls. Neither is as pleasant as an answer that
//! comes back, and both are the price of an editor that stays responsive while
//! a question is on screen.
//!
//! # Why there are two strictnesses
//!
//! Because the cost of a mistaken answer is not the same everywhere. A
//! question whose wrong answer loses work should be hard to answer by
//! accident, and a question asked eight times in a row should not need eight
//! words typed at it. [`yes_or_no`] is the careful one and the default for
//! anything irreversible; [`y_or_n`] is for the questions you ask in a loop.
//!
//! # Why the callbacks are carried in the form rather than in a variable
//!
//! Each of these builds the *call* that will answer it -- `(ask--answer (quote
//! CALLBACK))` -- and hands that to a keymap or to the minibuffer. Nothing is
//! stored anywhere in the meantime, so a question asked from inside the answer
//! to another one is simply two forms, not two writes to one slot racing over
//! which question is live.
use super::*;
use crate::input::{Keymap, OnUnbound, TransientKeymap};
use crate::lisp::{Lambda, call_callable, eval};

/// VALUE, wrapped so that evaluating it gives the value back.
///
/// The callbacks these primitives are handed are already values -- a lambda, a
/// symbol naming a function -- and they have to survive being written into a
/// form that will later be evaluated. `quote` is exactly that: it hands back
/// its argument untouched.
fn quoted<B: BufferTrait>(value: &ELispExp<B>) -> ELispExp<B> {
    ELispExp::form(vec![ELispExp::symbol("quote".into()), value.clone()])
}

fn nil_or<B: BufferTrait>(value: Option<&ELispExp<B>>) -> ELispExp<B> {
    value.cloned().unwrap_or_else(ELispExp::nil)
}

fn prompt_arg<B: BufferTrait>(args: &[ELispExp<B>]) -> Result<String, EvalError<EditorState<B>>> {
    match args.first() {
        Some(ELispExp::String(text)) => Ok(text.to_string()),
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

/// The call that answers a one-key question: take the map down, then run
/// CALLBACK.
fn answer_form<B: BufferTrait>(callback: &ELispExp<B>) -> ELispExp<B> {
    ELispExp::form(vec![
        ELispExp::symbol("ask--answer".into()),
        quoted(callback),
    ])
}

/// Put a question on screen that only the listed keys can answer.
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

pub const Y_OR_N_DOC: &str = "(y-or-n PROMPT ON-YES &optional ON-NO): Ask PROMPT and wait for a \
         single `y' or `n'. Returns immediately; ON-YES or ON-NO is called, with no arguments, \
         when the key arrives.\n\n\
         Every other key does nothing and leaves the question up, so a mistyped key cannot \
         answer it -- but `y' is one keystroke, which is what makes this the right question to \
         ask in a loop. For anything whose wrong answer cannot be undone, use `yes-or-no'.\n\n\
         Example:\n\
         (y-or-n \"Delete this line?\" 'kill-whole-line)";

primitive!(y_or_n, args, _env, ctx, {
    let prompt = prompt_arg(args)?;
    let on_yes = nil_or(args.get(1));
    let on_no = nil_or(args.get(2));
    ask_with_keys(
        ctx,
        format!("{prompt} (y or n)"),
        vec![("y", answer_form(&on_yes)), ("n", answer_form(&on_no))],
    );
    Ok(ELispExp::t())
});

pub const YES_OR_NO_DOC: &str = "(yes-or-no PROMPT ON-YES &optional ON-NO): Ask PROMPT and wait \
         for the whole word `yes' or `no' to be typed and confirmed. Returns immediately; ON-YES \
         or ON-NO is called, with no arguments, when the answer arrives.\n\n\
         Anything else is refused and the question asked again, so the answer cannot be given by \
         a key pressed in passing. That deliberate slowness is the point: this is the question to \
         ask when the wrong answer loses something. Cancelling the prompt counts as `no', because \
         backing out of a question about an irreversible act means not doing it.\n\n\
         Example:\n\
         (yes-or-no \"Really discard your changes?\" 'revert-buffer)";

primitive!(yes_or_no, args, env, ctx, {
    let prompt = prompt_arg(args)?;
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
            quoted(&ELispExp::string(prompt.clone())),
            quoted(&on_yes),
            quoted(&on_no),
        ])],
        env: env.clone(),
        doc: None,
    });

    let read = ELispExp::form(vec![
        ELispExp::symbol("minibuffer-read".into()),
        ELispExp::string(format!("{prompt} (yes or no)")),
        quoted(&confirm),
        ELispExp::nil(),
        // Cancelling is not an answer, and treating it as one would be the
        // wrong one: `no' is the safe reading of "I did not mean to be here".
        quoted(&on_no),
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
                    quoted(&prompt),
                    quoted(&on_yes),
                    quoted(&on_no),
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

pub(super) fn install<B: BufferTrait>(env: &std::sync::Arc<Env<EditorState<B>>>) {
    env.set_function(
        "y-or-n".into(),
        ELispExp::primitive(y_or_n, Some(Y_OR_N_DOC.into())),
    );
    env.set_function(
        "yes-or-no".into(),
        ELispExp::primitive(yes_or_no, Some(YES_OR_NO_DOC.into())),
    );
    env.set_function(
        "ask--answer".into(),
        ELispExp::primitive(ask_answer, Some(ASK_ANSWER_DOC.into())),
    );
    env.set_function(
        "ask--typed".into(),
        ELispExp::primitive(ask_typed, Some(ASK_TYPED_DOC.into())),
    );
}
