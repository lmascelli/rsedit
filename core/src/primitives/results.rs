//! Lists of places: what one holds, walking it with `next-error`, attaching
//! one from Lisp, and showing it.
//!
//! Design: `doc/architettura-editor.typ, <risultati>`.
use super::*;
use crate::primitives::args;
use crate::text::results::{Entry, KIND_FILE, RESULTS_KEY, Results};
use risp::eval;
use std::path::Path;

/// Attaches RESULTS to the buffer called NAME, in place of what was there.
pub(crate) fn attach<B: BufferTrait>(ctx: &EditorState<B>, name: &str, results: Results) {
    ctx.with_buffer_mut(name, |buf| buf.put_data(RESULTS_KEY, results));
}

/// Returns BODY applied to the results attached to NAME, or `None` when
/// there are none.
fn with_results<B: BufferTrait, R>(
    ctx: &EditorState<B>,
    name: &str,
    body: impl FnOnce(&Results) -> R,
) -> Option<R> {
    ctx.with_buffer(name, |buf| buf.data::<Results>(RESULTS_KEY).map(body))
        .flatten()
}

/// Returns BODY applied to the results attached to NAME, which it may
/// change, or `None` when there are none.
fn with_results_mut<B: BufferTrait, R>(
    ctx: &EditorState<B>,
    name: &str,
    body: impl FnOnce(&mut Results) -> R,
) -> Option<R> {
    ctx.with_buffer_mut(name, |buf| buf.data_mut::<Results>(RESULTS_KEY).map(body))
        .flatten()
}

/// The overlay marking the line being visited, in the file.
const TARGET_CATEGORY: &str = "results-target";
/// The overlay marking the entry being visited, in the listing.
const HERE_CATEGORY: &str = "results-here";
/// Where these marks sit among overlays that overlap them: above a manual
/// page's emphasis, at 10.
const MARK_PRIORITY: i32 = 20;

/// Returns the buffer whose set the walking commands work on, while it lives.
fn current_buffer_of_results<B: BufferTrait>(ctx: &EditorState<B>) -> Option<String> {
    let name = ctx.current_results()?;
    // A listing that has been killed is not the current one any more. Asked
    // here rather than remembered, because nothing tells this when a buffer
    // goes.
    ctx.has_buffer(&name).then_some(name)
}

/// Marks the line of point in the current buffer with an overlay of CATEGORY,
/// in the face of that name, or does nothing when there is no such face: a
/// view that has not said how the marks look has not asked for them.
fn mark_line<B: BufferTrait>(ctx: &EditorState<B>, category: &str) {
    let Some(face) = crate::ui::Face::named(category) else {
        return;
    };
    ctx.with_current_buffer_mut(|buf| {
        buf.overlays.remove_category(Some(category));
        let point = buf.text.cursor_pos_1d();
        let line = buf.text.cursor_1d_to_2d(point).0;
        let (start, end) = crate::primitives::movement::line_bounds(&buf.text, line);
        if end > start {
            buf.overlays
                .add(start, end, face, MARK_PRIORITY, category.into());
        }
    });
}

/// Takes the overlays of CATEGORY off BUFFER.
fn unmark<B: BufferTrait>(ctx: &EditorState<B>, buffer: &str, category: &str) {
    ctx.with_buffer_mut(buffer, |buf| {
        buf.overlays.remove_category(Some(category));
    });
}

// TODO(RIC-5): an entry is a line and a column, which an edit above it
// leaves behind.
/// Goes to ENTRY: opens what it is in, puts point on it and marks the line;
/// false when its file or its buffer is gone.
fn visit<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    entry: &Entry,
) -> Result<bool, EvalError<EditorState<B>>> {
    if entry.kind == KIND_FILE {
        // Said rather than opened: a path that is not there was written by a
        // build run somewhere else, and an empty buffer of the same name would
        // look like the file.
        if !Path::new(&crate::primitives::file_names::expand_path(&entry.source)).exists() {
            ctx.set_echo_message(&format!("No such file: {}", entry.source));
            return Ok(false);
        }
        // Through `find-file` rather than around it: opening a file means
        // reusing the buffer already visiting it, reporting a file that has
        // gone, noticing an auto-save copy. A second implementation of that
        // would be a second set of answers.
        let opened = eval(
            &ELispExp::form(vec![
                ELispExp::symbol("find-file".into()),
                ELispExp::string(entry.source.clone()),
            ]),
            env.clone(),
            ctx,
        )?;
        if opened.is_nil() {
            ctx.set_echo_message(&format!("Cannot open {}", entry.source));
            return Ok(false);
        }
    } else if ctx.has_buffer(&entry.source) {
        ctx.switch_to_buffer(&entry.source);
    } else {
        ctx.set_echo_message(&format!("{} is gone", entry.source));
        return Ok(false);
    }
    ctx.with_current_buffer_mut(|buf| {
        // By line and column rather than by the offset: a file edited since
        // the search still has the line roughly where it was, while an offset
        // taken from older text lands wherever that many characters now is.
        let line = entry.line.saturating_sub(1);
        let column = entry
            .column
            .min(crate::primitives::movement::line_length(&buf.text, line));
        buf.text.cursor_move(line, column);
    });
    mark_line(ctx, TARGET_CATEGORY);
    Ok(true)
}

/// Visits entry INDEX of the set in BUFFER, the current entry from now on,
/// and with PREVIEW gives the focus back; returns the entry, or nil.
fn visit_index<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    buffer: &str,
    index: usize,
    preview: bool,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let Some(entry) = with_results(ctx, buffer, |results| results.get(index).cloned()).flatten()
    else {
        return Ok(ELispExp::nil());
    };
    let from = ctx.get_focused_window_id();

    if !ctx.select_results_target(buffer, preview, env) {
        ctx.set_echo_message(
            "Cannot select a window for the result",
        );
        return Ok(ELispExp::nil());
    }

    ctx.set_current_results(Some(buffer.to_string()));
    // Recorded before the jump: a visit that failed still moved the walk on,
    // or `next-error` would offer the same unreachable file for ever.
    let previous = with_results_mut(ctx, buffer, |results| {
        let previous = results.marked.clone();
        results.current = Some(index);
        results.marked = Some(entry.source.clone());
        previous
    })
    .flatten();
    // The mark left in whatever was visited before, which is not necessarily
    // the buffer this is called from.
    if let Some(previous) = previous {
        unmark(ctx, &previous, TARGET_CATEGORY);
    }
    // The listing's own mark, put on the line for this entry: the line its
    // producer gave, or else the header line's worth after the index.
    let row = entry.at.map_or(index + 1, |line| line.saturating_sub(1));
    ctx.with_buffer_mut(buffer, |buf| {
        buf.text.cursor_move(row, 0);
    });
    if ctx.has_buffer(buffer) {
        let was = ctx.get_current_buffer_name();
        ctx.switch_to_buffer(buffer);
        mark_line(ctx, HERE_CATEGORY);
        ctx.switch_to_buffer(&was);
    }
    let visited = visit(ctx, env, &entry);

    if preview {
        ctx.select_window(from);
    }

    if !visited? {
        return Ok(ELispExp::nil());
    }

    Ok(entry_form(&entry))
}

/// Returns ENTRY as Lisp sees it, the list `scan-buffer` and the result
/// lists hand out: (KIND SOURCE LINE COLUMN OFFSET TEXT MATCH-START MATCH-END).
pub(crate) fn entry_form<B: BufferTrait>(entry: &Entry) -> ELispExp<B> {
    ELispExp::proper_list(vec![
        ELispExp::string(entry.kind.clone()),
        ELispExp::string(entry.source.clone()),
        ELispExp::number(entry.line as f64),
        ELispExp::number(entry.column as f64),
        ELispExp::number(entry.offset as f64),
        ELispExp::string(entry.text.clone()),
        ELispExp::number(entry.start as f64),
        ELispExp::number(entry.end as f64),
    ])
}

/// Returns the buffer a query is about: the one named, the list `next-error`
/// walks, or the current buffer when it holds a set.
fn asked_about<B: BufferTrait>(
    args: &[ELispExp<B>],
    index: usize,
    ctx: &EditorState<B>,
) -> Option<String> {
    match args.get(index) {
        Some(ELispExp::String(name)) if !name.is_empty() => Some(name.to_string()),
        Some(ELispExp::Symbol(name)) if &**name != "nil" => Some(name.to_string()),
        _ => current_buffer_of_results(ctx)
            .or_else(|| Some(ctx.get_current_buffer_name()).filter(|name| has_results(ctx, name))),
    }
}

fn has_results<B: BufferTrait>(ctx: &EditorState<B>, name: &str) -> bool {
    with_results(ctx, name, |_| ()).is_some()
}

pub const RESULTS_COUNT_DOC: &str = "(results-count &optional BUFFER): Return how many entries \
         the list in BUFFER has, or nil when it holds none. BUFFER defaults to the list \
         `next-error' walks, and failing that to the current buffer.";

primitive!(results_count, args, _env, ctx, {
    let Some(buffer) = asked_about(args, 0, ctx) else {
        return Ok(ELispExp::nil());
    };
    Ok(match with_results(ctx, &buffer, |results| results.len()) {
        Some(count) => ELispExp::number(count as f64),
        None => ELispExp::nil(),
    })
});

pub const RESULTS_ENTRY_DOC: &str = "(results-entry N &optional BUFFER): Return entry N of the \
         list in BUFFER, counting from 0, or nil. An entry is (KIND SOURCE LINE COLUMN OFFSET \
         TEXT MATCH-START MATCH-END); BUFFER defaults as for `results-count'.\n\n\
         Example:\n\
         (results-entry 0) => (\"file\" \"src/main.rs\" 12 4 213 \"    // TODO\" 7 11)";

primitive!(results_entry, args, _env, ctx, {
    let index = match args.first() {
        Some(ELispExp::Number(n)) if *n >= 0.0 => *n as usize,
        _ => return Ok(ELispExp::nil()),
    };
    let Some(buffer) = asked_about(args, 1, ctx) else {
        return Ok(ELispExp::nil());
    };
    Ok(
        match with_results(ctx, &buffer, |results| results.get(index).cloned()).flatten() {
            Some(entry) => entry_form(&entry),
            None => ELispExp::nil(),
        },
    )
});

pub const RESULTS_STATE_DOC: &str = "(results-state &optional BUFFER): Return what the list in \
         BUFFER is, as (PATTERN OVER COUNT DONE TRUNCATED CURRENT), or nil. DONE is nil while a \
         search is still filling it, TRUNCATED is t when a limit stopped it, and CURRENT is the \
         entry last visited, or nil.\n\n\
         Example:\n\
         (results-state) => (\"TODO\" \"src\" 12 t nil 3)";

primitive!(results_state, args, _env, ctx, {
    let Some(buffer) = asked_about(args, 0, ctx) else {
        return Ok(ELispExp::nil());
    };
    Ok(with_results(ctx, &buffer, |results| {
        ELispExp::proper_list(vec![
            ELispExp::string(results.pattern.clone()),
            ELispExp::string(results.over.clone()),
            ELispExp::number(results.len() as f64),
            ELispExp::boolean(results.done),
            ELispExp::boolean(results.truncated),
            match results.current {
                Some(current) => ELispExp::number(current as f64),
                None => ELispExp::nil(),
            },
        ])
    })
    .unwrap_or_else(ELispExp::nil))
});

pub const RESULTS_VISIT_DOC: &str =
    "(results-visit N &optional BUFFER PREVIEW): Go to entry N \
     of the list in BUFFER, open its location and mark the line. \
     PREVIEW shows it in another window without moving focus. \
     Ordinary visits follow results-visit-same-window. \
     Return the entry, or nil. next-error continues from N.";
     
primitive!(results_visit, args, env, ctx, {
    let index = match args.first() {
        Some(ELispExp::Number(n)) if *n >= 0.0 => *n as usize,
        _ => return Ok(ELispExp::nil()),
    };
    let Some(buffer) = asked_about(args, 1, ctx) else {
        return Ok(ELispExp::nil());
    };
    visit_index(ctx, &env, &buffer, index, args.get(2).is_some_and(|value| value.is_truthy()),)
});

/// The name a listing files its refresher under. See [`refresh`].
const REFRESH_KEY: &str = "lisp:results-refresh";

/// Calls the function the producer of BUFFER's list attached under
/// `results-refresh`, when there is one, with the listing's name: a list that
/// is still growing, as a compilation's does, is made current before a step.
fn refresh<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    buffer: &str,
) -> Result<(), EvalError<EditorState<B>>> {
    let refresher = ctx
        .with_buffer(buffer, |buf| {
            buf.data::<ELispExp<B>>(REFRESH_KEY)
                .filter(|value| value.is_truthy())
                .cloned()
        })
        .flatten();
    let Some(refresher) = refresher else {
        return Ok(());
    };
    eval(
        &ELispExp::form(vec![
            ELispExp::symbol("funcall".into()),
            // Quoted, or a refresher named by a symbol would be looked up as
            // a variable and reported unbound.
            ELispExp::form(vec![ELispExp::symbol("quote".into()), refresher]),
            ELispExp::string(buffer.to_string()),
        ]),
        env.clone(),
        ctx,
    )?;
    Ok(())
}

/// Steps the walk by STEP and visits the entry reached: `next-error` and
/// `previous-error`, which differ by a sign.
fn step_and_visit<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    step: isize,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let Some(buffer) = current_buffer_of_results(ctx) else {
        ctx.set_echo_message("No search results to walk");
        return Ok(ELispExp::nil());
    };
    refresh(ctx, env, &buffer)?;
    let next = with_results(ctx, &buffer, |results| results.step(step)).flatten();
    let Some(next) = next else {
        // Said rather than wrapped: the end of a list is information, and a
        // walk that silently started again would have you fixing the first
        // one twice while believing you were making progress.
        ctx.set_echo_message(if step >= 0 {
            "No more results"
        } else {
            "No earlier results"
        });
        return Ok(ELispExp::nil());
    };
    visit_index(ctx, env, &buffer, next, false)
}

pub const NEXT_ERROR_DOC: &str = "(next-error): Go to the next place in the newest list -- a \
         search's, a compilation's -- opening it in the output window, and return its entry. At \
         the end of the list, say so and return nil.";

primitive!(next_error, _args, env, ctx, {
    step_and_visit(ctx, &env, 1)
});

pub const PREVIOUS_ERROR_DOC: &str = "(previous-error): Go to the place before the current one in \
         the newest list, and return its entry, or nil at the start.";

primitive!(previous_error, _args, env, ctx, {
    step_and_visit(ctx, &env, -1)
});

pub const RESULTS_BUFFER_DOC: &str = "(results-buffer): Return the name of the list `next-error' \
         walks, or nil when there is none.\n\n\
         Example:\n\
         (results-buffer) => \"*Grep*\"";

primitive!(results_buffer, _args, _env, ctx, {
    Ok(match current_buffer_of_results(ctx) {
        Some(name) => ELispExp::string(name),
        None => ELispExp::nil(),
    })
});

pub const RESULTS_SELECT_DOC: &str = "(results-select &optional BUFFER): Make the list in BUFFER, \
         or in the current buffer, the one `next-error' walks. Return its name, or nil when \
         BUFFER holds no list.";

primitive!(results_select, args, _env, ctx, {
    // TODO(RIC-9): a BUFFER of nil is taken as the buffer named nil.
    let name = match args.first() {
        Some(ELispExp::String(name)) if !name.is_empty() => name.to_string(),
        Some(ELispExp::Symbol(name)) => name.to_string(),
        _ => ctx.get_current_buffer_name(),
    };
    if !has_results(ctx, &name) {
        return Ok(ELispExp::nil());
    }
    ctx.set_current_results(Some(name.clone()));
    Ok(ELispExp::string(name))
});

pub const RESULTS_PUT_DOC: &str = "(results-put PATTERN OVER ENTRIES &optional BUFFER): Attach \
         ENTRIES to BUFFER, or to the current buffer, as a list of places, make it the list \
         `next-error' walks, and return how many there are. PATTERN and OVER say what was looked \
         for, and where. An entry is (KIND SOURCE LINE COLUMN OFFSET TEXT MATCH-START MATCH-END), \
         and all but KIND and SOURCE may be left off; a ninth element is the line of BUFFER it \
         is shown on, which is otherwise taken to follow one header line.\n\n\
         Example:\n\
         (results-put \"lint\" \"the project\" '((\"file\" \"src/main.rs\" 12)))";

primitive!(results_put, args, _env, ctx, {
    if args.len() < 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }
    let pattern = match &args[0] {
        ELispExp::String(text) => text.to_string(),
        other => risp::lisp_display(other),
    };
    let over = match &args[1] {
        ELispExp::String(text) => text.to_string(),
        other => risp::lisp_display(other),
    };
    // TODO(RIC-9): a BUFFER of nil is taken as the buffer named nil.
    let buffer = match args.get(3) {
        Some(ELispExp::String(name)) if !name.is_empty() => name.to_string(),
        Some(ELispExp::Symbol(name)) => name.to_string(),
        _ => ctx.get_current_buffer_name(),
    };
    let mut results = Results::new(&pattern, &over);
    for item in args[2].iter() {
        let parts: Vec<ELispExp<B>> = item.iter().collect();
        let text_at = |index: usize| match parts.get(index) {
            Some(ELispExp::String(text)) => text.to_string(),
            Some(ELispExp::Symbol(text)) => text.to_string(),
            _ => String::new(),
        };
        let number_at = |index: usize, fallback: usize| match parts.get(index) {
            Some(ELispExp::Number(n)) if *n >= 0.0 => *n as usize,
            _ => fallback,
        };
        if parts.len() < 2 {
            continue;
        }
        results.entries.push(Entry {
            kind: text_at(0),
            source: text_at(1),
            line: number_at(2, 1),
            column: number_at(3, 0),
            offset: number_at(4, 0),
            text: text_at(5),
            start: number_at(6, 0),
            end: number_at(7, 0),
            at: match parts.get(8) {
                Some(ELispExp::Number(n)) if *n >= 1.0 => Some(*n as usize),
                _ => None,
            },
        });
    }
    let count = results.len();
    results.done = true;
    // Where the walk had got to survives the list being replaced, as a
    // growing one is before each step; by index, since such a list grows at
    // its end.
    if let Some((current, marked)) =
        with_results(ctx, &buffer, |old| (old.current, old.marked.clone()))
    {
        results.current = current.filter(|index| *index < results.len());
        results.marked = marked;
    }
    attach(ctx, &buffer, results);
    ctx.set_current_results(Some(buffer));
    Ok(ELispExp::number(count as f64))
});

pub const RESULTS_DISPLAY_DOC: &str =
    "(results-display BUFFER HEIGHT &optional SELECT): Show \
     BUFFER according to results-display-location. SELECT gives \
     it focus. Return its window id, or nil if the buffer is absent.";

primitive!(results_display, args, env, ctx, {
    if args.len() < 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }

    let buffer = args::name(args.first())?;

    let Some(ELispExp::Number(height)) = args.get(1) else {
        return Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: args[1].clone(),
        });
    };

    let select = args
        .get(2)
        .is_some_and(|value| value.is_truthy());

    Ok(ctx
        .display_results(
            &buffer,
            height.max(1.0) as usize,
            select,
            &env,
        )
        .map(Into::into)
        .unwrap_or_else(ELispExp::nil))
});

pub const RESULTS_QUIT_DOC: &str =
    "(results-quit): Close a managed results strip or restore \
     the buffer replaced by same-window output. Keep the output \
     buffer and its job alive.";

primitive!(results_quit, _args, _env, ctx, {
    Ok(if ctx.quit_results() {
        ELispExp::t()
    } else {
        ELispExp::nil()
    })
});

/// Registers the primitives of lists of places.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.command("next-error", next_error, &[], NEXT_ERROR_DOC);
    into.command("previous-error", previous_error, &[], PREVIOUS_ERROR_DOC);
    into.function("results-count", results_count, RESULTS_COUNT_DOC);
    into.function("results-entry", results_entry, RESULTS_ENTRY_DOC);
    into.function("results-state", results_state, RESULTS_STATE_DOC);
    into.function("results-visit", results_visit, RESULTS_VISIT_DOC);
    into.function("results-buffer", results_buffer, RESULTS_BUFFER_DOC);
    into.command("results-select", results_select, &[], RESULTS_SELECT_DOC);
    into.function("results-put", results_put, RESULTS_PUT_DOC);
    into.function(
    "results-display",
    results_display,
    RESULTS_DISPLAY_DOC,
);

into.command(
    "results-quit",
    results_quit,
    &[],
    RESULTS_QUIT_DOC,
);
}
