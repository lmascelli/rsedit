//! Leaving the editor without losing work: `quit`, `save-some-buffers`, and
//! what a buffer must be for its loss to be worth a question.
//!
//! Design: `doc/architettura-editor.typ, <uscire>`.
use super::*;
use crate::primitives::files::write_buffer;
use crate::primitives::{args, ask};
use risp::{call_callable, eval};

/// Answers whether closing the buffer called NAME would lose something: it is
/// modified, not read-only, and visiting a file.
///
/// Shared by quitting and by `close-buffer`, so that the two agree on which
/// buffers matter.
pub(super) fn has_unsaved_work<B: BufferTrait>(ctx: &EditorState<B>, name: &str) -> bool {
    ctx.with_buffer(name, |buf| {
        buf.is_modified && !buf.read_only && buf.file_path.is_some()
    })
    .unwrap_or(false)
}

pub const SAVE_SOME_BUFFERS_DOC: &str = "(save-some-buffers &optional AFTER): Offer to save each \
         buffer with unsaved changes that is visiting a file, one at a time, then call AFTER with \
         no arguments, however the questions ended. y saves the buffer, n leaves it, ! saves it \
         and every one left, and q stops asking; no other key answers.\n\n\
         Example:\n\
         (save-some-buffers (lambda () (message \"done\")))";

primitive!(save_some_buffers, args, env, ctx, {
    let after = args.first().cloned().unwrap_or_else(ELispExp::nil);
    let names = unsaved_file_buffers(ctx);
    ask_about(ctx, env, &names, &after)
});

/// Returns the buffers a quit has to account for: those
/// [`has_unsaved_work`] answers yes for.
fn unsaved_file_buffers<B: BufferTrait>(ctx: &EditorState<B>) -> Vec<String> {
    ctx.buffer_names()
        .into_iter()
        .filter(|name| has_unsaved_work(ctx, name))
        .collect()
}

/// Asks about the first of NAMES, or runs AFTER when there are none left.
fn ask_about<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: std::sync::Arc<Env<EditorState<B>>>,
    names: &[String],
    after: &ELispExp<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    ask_about_impl(ctx, env, names, after, false)
}

fn ask_about_impl<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: std::sync::Arc<Env<EditorState<B>>>,
    names: &[String],
    after: &ELispExp<B>,
    asynchronous: bool,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let Some((name, rest)) = names.split_first() else {
        if after.is_truthy() {
            call_callable(after, &[], env, ctx)?;
        }
        return Ok(ELispExp::nil());
    };
    // Re-checked rather than trusted: the list was taken before the first
    // question, and answering one runs arbitrary Lisp -- a hook, a callback --
    // which may have saved or killed this one in the meantime.
    let still_unsaved = ctx
        .with_buffer(name, |buf| buf.is_modified && buf.file_path.is_some())
        .unwrap_or(false);
    if !still_unsaved {
        return ask_about_impl(ctx, env, rest, after, asynchronous);
    }

    let path = ctx
        .with_buffer(name, |buf| buf.file_path.clone())
        .flatten()
        .unwrap_or_else(|| name.clone());
    ask::ask_with_keys(
        ctx,
        format!("Save {path}? (y, n, ! = all, q = stop)"),
        vec![
            ("y", answer_form("save", name, rest, after, asynchronous)),
            ("n", answer_form("skip", name, rest, after, asynchronous)),
            ("!", answer_form("all", name, rest, after, asynchronous)),
            ("q", answer_form("stop", name, rest, after, asynchronous)),
        ],
    );
    Ok(ELispExp::t())
}

/// Returns the call that answers one buffer's question.
fn answer_form<B: BufferTrait>(
    action: &str,
    name: &str,
    rest: &[String],
    after: &ELispExp<B>,
    asynchronous: bool,
) -> ELispExp<B> {
    ELispExp::form(vec![
        ELispExp::symbol(
            if asynchronous {
                "save-some-buffers--async-answer"
            } else {
                "save-some-buffers--answer"
            }
            .into(),
        ),
        ELispExp::string(action.to_string()),
        ELispExp::string(name.to_string()),
        args::quoted(names_list(rest)),
        args::quoted(after.clone()),
    ])
}

fn names_list<B: BufferTrait>(names: &[String]) -> ELispExp<B> {
    ELispExp::proper_list(names.iter().map(|n| ELispExp::string(n.clone())).collect())
}

pub const SAVE_SOME_BUFFERS_ANSWER_DOC: &str = "(save-some-buffers--answer ACTION NAME REST AFTER): \
         Act on one answer and move on. Not called directly -- it is what `save-some-buffers' \
         binds its keys to.";

primitive!(save_some_buffers_answer, args, env, ctx, {
    // Before anything else, so that a callback which asks its own question
    // installs a keymap that this does not then take down again.
    ctx.clear_transient_keymap();

    let action = match args.first() {
        Some(ELispExp::String(text)) => text.to_string(),
        _ => return Ok(ELispExp::nil()),
    };
    let name = match args.get(1) {
        Some(ELispExp::String(text)) => text.to_string(),
        _ => return Ok(ELispExp::nil()),
    };
    let rest: Vec<String> = strings_from(args.get(2));
    let after = args.get(3).cloned().unwrap_or_else(ELispExp::nil);

    match action.as_str() {
        "save" => {
            write_buffer(ctx, &env, &name);
        }
        "all" => {
            // This one and every one left, with no further questions.
            write_buffer(ctx, &env, &name);
            for other in &rest {
                write_buffer(ctx, &env, other);
            }
            return ask_about(ctx, env, &[], &after);
        }
        "stop" => return ask_about(ctx, env, &[], &after),
        // "skip", and anything else, leaves it alone.
        _ => {}
    }
    ask_about(ctx, env, &rest, &after)
});

fn strings_from<B: BufferTrait>(value: Option<&ELispExp<B>>) -> Vec<String> {
    value
        .map(|list| {
            list.iter()
                .filter_map(|item| match item {
                    ELispExp::String(text) => Some(text.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub const QUIT_DOC: &str = "(quit): Leave the editor, first offering to save each buffer with \
         unsaved changes to a file, as `save-some-buffers' does. If any are still unsaved after \
         that, leaving has to be confirmed by typing yes. `quit-without-saving' asks nothing.";

primitive!(quit, _args, env, ctx, {
    let names = unsaved_file_buffers(ctx);
    let after = ELispExp::symbol("quit--confirm".into());
    ask_about(ctx, env, &names, &after)
});

pub const QUIT_CONFIRM_DOC: &str = "(quit--confirm): Quit, or ask first if anything is still \
         unsaved. Not called directly -- `quit' runs it once the offers to save are done.";

primitive!(quit_confirm, _args, env, ctx, {
    if ctx.file_writes_pending() {
        ctx.set_echo_message(
            "File writes are still pending; wait for their outcome before quitting",
        );
        return Ok(ELispExp::nil());
    }
    let left = unsaved_file_buffers(ctx);
    if left.is_empty() {
        ctx.quit();
        return Ok(ELispExp::nil());
    }
    // Typed in full, because this is the step that loses the work. The
    // per-buffer questions before it are single keys precisely so that the
    // budget for care can be spent here.
    let prompt = match left.len() {
        1 => format!("{} is unsaved. Quit anyway?", left[0]),
        n => format!("{n} buffers are unsaved. Quit anyway?"),
    };
    eval(
        &ELispExp::form(vec![
            ELispExp::symbol("yes-or-no".into()),
            ELispExp::string(prompt),
            args::quoted(ELispExp::symbol("quit-without-saving".into())),
        ]),
        env,
        ctx,
    )
});

pub const QUIT_WITHOUT_SAVING_DOC: &str = "(quit-without-saving): Leave the editor at once, \
         asking nothing and saving nothing.";

primitive!(quit_without_saving, _args, _env, ctx, {
    ctx.quit();
    Ok(ELispExp::nil())
});

pub const SAVE_SOME_ASYNC_DOC: &str = "(save-some-buffers-async &optional AFTER): Offer to save \
    modified file buffers, as save-some-buffers does, waiting asynchronously for each accepted \
    save before continuing. An I/O failure stops the sequence. AFTER takes no arguments.";
primitive!(save_some_buffers_async, args, env, ctx, {
    let after = args.first().cloned().unwrap_or_else(ELispExp::nil);
    ask_about_impl(ctx, env, &unsaved_file_buffers(ctx), &after, true)
});

pub const QUIT_ASYNC_DOC: &str = "(quit-async): Offer to save modified file buffers asynchronously \
    before quitting. A pending file write prevents normal exit; quit-without-saving forces exit.";
primitive!(quit_async, _args, env, ctx, {
    if ctx.file_writes_pending() {
        ctx.set_echo_message("File writes are still pending; wait before quitting");
        return Ok(ELispExp::nil());
    }
    ask_about_impl(
        ctx,
        env,
        &unsaved_file_buffers(ctx),
        &ELispExp::symbol("quit--confirm".into()),
        true,
    )
});

fn continue_async<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: std::sync::Arc<Env<EditorState<B>>>,
    names: &[String],
    after: &ELispExp<B>,
    all: bool,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    if !all || names.is_empty() {
        return ask_about_impl(ctx, env, names, after, true);
    }
    let (name, rest) = names.split_first().expect("nonempty");
    if !has_unsaved_work(ctx, name) {
        return continue_async(ctx, env, rest, after, all);
    }
    let callback = ELispExp::lambda(risp::Lambda {
        params: vec!["ok".into()],
        optionals: Vec::new(),
        rest: None,
        env: env.clone(),
        doc: None,
        body: vec![ELispExp::form(vec![
            ELispExp::symbol("if".into()),
            ELispExp::symbol("ok".into()),
            ELispExp::form(vec![
                ELispExp::symbol("save-some-buffers--async-next".into()),
                args::quoted(names_list(rest)),
                args::quoted(after.clone()),
                ELispExp::t(),
            ]),
            ELispExp::form(vec![
                ELispExp::symbol("message".into()),
                ELispExp::string("Save sequence stopped; unsaved buffers preserved".into()),
            ]),
        ])],
    });
    super::file_io::save_named(ctx, &env, name, None, callback)
}

pub const ASYNC_NEXT_DOC: &str = "(save-some-buffers--async-next NAMES AFTER ALL): Private continuation \
    for asynchronous save offers.";
primitive!(save_some_async_next, args, env, ctx, {
    let names = strings_from(args.first());
    let after = args.get(1).cloned().unwrap_or_else(ELispExp::nil);
    continue_async(
        ctx,
        env,
        &names,
        &after,
        args.get(2).is_some_and(ELispExp::is_truthy),
    )
});

pub const ASYNC_ANSWER_DOC: &str = "(save-some-buffers--async-answer ACTION NAME REST AFTER): Private \
    answer callback for asynchronous save offers.";
primitive!(save_some_async_answer, args, env, ctx, {
    ctx.clear_transient_keymap();
    let Some(ELispExp::String(action)) = args.first() else {
        return Ok(ELispExp::nil());
    };
    let Some(ELispExp::String(name)) = args.get(1) else {
        return Ok(ELispExp::nil());
    };
    let rest = strings_from(args.get(2));
    let after = args.get(3).cloned().unwrap_or_else(ELispExp::nil);
    match action.as_str() {
        "stop" => ask_about_impl(ctx, env, &[], &after, true),
        "skip" => ask_about_impl(ctx, env, &rest, &after, true),
        "all" => {
            let mut names = vec![name.to_string()];
            names.extend(rest);
            continue_async(ctx, env, &names, &after, true)
        }
        "save" => {
            let callback = ELispExp::lambda(risp::Lambda {
                params: vec!["ok".into()],
                optionals: Vec::new(),
                rest: None,
                env: env.clone(),
                doc: None,
                body: vec![ELispExp::form(vec![
                    ELispExp::symbol("if".into()),
                    ELispExp::symbol("ok".into()),
                    ELispExp::form(vec![
                        ELispExp::symbol("save-some-buffers--async-next".into()),
                        args::quoted(names_list(&rest)),
                        args::quoted(after),
                        ELispExp::nil(),
                    ]),
                    ELispExp::form(vec![
                        ELispExp::symbol("message".into()),
                        ELispExp::string("Save sequence stopped; unsaved buffers preserved".into()),
                    ]),
                ])],
            });
            super::file_io::save_named(ctx, &env, name, None, callback)
        }
        _ => Ok(ELispExp::nil()),
    }
});

/// Registers the primitives that leave the editor, and offer to save first.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command(
        "save-some-buffers-async",
        save_some_buffers_async,
        &[],
        SAVE_SOME_ASYNC_DOC,
    );
    into.command("quit-async", quit_async, &[], QUIT_ASYNC_DOC);
    into.function(
        "save-some-buffers--async-answer",
        save_some_async_answer,
        ASYNC_ANSWER_DOC,
    );
    into.function(
        "save-some-buffers--async-next",
        save_some_async_next,
        ASYNC_NEXT_DOC,
    );
    into.env.put_property(
        "quit",
        "interactive-function",
        ELispExp::symbol("quit-async".into()),
    );
    into.env.put_property(
        "save-some-buffers",
        "interactive-function",
        ELispExp::symbol("save-some-buffers-async".into()),
    );
    into.command("quit", quit, &[], QUIT_DOC);
    into.command(
        "quit-without-saving",
        quit_without_saving,
        &[],
        QUIT_WITHOUT_SAVING_DOC,
    );
    into.command(
        "save-some-buffers",
        save_some_buffers,
        &[],
        SAVE_SOME_BUFFERS_DOC,
    );
    into.function(
        "save-some-buffers--answer",
        save_some_buffers_answer,
        SAVE_SOME_BUFFERS_ANSWER_DOC,
    );
    into.function("quit--confirm", quit_confirm, QUIT_CONFIRM_DOC);
}
