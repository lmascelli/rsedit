//! Completion at point: the sources a mode and every buffer declare, and the
//! runner that asks them what could go where point is, merges what they
//! answer, and puts the chosen candidate in.
//!
//! Design: `doc/architettura-editor.typ, <completamento>`.
use super::*;
use crate::editor::{COMPLETION_FILTER_FUNCTION, COMPLETION_READ_FUNCTION};
use crate::primitives::edits::{delete_range, edited, insert_text};
use risp::call_callable;

/// Where the text being completed begins, while a presenter is handed the
/// candidates, and only for that long. A presenter reads it once, as it
/// opens, to tell a completion in a buffer from one in a prompt: left set, the
/// next prompt would take a word's start in another buffer for its own.
const START_VAR: &str = "*completion-at-point-start*";

/// The text a choice replaces, between offering the candidates and hearing
/// which was chosen: `(START END LENGTH)`, LENGTH being how long the buffer
/// was when they were offered, so that END can follow what is typed in the
/// meantime. Bound at the root: the choice comes keystrokes later, after the
/// command that offered and its caller have returned.
const REGION_VAR: &str = "*completion-at-point-region*";

/// Returns the text ITEM, a candidate, completes to: the string or symbol
/// itself, or the VALUE of a `(VALUE . DESCRIPTION)` pair.
pub(super) fn candidate_value<B: BufferTrait>(item: &ELispExp<B>) -> Option<String> {
    match item {
        ELispExp::String(text) | ELispExp::Symbol(text) => Some(text.to_string()),
        ELispExp::Cons(cell) => candidate_value(&cell.car),
        _ => None,
    }
}

fn position<B: BufferTrait>(exp: &ELispExp<B>) -> Option<usize> {
    match exp {
        ELispExp::Number(n) if n.is_finite() && *n >= 0.0 => Some(*n as usize),
        _ => None,
    }
}

/// Returns a source's ANSWER taken apart, or why it makes no sense.
fn parse_answer<B: BufferTrait>(
    answer: &ELispExp<B>,
) -> Result<(usize, usize, Vec<ELispExp<B>>), String> {
    let parts = args::list(answer);
    if parts.len() < 3 {
        return Err(format!(
            "expected (START END CANDIDATES), got {} element(s)",
            parts.len()
        ));
    }
    let (Some(start), Some(end)) = (position(&parts[0]), position(&parts[1])) else {
        return Err("START and END must be non-negative numbers".into());
    };
    if end < start {
        return Err(format!("END ({end}) is before START ({start})"));
    }
    Ok((start, end, args::list(&parts[2])))
}

/// Returns the characters of the current buffer from START to END.
fn text_between<B: BufferTrait>(ctx: &EditorState<B>, start: usize, end: usize) -> String {
    ctx.with_current_buffer(|buf| buf.text.slice(start, end))
}

/// Puts TEXT in place of START to END, with point after it, or answers false
/// when the buffer refuses. Both halves are in one command, so a completion is
/// one step of undo.
fn replace_region<B: BufferTrait>(
    ctx: &EditorState<B>,
    start: usize,
    end: usize,
    text: &str,
) -> bool {
    ctx.with_current_buffer_mut(|buf| {
        // No read-only check: the doors refuse, and a second copy of their
        // rule could disagree with it. A refusal leaves point where it is;
        // the lock is held across both doors, so half a replacement never
        // lands.
        if !delete_range(buf, start, end) || !insert_text(buf, start, text) {
            return false;
        }
        let target = (start + text.chars().count()).min(buf.text.len());
        let (line, col) = buf.text.cursor_1d_to_2d(target);
        buf.text.cursor_move(line, col);
        true
    })
}

/// Returns the longest text every one of VALUES begins with.
fn common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };
    let mut prefix: Vec<char> = first.chars().collect();
    for value in &values[1..] {
        let shared = prefix
            .iter()
            .zip(value.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        prefix.truncate(shared);
    }
    prefix.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Declaring sources
// ---------------------------------------------------------------------------

pub const ADD_COMPLETION_FUNCTION_DOC: &str = "(add-completion-function MODE FUNCTION): Add \
         FUNCTION to the completion sources of the major mode MODE, after the ones it has, or to \
         those of every buffer when MODE is nil. Return t, or nil when MODE is unknown. FUNCTION \
         takes no arguments and returns nil, or (START END CANDIDATES): the positions of the text \
         it would replace, and a list of strings or (VALUE . DESCRIPTION) pairs. A mode's sources \
         are asked before the global ones.\n\n\
         Example:\n\
         (add-completion-function 'manpage-mode 'capf-manpage-names)";

primitive!(add_completion_function, args, _env, ctx, {
    exact_arity(args, 2)?;
    let mode = args::optional_name(args.first(), "Symbol, String or nil")?;
    if ctx.add_completion_function(mode.as_deref(), args[1].clone()) {
        Ok(ELispExp::t())
    } else {
        ctx.log_diagnostic(&format!("Mode {} does not exist", mode.unwrap_or_default()));
        Ok(ELispExp::nil())
    }
});

pub const SET_COMPLETION_FUNCTIONS_DOC: &str = "(set-completion-functions MODE FUNCTIONS): Make \
         the list FUNCTIONS, in the order they are to be asked, the completion sources of the \
         major mode MODE, or of every buffer when MODE is nil. Return t, or nil when MODE is \
         unknown.\n\n\
         Example:\n\
         (set-completion-functions nil '(capf-file-name capf-buffer-words))";

primitive!(set_completion_functions, args, _env, ctx, {
    exact_arity(args, 2)?;
    let mode = args::optional_name(args.first(), "Symbol, String or nil")?;
    if ctx.set_completion_functions(mode.as_deref(), args::list(&args[1])) {
        Ok(ELispExp::t())
    } else {
        ctx.log_diagnostic(&format!("Mode {} does not exist", mode.unwrap_or_default()));
        Ok(ELispExp::nil())
    }
});

pub const COMPLETION_FUNCTIONS_DOC: &str = "(completion-functions &optional MODE): Return the \
         completion sources of the major mode MODE, or of every buffer when MODE is nil or not \
         given: one of the two lists `completion-at-point' asks.\n\n\
         Example:\n\
         (completion-functions 'manpage-mode)";

primitive!(completion_functions, args, _env, ctx, {
    let mode = args::optional_name(args.first(), "Symbol, String or nil")?;
    match ctx.completion_function_list(mode.as_deref()) {
        Some(list) => Ok(ELispExp::proper_list(list)),
        None => Ok(ELispExp::nil()),
    }
});

fn current_mode<B: BufferTrait>(ctx: &EditorState<B>) -> String {
    ctx.with_current_buffer(|buf| buf.current_mode.clone())
}

// ---------------------------------------------------------------------------
// Words lying around in a buffer
// ---------------------------------------------------------------------------

/// Whether C belongs to a symbol, for every source that offers names:
/// letters, digits, `_`, `-` and `*`, which Lisp spells its names with. One
/// rule for every source, since only sources that claim the same text merge.
fn is_symbol_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '*'
}

/// Whether C belongs to a file name: a symbol's characters and those a path
/// is built of. Wider, so that a path claims other text than the symbol in it,
/// and the two sources do not merge.
fn is_file_char(c: char) -> bool {
    is_symbol_char(c) || matches!(c, '/' | '\\' | '.' | '~' | ':' | '+')
}

pub const BUFFER_WORDS_DOC: &str = "(buffer-words &optional BUFFER MINIMUM): Return the distinct \
         words of BUFFER, the current buffer by default, sorted: runs of letters, digits, _, - \
         and * at least MINIMUM characters long, 3 by default.\n\n\
         Example:\n\
         (buffer-words \"*scratch*\" 4)";

primitive!(buffer_words, args, _env, ctx, {
    if args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let buffer = args::optional_name(args.first(), "String naming a buffer")?;
    let minimum = match args.get(1) {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 1.0 => *n as usize,
        _ => 3,
    };

    // Named or current, one shape: the name is resolved first so the two
    // cases meet at a single accessor.
    let name = buffer.unwrap_or_else(|| ctx.get_current_buffer_name());
    let Some(mut words) = ctx.with_buffer(&name, |buf| {
        let mut words: Vec<String> = Vec::new();
        let mut current = String::new();
        for at in 0..buf.text.len() {
            match buf.text.at(at) {
                Some(c) if is_symbol_char(c) => current.push(c),
                _ => {
                    if current.chars().count() >= minimum {
                        words.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                }
            }
        }
        // A buffer whose last character is a word character ends mid-word, and
        // that word is as real as any other.
        if current.chars().count() >= minimum {
            words.push(current);
        }
        words
    }) else {
        return Ok(ELispExp::nil());
    };
    words.sort();
    words.dedup();
    Ok(ELispExp::proper_list(
        words.into_iter().map(ELispExp::string).collect(),
    ))
});

pub const BOUNDS_OF_THING_AT_POINT_DOC: &str = "(bounds-of-thing-at-point &optional KIND): Return \
         (START END) for the thing point is in or at the end of, or nil when there is none. KIND \
         is `symbol', the default, for a run of letters, digits, _, - and *, or `filename', which \
         takes / \\ . ~ : and + as well. A completion source of names takes its bounds from here, \
         so that it agrees with the others.\n\n\
         Example:\n\
         (bounds-of-thing-at-point 'filename)";

primitive!(bounds_of_thing_at_point, args, _env, ctx, {
    let wanted = args::optional_name(args.first(), "Symbol or String")?
        .unwrap_or_else(|| "symbol".to_string());
    let belongs: fn(char) -> bool = match wanted.as_str() {
        "symbol" => is_symbol_char,
        "filename" => is_file_char,
        other => {
            return Err(EvalError::RuntimeMessage(format!(
                "{other:?} is not a kind of thing; expected symbol or filename"
            )));
        }
    };

    let (start, end) = ctx.with_current_buffer(|buf| {
        let point = buf.text.cursor_pos_1d();
        let len = buf.text.len();
        // Backwards from point, then forwards. Both ends, rather than stopping
        // at point, because completing in the middle of a word should replace
        // the whole word -- typing `for` inside `ward` and completing should
        // not leave `forward` next to a stray `ward`.
        let mut start = point.min(len);
        while start > 0 && buf.text.at(start - 1).is_some_and(belongs) {
            start -= 1;
        }
        let mut end = point.min(len);
        while end < len && buf.text.at(end).is_some_and(belongs) {
            end += 1;
        }
        (start, end)
    });
    if start == end {
        return Ok(ELispExp::nil());
    }
    Ok(ELispExp::proper_list(vec![
        ELispExp::number(start as f64),
        ELispExp::number(end as f64),
    ]))
});

// ---------------------------------------------------------------------------
// The runner
// ---------------------------------------------------------------------------

pub const COMPLETION_AT_POINT_DOC: &str = "(completion-at-point): Complete the text at point. The \
         sources of the buffer's mode are asked, then the global ones: the first to answer \
         decides which text is completed, and later ones claiming the same text add their \
         candidates. They are matched against what is typed by `*completion-filter-function*', or \
         from the start when it is nil. One candidate is put in; several go to \
         `*completion-read-function*', or, when it is nil, as much as they all begin with is put \
         in and their number shown. C-M-i.";

primitive!(completion_at_point, _args, env, ctx, {
    // A new completion, so whatever the last one left for a late answer to
    // fill in no longer describes anything anybody is choosing for.
    env.set_root_variable(REGION_VAR.into(), ELispExp::nil());
    let mode = current_mode(ctx);
    let sources = ctx.completion_sources(&mode);

    let mut region: Option<(usize, usize)> = None;
    let mut candidates: Vec<ELispExp<B>> = Vec::new();

    for source in sources {
        let answer = call_callable(&source, &[], env.clone(), ctx)?;
        if answer.is_nil() {
            continue;
        }
        // A malformed answer costs that source, not the command. A completion
        // list is configuration in the same way a grammar is: one broken entry
        // should not be the difference between completing and not.
        match parse_answer(&answer) {
            Err(why) => ctx.log_diagnostic(&format!("Completion source {source:?}: {why}")),
            Ok((start, end, items)) => match region {
                None => {
                    region = Some((start, end));
                    candidates.extend(items);
                }
                Some(claimed) if claimed == (start, end) => candidates.extend(items),
                Some(_) => {}
            },
        }
    }

    let Some((start, end)) = region else {
        ctx.set_echo_message("No completion source has anything here");
        return Ok(ELispExp::nil());
    };

    let pattern = text_between(ctx, start, end);
    let matching = match env.get_variable(COMPLETION_FILTER_FUNCTION) {
        Some(filter) if filter.is_truthy() => args::list(&call_callable(
            &filter,
            &[
                ELispExp::string(pattern.clone()),
                ELispExp::proper_list(candidates),
            ],
            env.clone(),
            ctx,
        )?),
        _ => candidates
            .into_iter()
            .filter(|item| candidate_value(item).is_some_and(|value| value.starts_with(&pattern)))
            .collect(),
    };

    // Two sources may well offer the same name -- a function that is also a
    // word in the buffer. The first one to offer it keeps its description,
    // which is why the mode's sources are tried first.
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<ELispExp<B>> = matching
        .into_iter()
        .filter(|item| match candidate_value(item) {
            Some(value) => seen.insert(value),
            None => false,
        })
        .collect();
    let values: Vec<String> = unique.iter().filter_map(candidate_value).collect();

    match values.len() {
        0 => {
            ctx.set_echo_message(&format!("No completion for {pattern:?}"));
            Ok(ELispExp::nil())
        }
        1 => {
            if replace_region(ctx, start, end, &values[0]) {
                Ok(ELispExp::t())
            } else {
                // Through the same reporter every refused edit goes through,
                // so a read-only buffer says the one thing it always says.
                Ok(edited(ctx, false))
            }
        }
        count => {
            if let Some(present) = env.get_variable(COMPLETION_READ_FUNCTION)
                && present.is_truthy()
            {
                let length = ctx.with_current_buffer(|buf| buf.text.len());
                env.set_root_variable(
                    REGION_VAR.into(),
                    ELispExp::proper_list(vec![
                        ELispExp::number(start as f64),
                        ELispExp::number(end as f64),
                        ELispExp::number(length as f64),
                    ]),
                );
                env.set_root_variable(START_VAR.into(), ELispExp::number(start as f64));
                let presented = call_callable(
                    &present,
                    &[
                        ELispExp::proper_list(unique),
                        ELispExp::symbol("completion-at-point-choose".into()),
                    ],
                    env.clone(),
                    ctx,
                );
                // Put back whether or not the presenter succeeded: see
                // START_VAR for what it costs to leave it set.
                env.set_root_variable(START_VAR.into(), ELispExp::nil());
                presented?;
                return Ok(ELispExp::t());
            }
            // No presenter: fill in as far as the candidates agree, which is
            // what makes this useful on its own rather than only as a back end
            // for a module that may not be loaded.
            let shared = common_prefix(&values);
            if shared.chars().count() > pattern.chars().count()
                && !replace_region(ctx, start, end, &shared)
            {
                return Ok(edited(ctx, false));
            }
            ctx.set_echo_message(&format!("{count} completions"));
            Ok(ELispExp::nil())
        }
    }
});

pub const COMPLETION_AT_POINT_CHOOSE_DOC: &str = "(completion-at-point-choose VALUE): Put VALUE \
         in place of the text `completion-at-point' offered to complete, and of what has been \
         typed into it since. Return VALUE, or nil when no completion is in progress. The \
         function `*completion-read-function*' is given to call with the candidate chosen in a \
         buffer.";

primitive!(completion_at_point_choose, args, env, ctx, {
    let Some(value) = args.first().and_then(candidate_value) else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let region = env
        .get_variable(REGION_VAR)
        .map(|region| args::list(&region))
        .unwrap_or_default();
    let (Some(start), Some(end), Some(offered_length)) = (
        region.first().and_then(position),
        region.get(1).and_then(position),
        region.get(2).and_then(position),
    ) else {
        // Nothing is offering a completion, so there is nowhere to put this.
        // Inserting at point anyway would edit a buffer nobody asked to edit.
        ctx.log_diagnostic("completion-at-point-choose called with no completion in progress");
        return Ok(ELispExp::nil());
    };
    // Carried forward over whatever was typed into the word since the
    // candidates were offered -- see REGION_VAR.
    let length = ctx.with_current_buffer(|buf| buf.text.len());
    let end = (end + length).saturating_sub(offered_length).max(start);
    if !replace_region(ctx, start, end, &value) {
        return Ok(edited(ctx, false));
    }
    env.set_root_variable(REGION_VAR.into(), ELispExp::nil());
    Ok(args[0].clone())
});

/// Registers the primitives of completion at point.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // The registry of sources and the merge; what each source knows is Lisp,
    // in `completion-at-point.lisp`.
    into.command(
        "completion-at-point",
        completion_at_point,
        &[],
        COMPLETION_AT_POINT_DOC,
    );
    into.function(
        "completion-at-point-choose",
        completion_at_point_choose,
        COMPLETION_AT_POINT_CHOOSE_DOC,
    );
    into.function(
        "add-completion-function",
        add_completion_function,
        ADD_COMPLETION_FUNCTION_DOC,
    );
    into.function(
        "set-completion-functions",
        set_completion_functions,
        SET_COMPLETION_FUNCTIONS_DOC,
    );
    into.function(
        "completion-functions",
        completion_functions,
        COMPLETION_FUNCTIONS_DOC,
    );
    into.function("buffer-words", buffer_words, BUFFER_WORDS_DOC);
    into.function(
        "bounds-of-thing-at-point",
        bounds_of_thing_at_point,
        BOUNDS_OF_THING_AT_POINT_DOC,
    );
}
