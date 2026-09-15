//! Completion at point: asking a set of functions what could go where the
//! cursor is, and putting one of their answers there.
//!
//! # The protocol
//!
//! A **completion function** takes no arguments. It is called with its buffer
//! current and point where the user left it, and returns either `nil` -- "not
//! mine" -- or
//!
//! ```text
//!     (START END CANDIDATES)
//! ```
//!
//! START and END are buffer positions: the text this source is offering to
//! replace. CANDIDATES is a list of strings, or of `(VALUE . DESCRIPTION)`
//! pairs, which is the shape `*completion-read-function*` already shows.
//!
//! # Why the bounds, and not just the candidates
//!
//! Because only the source knows what it is completing. In `foo-ba|` the
//! thing being completed is a symbol; in `/usr/lo|` it is the last path
//! component and not the slash before it; inside a string it may be the whole
//! string. A source that returned only candidates would leave this function
//! to *guess* which characters they were meant to replace -- one guess, shared
//! by every source, and wrong for all but one of them.
//!
//! # Why filtering happens here and not in the sources
//!
//! A source answers "what could go here", not "what matches what has been
//! typed". Keeping the match in one place means there is one answer to what
//! matching means, and one place to change it: bind
//! `*completion-filter-function*` and every source becomes fuzzy at once,
//! without any of them being touched.
//!
//! # Why the sources are merged rather than raced
//!
//! Emacs takes the first function that answers and stops. That works there
//! because its sources are specific to a mode. Here the useful answer to
//! `comp` in a Lisp buffer is *both* the functions called `comp...` and the
//! words already in the buffer, and a race would silently hide one of them
//! depending on the order of a list the user never sees.
//!
//! So: the first source to answer fixes the region, and every later source
//! that claims **the same region** has its candidates appended. A source
//! claiming a different region -- a file path where another saw a symbol --
//! is passed over, because two sources completing different spans of text
//! cannot both be right.
use super::*;
use crate::lisp::call_callable;
use crate::primitives::edits::{delete_range, edited, insert_text};

/// Where the region being completed is recorded between offering the
/// candidates and hearing which one was chosen.
///
/// Variables rather than a field, for the reason the minibuffer keeps
/// `*minibuffer-completions*` in one: a presenter may be a Lisp module that
/// takes several keystrokes to make up its mind, so the state has to outlive
/// this call and be visible to anyone debugging it.
const START_VAR: &str = "*completion-at-point-start*";
const END_VAR: &str = "*completion-at-point-end*";

/// A Lisp function of (PATTERN CANDIDATES) returning the candidates that
/// match. Unset, the match is a plain prefix.
const FILTER_HOOK: &str = "*completion-filter-function*";

/// The presenter, shared with the minibuffer -- see `crate::minibuffer`.
const PRESENT_HOOK: &str = "*completion-read-function*";

/// Read a list that may have arrived as data or as syntax.
///
/// A list written `'(a b)` in source is a cons chain, but the same list built
/// by a macro's backquote is still a `Form`. A caller asking for "a list"
/// means both.
fn as_list<B: BufferTrait>(exp: &ELispExp<B>) -> Vec<ELispExp<B>> {
    match exp {
        ELispExp::Form(items) => items.to_vec(),
        other if other.is_nil() => Vec::new(),
        other => other.iter().collect(),
    }
}

/// The text a candidate completes to, whatever shape it arrived in.
fn candidate_value<B: BufferTrait>(item: &ELispExp<B>) -> Option<String> {
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

/// Take a source's answer apart, or say why it made no sense.
fn parse_answer<B: BufferTrait>(
    answer: &ELispExp<B>,
) -> Result<(usize, usize, Vec<ELispExp<B>>), String> {
    let parts = as_list(answer);
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
    Ok((start, end, as_list(&parts[2])))
}

/// The characters of the current buffer between two offsets.
fn text_between<B: BufferTrait>(ctx: &EditorState<B>, start: usize, end: usize) -> String {
    let buf = ctx.get_current_buffer();
    let buf = buf.read().expect("read lock on buffer");
    let end = end.min(buf.text.len());
    (start.min(end)..end)
        .filter_map(|at| buf.text.at(at))
        .collect()
}

/// Put TEXT where the region was, and leave point after it.
///
/// Both halves run inside one command, so undo already groups them: a
/// completion is undone in one step without this having to say so. See
/// `UndoHistory::record`.
fn replace_region<B: BufferTrait>(
    ctx: &EditorState<B>,
    start: usize,
    end: usize,
    text: &str,
) -> bool {
    let buf = ctx.get_current_buffer();
    let mut buf = buf.write().expect("write lock on buffer");
    // No `read_only` check here, deliberately. The refusal lives at the two
    // doors and this goes through them, so asking again would be a second
    // copy of the rule -- and a second copy is one that can disagree. What is
    // needed instead is to notice they refused, so that point does not travel
    // to the end of text that was never inserted: in a read-only buffer point
    // moves freely, and it should stay exactly where the user put it.
    // Both doors report a refusal the same way and both refuse for the same
    // one reason, so either answering false means the buffer is protected.
    // They cannot disagree between these two lines: the write lock is held
    // across both, so there is no window in which half the replacement lands.
    if !delete_range(&mut buf, start, end) || !insert_text(&mut buf, start, text) {
        return false;
    }
    let target = (start + text.chars().count()).min(buf.text.len());
    let (line, col) = buf.text.cursor_1d_to_2d(target);
    buf.text.cursor_move(line, col);
    true
}

/// The longest text every candidate begins with.
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
         FUNCTION to the completion sources tried in major mode MODE, or -- when MODE is nil -- \
         to the ones tried in every buffer. Returns t, or nil (logging a diagnostic) if MODE \
         names an unknown mode.\n\n\
         FUNCTION takes no arguments. It is called with its buffer current and point where the \
         user left it, and returns nil if it has nothing to offer there, or a list \
         (START END CANDIDATES): the buffer positions of the text it is offering to replace, and \
         a list of strings or (VALUE . DESCRIPTION) pairs.\n\n\
         A mode's own sources are tried before the global ones. `nil' means global here for the \
         same reason it does in `define-key'.\n\n\
         Example:\n\
         (defun capf-greeting ()\n\
           (list (point) (point) '((\"hello\" . \"a greeting\"))))\n\
         (add-completion-function nil 'capf-greeting)";

primitive!(add_completion_function, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let mode = mode_argument(&args[0])?;
    if ctx.add_completion_function(mode.as_deref(), args[1].clone()) {
        Ok(ELispExp::t())
    } else {
        ctx.log_diagnostic(&format!("Mode {} does not exist", mode.unwrap_or_default()));
        Ok(ELispExp::nil())
    }
});

pub const SET_COMPLETION_FUNCTIONS_DOC: &str = "(set-completion-functions MODE FUNCTIONS): Replace \
         the whole list of completion sources for major mode MODE, or the global list when MODE \
         is nil. Returns t, or nil if MODE is unknown.\n\n\
         This is how a source is removed, or the order of two of them changed -- neither of \
         which `add-completion-function' can express. The order matters: the first source to \
         answer decides which text is being completed, and later sources only contribute if they \
         agree.\n\n\
         Example:\n\
         (set-completion-functions nil '(capf-file-name capf-buffer-words))";

primitive!(set_completion_functions, args, _env, ctx, {
    if args.len() != 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let mode = mode_argument(&args[0])?;
    if ctx.set_completion_functions(mode.as_deref(), as_list(&args[1])) {
        Ok(ELispExp::t())
    } else {
        ctx.log_diagnostic(&format!("Mode {} does not exist", mode.unwrap_or_default()));
        Ok(ELispExp::nil())
    }
});

pub const COMPLETION_FUNCTIONS_DOC: &str = "(completion-functions &optional MODE): The completion \
         sources declared for major mode MODE, or the global ones when MODE is nil or omitted. \
         One list on its own, not the merged list `completion-at-point' actually tries.\n\n\
         Example:\n\
         (completion-functions 'risp-mode)";

primitive!(completion_functions, args, _env, ctx, {
    let mode = match args.first() {
        None => None,
        Some(exp) => mode_argument(exp)?,
    };
    match ctx.completion_function_list(mode.as_deref()) {
        Some(list) => Ok(ELispExp::proper_list(list)),
        None => Ok(ELispExp::nil()),
    }
});

/// A mode argument that may be a symbol, a string, or nil for "global".
fn mode_argument<B: BufferTrait>(
    exp: &ELispExp<B>,
) -> Result<Option<String>, EvalError<EditorState<B>>> {
    match exp {
        other if other.is_nil() => Ok(None),
        ELispExp::Symbol(name) | ELispExp::String(name) => Ok(Some(name.to_string())),
        other => Err(EvalError::WrongArgumentType {
            expected: "Symbol, String or nil".into(),
            got: other.clone(),
        }),
    }
}

fn current_mode<B: BufferTrait>(ctx: &EditorState<B>) -> String {
    ctx.get_current_buffer()
        .read()
        .expect("read lock on buffer")
        .current_mode
        .clone()
}

// ---------------------------------------------------------------------------
// Words lying around in a buffer
// ---------------------------------------------------------------------------

/// What counts as one symbol, for every source that offers names.
///
/// Letters, digits, `_`, `-` and `*`. The last two are in because this
/// editor's own language spells almost every name with a hyphen and several
/// with stars -- `*completion-read-function*` is one identifier, not four --
/// and code that uses them as operators does not write `a-b` against a letter
/// often enough for the occasional stray token to matter.
///
/// **One rule, used by all of them**, and that is the point rather than an
/// economy: `completion-at-point` merges only the sources that claim the same
/// span of text, so a source scanning back over a different set of characters
/// would silently drop out of every list. It is not a syntax table and does
/// not pretend to be one; when it becomes one, it becomes one here.
fn is_symbol_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '*'
}

/// What counts as part of a file name: a symbol, plus the characters a path is
/// built out of. Deliberately wider, so that a path claims a different span
/// from the symbol inside it and the two sources do not merge.
fn is_file_char(c: char) -> bool {
    is_symbol_char(c) || matches!(c, '/' | '\\' | '.' | '~' | ':' | '+')
}

pub const BUFFER_WORDS_DOC: &str = "(buffer-words &optional BUFFER MINIMUM): Every distinct word \
         in BUFFER (default: the current buffer), sorted, as a list of strings. A word is a run \
         of symbol characters -- see `bounds-of-thing-at-point'. MINIMUM, if given, is the shortest word \
         worth reporting (default 3).\n\n\
         In Rust because this reads a whole buffer: doing it in Lisp means `buffer-string' \
         copying every character into one value first, for every buffer, on a key press.\n\n\
         Example:\n\
         (buffer-words \"*scratch*\" 4)";

primitive!(buffer_words, args, _env, ctx, {
    if args.len() > 2 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 2,
            got: args.len(),
        });
    }
    let buffer = match args.first() {
        None => None,
        Some(exp) if exp.is_nil() => None,
        Some(ELispExp::String(name)) | Some(ELispExp::Symbol(name)) => Some(name.to_string()),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "String naming a buffer".into(),
                got: other.clone(),
            });
        }
    };
    let minimum = match args.get(1) {
        Some(ELispExp::Number(n)) if n.is_finite() && *n >= 1.0 => *n as usize,
        _ => 3,
    };

    let Some(handle) = (match buffer {
        Some(name) => ctx.get_buffer(&name),
        None => Some(ctx.get_current_buffer()),
    }) else {
        return Ok(ELispExp::nil());
    };

    let mut words: Vec<String> = {
        let buf = handle.read().expect("read lock on buffer");
        let mut words = Vec::new();
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
    };
    words.sort();
    words.dedup();
    Ok(ELispExp::proper_list(
        words.into_iter().map(ELispExp::string).collect(),
    ))
});

pub const BOUNDS_OF_THING_AT_POINT_DOC: &str = "(bounds-of-thing-at-point &optional KIND): The \
         positions (START END) of the thing point is inside or at the end of, or nil if there is \
         none there.\n\n\
         KIND is `symbol' (the default) for a run of letters, digits, `_', `-' and `*', or \
         `filename' for those plus the characters a path is made of. Every completion source \
         that offers names should use the same KIND: `completion-at-point' merges the sources \
         that claim the same text, so one scanning back over a different set of characters would \
         quietly drop out of the list.\n\n\
         Example:\n\
         (bounds-of-thing-at-point 'symbol) => (12 18)";

primitive!(bounds_of_thing_at_point, args, _env, ctx, {
    let wanted = match args.first() {
        None => "symbol".to_string(),
        Some(exp) if exp.is_nil() => "symbol".to_string(),
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) => name.to_string(),
        Some(other) => {
            return Err(EvalError::WrongArgumentType {
                expected: "Symbol or String".into(),
                got: other.clone(),
            });
        }
    };
    let belongs: fn(char) -> bool = match wanted.as_str() {
        "symbol" => is_symbol_char,
        "filename" => is_file_char,
        other => {
            return Err(EvalError::RuntimeMessage(format!(
                "{other:?} is not a kind of thing; expected symbol or filename"
            )));
        }
    };

    let buf = ctx.get_current_buffer();
    let buf = buf.read().expect("read lock on buffer");
    let point = buf.text.cursor_pos_1d();
    let len = buf.text.len();

    // Backwards from point, then forwards. Both ends, rather than stopping at
    // point, because completing in the middle of a word should replace the
    // whole word -- typing `for` inside `ward` and completing should not leave
    // `forward` next to a stray `ward`.
    let mut start = point.min(len);
    while start > 0 && buf.text.at(start - 1).is_some_and(belongs) {
        start -= 1;
    }
    let mut end = point.min(len);
    while end < len && buf.text.at(end).is_some_and(belongs) {
        end += 1;
    }
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

pub const COMPLETION_AT_POINT_DOC: &str = "(completion-at-point): Complete the text at point, \
         asking this buffer's mode and then the global list what could go there.\n\n\
         Each source is called with no arguments and answers nil, or (START END CANDIDATES). The \
         first to answer decides which text is being completed; later sources claiming the same \
         text have their candidates added, and ones claiming different text are passed over. The \
         candidates are then filtered against what is already written there -- by \
         `*completion-filter-function*' if it is bound, and by a plain prefix match if not.\n\n\
         One candidate is inserted. Several are shown by `*completion-read-function*' if a module \
         has bound it; with nothing bound, as much of the candidates as they all agree on is \
         inserted and the number left is reported -- so this works with no module loaded.\n\n\
         Example:\n\
         (define-key nil \"C-M-i\" 'completion-at-point)";

primitive!(completion_at_point, _args, env, ctx, {
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
    let matching = match env.get_variable(FILTER_HOOK) {
        Some(filter) if filter.is_truthy() => as_list(&call_callable(
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
            if let Some(present) = env.get_variable(PRESENT_HOOK)
                && present.is_truthy()
            {
                env.set_variable(START_VAR.into(), ELispExp::number(start as f64));
                env.set_variable(END_VAR.into(), ELispExp::number(end as f64));
                call_callable(
                    &present,
                    &[
                        ELispExp::proper_list(unique),
                        ELispExp::symbol("completion-at-point-choose".into()),
                    ],
                    env.clone(),
                    ctx,
                )?;
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
         where the text `completion-at-point' offered to complete was. Returns VALUE.\n\n\
         This is the symbol handed to `*completion-read-function*' as the thing to call when the \
         user picks a candidate, so that a presenter needs to know nothing about buffers or \
         about where the candidates came from -- the same presenter serves this and the \
         minibuffer, which hands over `minibuffer-choose-completion' instead.";

primitive!(completion_at_point_choose, args, env, ctx, {
    let Some(value) = args.first().and_then(candidate_value) else {
        return Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: args.first().cloned().unwrap_or_else(ELispExp::nil),
        });
    };
    let (Some(start), Some(end)) = (
        env.get_variable(START_VAR).as_ref().and_then(position),
        env.get_variable(END_VAR).as_ref().and_then(position),
    ) else {
        // Nothing is offering a completion, so there is nowhere to put this.
        // Inserting at point anyway would edit a buffer nobody asked to edit.
        ctx.log_diagnostic("completion-at-point-choose called with no completion in progress");
        return Ok(ELispExp::nil());
    };
    if !replace_region(ctx, start, end, &value) {
        return Ok(edited(ctx, false));
    }
    env.set_variable(START_VAR.into(), ELispExp::nil());
    env.set_variable(END_VAR.into(), ELispExp::nil());
    Ok(args[0].clone())
});
