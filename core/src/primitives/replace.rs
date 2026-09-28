//! The replace, as Lisp sees it: the verbs, and the view that is used when
//! nothing better is loaded.
//!
//! # The split
//!
//! Everything that decides *what* gets replaced is in Rust and cannot be taken
//! away by a missing module -- see [`crate::editor::replace`]. Everything that
//! decides *how you are asked* is named by `*replace-read-function*`, which
//! anything may rebind. A window-based replace with a preview pane and two
//! buttons rebinds that one variable and calls the same verbs from its
//! buttons; nothing below has to know it exists.
//!
//! The built-in view is deliberately the plainest thing that works: a
//! transient keymap over the buffer with five keys. It exists so that the
//! feature is complete with no configuration at all, which is the same reason
//! the minibuffer has a built-in prompt.
use super::*;
use crate::input::{Keymap, OnUnbound, TransientKeymap};
use crate::lisp::call_callable;

/// The name of the variable naming the view.
pub const REPLACE_READ_FUNCTION: &str = "*replace-read-function*";

/// The name of the variable that turns case-preserving replacement off.
pub const CASE_REPLACE: &str = "case-replace";

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

fn string_arg<B: BufferTrait>(args: &[ELispExp<B>], index: usize) -> String {
    match args.get(index) {
        Some(ELispExp::String(text)) => text.to_string(),
        Some(ELispExp::Symbol(text)) => text.to_string(),
        _ => String::new(),
    }
}

/// Start a session and hand it to whoever is asking.
fn begin<B: BufferTrait>(
    args: &[ELispExp<B>],
    env: std::sync::Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
    regexp: bool,
    interactive: bool,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let source = string_arg(args, 0);
    let replacement = string_arg(args, 1);
    if let Some(why) = ctx.begin_replace(
        &source,
        &replacement,
        regexp,
        // `case-fold-search' is the editor's one answer to "does searching
        // ignore case", already read by the incremental search and already
        // bound from Rust. Replace reads it through the same function rather
        // than deciding for itself -- two commands disagreeing about what one
        // variable means is the surprise worth avoiding here.
        crate::isearch::case_fold(&env),
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

/// Offer the current match, through whichever view is installed.
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

/// The view used when nothing has replaced it: five keys over the buffer.
///
/// `OnUnbound::Release` rather than `Refuse`, which is the difference between
/// a question and an offer. Any other key ends the replace and then does what
/// it always does -- so a replace is abandoned by simply carrying on typing,
/// which is the convention every editor with this feature shares and the one
/// thing about it nobody has to be taught.
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

pub const QUERY_REPLACE_DOC: &str = "(query-replace FROM TO): Replace FROM with TO, asking about \
         each one.\n\n\
         In the region when the mark is active, from point to the end of the buffer \
         otherwise.\n\n\
         `y' replaces this one, `n' leaves it, `!' replaces this one and all the rest, `^' takes \
         back the last replacement and offers it again, and `q' stops. Any other key stops and \
         then does what it normally does, so carrying on typing abandons it.\n\n\
         Each replacement is its own undo step, so `C-/' walks back through them one at a \
         time -- which is also what `^' is built out of.\n\n\
         Whether FROM matches regardless of case is `case-fold-search' -- the same variable the \
         incremental search reads, on unless something turned it off.\n\n\
         TO goes in with the casing of what it replaced, unless `case-replace' is nil: replacing \
         `colour' with `color' turns `Colour' into `Color'. Group references (`\\\\1' to `\\\\9', \
         `\\\\&' for the whole match) are filled in per match; under a literal pattern, which has \
         no groups, they expand to nothing.\n\n\
         How you are asked is `*replace-read-function*'; what gets replaced is not configurable, \
         because a replace that a missing module could silently change is not one anybody could \
         trust.\n\n\
         Example:\n\
         (query-replace \"colour\" \"color\")";

primitive!(query_replace, args, env, ctx, {
    begin(args, env, ctx, false, true)
});

pub const QUERY_REPLACE_REGEXP_DOC: &str = "(query-replace-regexp FROM TO): Like `query-replace', \
         with FROM read as a regular expression and `\\\\1'...`\\\\9' in TO standing for what its \
         groups matched.\n\n\
         Example:\n\
         (query-replace-regexp \"(\\\\\\\\w+), (\\\\\\\\w+)\" \"\\\\\\\\2 \\\\\\\\1\")";

primitive!(query_replace_regexp, args, env, ctx, {
    begin(args, env, ctx, true, true)
});

pub const REPLACE_STRING_DOC: &str = "(replace-string FROM TO): Replace every FROM with TO, \
         asking nothing. Returns how many.\n\n\
         In the region when the mark is active, from point to the end of the buffer otherwise -- \
         the same scope `query-replace' uses, and the same casing and group rules.";

primitive!(replace_string, args, env, ctx, {
    begin(args, env, ctx, false, false)
});

pub const REPLACE_REGEXP_DOC: &str = "(replace-regexp FROM TO): Like `replace-string', with FROM \
         read as a regular expression.";

primitive!(replace_regexp, args, env, ctx, {
    begin(args, env, ctx, true, false)
});

// ---------------------------------------------------------------------------
// The verbs, which are what a view calls
// ---------------------------------------------------------------------------

pub const REPLACE_THIS_DOC: &str = "(replace-this): Replace the match being offered and move to \
         the next. Returns t, or nil if no replace is running.\n\n\
         One of the verbs a view drives a replace with. The others are `replace-skip', \
         `replace-rest', `replace-back', `replace-done' and `replace-abandon'; `replace-match' \
         says what is being offered and `replace-count' how many have been done.";

primitive!(replace_this, _args, env, ctx, {
    if !ctx.replace_active() {
        return Ok(ELispExp::nil());
    }
    ctx.clear_transient_keymap();
    ctx.replace_this();
    ask(env, ctx)
});

pub const REPLACE_SKIP_DOC: &str =
    "(replace-skip): Leave the match being offered alone and move to the next.";

primitive!(replace_skip, _args, env, ctx, {
    if !ctx.replace_active() {
        return Ok(ELispExp::nil());
    }
    ctx.clear_transient_keymap();
    ctx.replace_skip();
    ask(env, ctx)
});

pub const REPLACE_REST_DOC: &str = "(replace-rest): Replace the match being offered and every one \
         after it, asking nothing more, and finish. Returns how many were replaced in all.";

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

pub const REPLACE_BACK_DOC: &str = "(replace-back): Take back the last replacement and offer it \
         again. Returns t if there was one.\n\n\
         An undo and a re-search, which is why each replacement is its own undo step.";

primitive!(replace_back, _args, env, ctx, {
    if !ctx.replace_active() || ctx.replace_count() == 0 {
        ctx.set_echo_message("No replacement to take back");
        return Ok(ELispExp::nil());
    }
    ctx.clear_transient_keymap();
    ctx.replace_back();
    ask(env, ctx)
});

pub const REPLACE_DONE_DOC: &str = "(replace-done): Stop here, keeping what has been replaced. \
         Returns how many.";

primitive!(replace_done, _args, _env, ctx, {
    ctx.clear_transient_keymap();
    let count = ctx.end_replace(false);
    ctx.set_echo_message(&plural(count));
    Ok(ELispExp::number(count as f64))
});

pub const REPLACE_ABANDON_DOC: &str = "(replace-abandon): Stop, and put point back where the \
         replace started. What has already been replaced stays replaced -- use `undo' for that.";

primitive!(replace_abandon, _args, _env, ctx, {
    ctx.clear_transient_keymap();
    let count = ctx.end_replace(true);
    Ok(ELispExp::number(count as f64))
});

pub const REPLACE_MATCH_DOC: &str = "(replace-match): What the replace is offering, as \
         (START END TEXT REPLACEMENT), or nil when it is offering nothing.\n\n\
         REPLACEMENT is what TEXT would become -- group references filled in and casing \
         applied -- so a view can show the change without working any of it out.";

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

pub const REPLACE_COUNT_DOC: &str =
    "(replace-count): How many have been replaced in the session running now, or 0.";

primitive!(replace_count, _args, _env, ctx, {
    Ok(ELispExp::number(ctx.replace_count() as f64))
});
