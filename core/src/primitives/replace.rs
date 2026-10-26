//! Replacing from Lisp: the four commands that start a replace, the verbs a
//! view answers each match with, and the view used when
//! `*replace-read-function*` names none, a transient keymap of five keys.
//! What is replaced, and where, is decided in `editor/replace.rs`.
//!
//! Design: `doc/architettura-editor.typ, <sostituzione>`.
use super::*;
use crate::editor::{CASE_REPLACE, REPLACE_READ_FUNCTION};
use crate::input::{Keymap, OnUnbound, TransientKeymap};
use crate::primitives::args;
use risp::call_callable;

fn flag<B: BufferTrait>(
    env: &std::sync::Arc<Env<EditorState<B>>>,
    name: &str,
    unset: bool,
) -> bool {
    match env.get_variable(name) {
        Some(value) => value.is_truthy(),
        None => unset,
    }
}

/// Starts a replace with a command's arguments, and replaces every match at
/// once or, when INTERACTIVE, offers the first.
fn begin<B: BufferTrait>(
    args: &[ELispExp<B>],
    env: std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
    regexp: bool,
    interactive: bool,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let source = args::name_or_empty(args.first());
    let replacement = args::name_or_empty(args.get(1));
    if let Some(why) = ctx.begin_replace(
        &source,
        &replacement,
        regexp,
        // Whether case is ignored is what `case-fold-search' says, read by
        // the function the incremental search reads it with.
        crate::feature::isearch::case_fold(&env),
        flag(&env, CASE_REPLACE, true),
    ) {
        ctx.set_echo_message(&why);
        return Ok(ELispExp::nil());
    }
    if ctx.replace_match().is_none() {
        ctx.end_replace(false);
        ctx.set_echo_message(&format!("No match for {source}"));
        return Ok(ELispExp::nil());
    }
    if !interactive {
        let count = ctx.replace_rest();
        ctx.end_replace(false);
        ctx.set_echo_message(&plural(count));
        return Ok(ELispExp::number(count as f64));
    }
    ask(env, ctx)
}

/// Offers the match on offer, through the view `*replace-read-function*`
/// names or the built-in one; with no match left, ends the replace.
fn ask<B: BufferTrait>(
    env: std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    if ctx.replace_match().is_none() {
        let count = ctx.end_replace(false);
        ctx.set_echo_message(&plural(count));
        return Ok(ELispExp::number(count as f64));
    }
    match env.get_variable(REPLACE_READ_FUNCTION) {
        Some(view) if view.is_truthy() => {
            call_callable(&view, &[], env.clone(), ctx)?;
        }
        _ => default_view(ctx),
    }
    Ok(ELispExp::t())
}

fn plural(count: usize) -> String {
    match count {
        0 => "Replaced nothing".to_string(),
        1 => "Replaced 1 occurrence".to_string(),
        n => format!("Replaced {n} occurrences"),
    }
}

// TODO(RIC-8): the replace does not end when another key takes the question
// down, and its match stays marked.
/// The view used when `*replace-read-function*` names none: a transient
/// keymap of five keys over the buffer. `OnUnbound::Release`, an offer rather
/// than a question: another key takes it down, and does what it always does.
fn default_view<B: BufferTrait>(ctx: &EditorState<B>) {
    let Some((_, _, text, expansion)) = ctx.replace_match() else {
        return;
    };
    let mut keymap = Keymap::new();
    for (key, command) in [
        ("y", "replace-this"),
        ("n", "replace-skip"),
        ("!", "replace-rest"),
        ("q", "replace-done"),
        ("^", "replace-back"),
    ] {
        if let Some(event) = super::parse_key(key) {
            keymap.insert_key(event, ELispExp::symbol(command.into()));
        }
    }
    ctx.set_transient_keymap(TransientKeymap {
        keymap,
        on_unbound: OnUnbound::Release,
        message: format!("Replace {text:?} with {expansion:?}? (y, n, ! = all, ^ = back, q)"),
    });
}

pub const QUERY_REPLACE_DOC: &str = "(query-replace FROM TO): Replace FROM with TO, asking at \
         each match, from point to the end of the buffer, or through the region when the mark is \
         active. y replaces, n skips, ! replaces all the rest, ^ takes back the last one and q \
         stops; any other key stops and then does its own job. TO takes the case of what it \
         replaces unless `case-replace' is nil, and which matches are found follows \
         `case-fold-search'. A function in `*replace-read-function*' asks instead, when there is \
         one.\n\n\
         Example:\n\
         (query-replace \"colour\" \"color\")";

primitive!(query_replace, args, env, ctx, {
    begin(args, env, ctx, false, true)
});

pub const QUERY_REPLACE_REGEXP_DOC: &str = "(query-replace-regexp FROM TO): Like `query-replace', \
         with FROM a regular expression. In TO, \\1 to \\9 stand for what its groups matched, and \
         \\& for the whole match.\n\n\
         Example:\n\
         (query-replace-regexp \"(\\\\w+) (\\\\w+)\" \"\\\\2 \\\\1\")";

primitive!(query_replace_regexp, args, env, ctx, {
    begin(args, env, ctx, true, true)
});

pub const REPLACE_STRING_DOC: &str = "(replace-string FROM TO): Replace every FROM with TO \
         without asking, from point to the end of the buffer, or through the region when the mark \
         is active. Return how many were replaced, or nil when there was none. Case is handled as \
         by `query-replace'.";

primitive!(replace_string, args, env, ctx, {
    begin(args, env, ctx, false, false)
});

pub const REPLACE_REGEXP_DOC: &str = "(replace-regexp FROM TO): Like `replace-string', with FROM \
         a regular expression and TO as in `query-replace-regexp'.";

primitive!(replace_regexp, args, env, ctx, {
    begin(args, env, ctx, true, false)
});

// ---------------------------------------------------------------------------
// The verbs, which are what a view calls
// ---------------------------------------------------------------------------

pub const REPLACE_THIS_DOC: &str = "(replace-this): Replace the match on offer, and offer the \
         next. Return t while matches are left, the number replaced once there are none, and nil \
         when no replace is running.";

primitive!(replace_this, _args, env, ctx, {
    if !ctx.replace_active() {
        return Ok(ELispExp::nil());
    }
    ctx.clear_transient_keymap();
    ctx.replace_this();
    ask(env, ctx)
});

pub const REPLACE_SKIP_DOC: &str = "(replace-skip): Leave the match on offer as it is, and offer \
         the next. Return as `replace-this' does.";

primitive!(replace_skip, _args, env, ctx, {
    if !ctx.replace_active() {
        return Ok(ELispExp::nil());
    }
    ctx.clear_transient_keymap();
    ctx.replace_skip();
    ask(env, ctx)
});

pub const REPLACE_REST_DOC: &str = "(replace-rest): Replace the match on offer and all the rest \
         without asking, finish, and return how many were replaced in all.";

primitive!(replace_rest, _args, _env, ctx, {
    if !ctx.replace_active() {
        return Ok(ELispExp::nil());
    }
    ctx.clear_transient_keymap();
    ctx.replace_rest();
    let count = ctx.end_replace(false);
    ctx.set_echo_message(&plural(count));
    Ok(ELispExp::number(count as f64))
});

pub const REPLACE_BACK_DOC: &str = "(replace-back): Take back the last replacement and offer that \
         match again. Return t, or nil when nothing has been replaced yet.";

primitive!(replace_back, _args, env, ctx, {
    if !ctx.replace_active() || ctx.replace_count() == 0 {
        ctx.set_echo_message("No replacement to take back");
        return Ok(ELispExp::nil());
    }
    ctx.clear_transient_keymap();
    ctx.replace_back();
    ask(env, ctx)
});

pub const REPLACE_DONE_DOC: &str = "(replace-done): Stop, keeping what has been replaced, and \
         return how many.";

primitive!(replace_done, _args, _env, ctx, {
    ctx.clear_transient_keymap();
    let count = ctx.end_replace(false);
    ctx.set_echo_message(&plural(count));
    Ok(ELispExp::number(count as f64))
});

pub const REPLACE_ABANDON_DOC: &str = "(replace-abandon): Stop, and put point back where the \
         replace started. What has been replaced stays replaced. Return how many.";

primitive!(replace_abandon, _args, _env, ctx, {
    ctx.clear_transient_keymap();
    let count = ctx.end_replace(true);
    Ok(ELispExp::number(count as f64))
});

pub const REPLACE_MATCH_DOC: &str = "(replace-match): Return the match on offer as (START END \
         TEXT REPLACEMENT), REPLACEMENT being what TEXT would become, or nil when nothing is on \
         offer.";

primitive!(replace_match, _args, _env, ctx, {
    Ok(match ctx.replace_match() {
        Some((start, end, text, expansion)) => ELispExp::proper_list(vec![
            ELispExp::number(start as f64),
            ELispExp::number(end as f64),
            ELispExp::string(text),
            ELispExp::string(expansion),
        ]),
        None => ELispExp::nil(),
    })
});

pub const REPLACE_COUNT_DOC: &str = "(replace-count): Return how many matches the replace running \
         now has replaced, or 0.";

primitive!(replace_count, _args, _env, ctx, {
    Ok(ELispExp::number(ctx.replace_count() as f64))
});

/// Registers the primitives of replacing.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Only the four that start a replace are commands: the verbs are called
    // by a view, not by a person.
    into.command(
        "query-replace",
        query_replace,
        &["sReplace: ", "sWith: "],
        QUERY_REPLACE_DOC,
    );
    into.command(
        "query-replace-regexp",
        query_replace_regexp,
        &["sReplace regexp: ", "sWith: "],
        QUERY_REPLACE_REGEXP_DOC,
    );
    into.command(
        "replace-string",
        replace_string,
        &["sReplace: ", "sWith: "],
        REPLACE_STRING_DOC,
    );
    into.command(
        "replace-regexp",
        replace_regexp,
        &["sReplace regexp: ", "sWith: "],
        REPLACE_REGEXP_DOC,
    );
    into.function("replace-this", replace_this, REPLACE_THIS_DOC);
    into.function("replace-skip", replace_skip, REPLACE_SKIP_DOC);
    into.function("replace-rest", replace_rest, REPLACE_REST_DOC);
    into.function("replace-back", replace_back, REPLACE_BACK_DOC);
    into.function("replace-done", replace_done, REPLACE_DONE_DOC);
    into.function("replace-abandon", replace_abandon, REPLACE_ABANDON_DOC);
    into.function("replace-match", replace_match, REPLACE_MATCH_DOC);
    into.function("replace-count", replace_count, REPLACE_COUNT_DOC);
}
