//! Completion at point: the sources a mode and every buffer declare, and the
//! runner that asks them what could go where point is, merges what they
//! answer, and puts the chosen candidate in.
//!
//! Design: `doc/architettura-editor.typ, <completamento>`.
use super::*;
use crate::editor::{
    COMPLETION_AT_POINT_WAIT, COMPLETION_FILTER_FUNCTION, COMPLETION_INVALIDATED_FUNCTION,
    COMPLETION_READ_FUNCTION, DEFAULT_COMPLETION_AT_POINT_WAIT, DirectoryScope, scoped_directory,
};
use crate::primitives::edits::{delete_range, edited, insert_text};
use crate::work::{Job, JobError, Spec, Step, StepContext, WorkId};
use risp::call_callable;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Where the text being completed begins, while a presenter is handed the
/// candidates, and only for that long. A presenter reads it once, as it
/// opens, to tell a completion in a buffer from one in a prompt: left set, the
/// next prompt would take a word's start in another buffer for its own.
const START_VAR: &str = "*completion-at-point-start*";

/// The text a choice replaces, between offering the candidates and hearing
/// which was chosen: `(START END LENGTH BUFFER)`, LENGTH being how long the
/// buffer was when they were offered, so that END can follow what is typed in
/// the meantime, and BUFFER the one they were offered for, followed by
/// `ON-CHOSEN CANDIDATES` when `completion-offer` was given a function to
/// call. Bound at the root: the choice comes keystrokes later, after the
/// command that offered and its caller have returned.
const REGION_VAR: &str = "*completion-at-point-region*";

/// The job that asks the sources left over, by its name, and the topic that
/// wakes it to ask the next.
const JOB_NAME: &str = "completion-at-point";
const ASKED_TOPIC: &str = "completion-at-point-asked";

/// What the job calls between commands to ask the next source.
const CONTINUE: &str = "completion-at-point--continue";

/// A completion whose sources are still being asked, as [`Pending`] writes
/// it: `(BUFFER LENGTH SOURCES REGION CANDIDATES PRESENTED)`. Bound at the
/// root, as REGION_VAR is, and nil when nothing is left to ask.
const PENDING_VAR: &str = "*completion-at-point-pending*";

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

pub const ADD_COMPLETION_FUNCTION_DOC: &str = "(add-completion-function MODE FUNCTION &optional \
         PRIORITY): Add FUNCTION to the completion sources of the major mode MODE, after the ones \
         it has, or to those of every buffer when MODE is nil, and give it PRIORITY when that is \
         given, as `set-completion-priority' does. Return t, or nil when MODE is unknown. FUNCTION \
         takes no arguments and returns nil, or (START END CANDIDATES): the positions of the text \
         it would replace, and a list of strings or (VALUE . DESCRIPTION) pairs. Sources are \
         asked from the highest priority down, and of one priority a mode's before the global \
         ones.\n\n\
         Example:\n\
         (add-completion-function 'manpage-mode 'capf-manpage-names)";

primitive!(add_completion_function, args, _env, ctx, {
    if !(2..=3).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: if args.len() < 2 { 2 } else { 3 },
            got: args.len(),
        });
    }
    let mode = args::optional_name(args.first(), "Symbol, String or nil")?;
    let priority = args.get(2).map(priority).transpose()?;
    if ctx.add_completion_function(mode.as_deref(), args[1].clone()) {
        if let Some(priority) = priority {
            ctx.set_completion_priority(args[1].clone(), priority);
        }
        Ok(ELispExp::t())
    } else {
        ctx.log_diagnostic(&format!("Mode {} does not exist", mode.unwrap_or_default()));
        Ok(ELispExp::nil())
    }
});

pub const SET_COMPLETION_FUNCTIONS_DOC: &str = "(set-completion-functions MODE FUNCTIONS &key \
         :global): Make the list FUNCTIONS, in the order they are to be asked, the completion \
         sources of the major mode MODE, or of every buffer when MODE is nil. :global nil makes \
         them the only sources of MODE, with the global ones never asked; :global t asks those \
         after them again, as at first. Return t, or nil when MODE is unknown.\n\n\
         Example:\n\
         (set-completion-functions 'eshell-mode '(eshell-completion) :global nil)";

primitive!(set_completion_functions, args, _env, ctx, {
    let (args, keywords) =
        args::with_keywords("set-completion-functions", args, 2, 2, &[":global"])?;
    let mode = args::optional_name(args.first(), "Symbol, String or nil")?;
    let global = keywords.get(":global").map(ELispExp::is_truthy);
    if global.is_some() && mode.is_none() {
        return Err(EvalError::RuntimeMessage(
            "set-completion-functions: :global is for the sources of a mode".into(),
        ));
    }
    if ctx.set_completion_functions(mode.as_deref(), args::list(&args[1]), global) {
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

/// Reads a source's priority: any number but an infinite one.
fn priority<B: BufferTrait>(exp: &ELispExp<B>) -> Result<f64, EvalError<EditorState<B>>> {
    match exp {
        ELispExp::Number(n) if n.is_finite() => Ok(*n),
        other => Err(EvalError::WrongArgumentType {
            expected: "Number".into(),
            got: other.clone(),
        }),
    }
}

pub const SET_COMPLETION_PRIORITY_DOC: &str = "(set-completion-priority FUNCTION PRIORITY): Give \
         FUNCTION, a completion source, PRIORITY, a number, in every list of sources it is or \
         will be in, and return PRIORITY. `completion-at-point' asks the sources from the highest \
         priority down, and those of one priority in the order of their lists; a source never \
         given one has 0.\n\n\
         Example:\n\
         (set-completion-priority 'capf-buffer-words -100)";

primitive!(set_completion_priority, args, _env, ctx, {
    exact_arity(args, 2)?;
    ctx.set_completion_priority(args[0].clone(), priority(&args[1])?);
    Ok(args[1].clone())
});

pub const COMPLETION_PRIORITY_DOC: &str = "(completion-priority FUNCTION): Return the priority of \
         FUNCTION as a completion source: the one it was last given, or 0.\n\n\
         Example:\n\
         (completion-priority 'capf-symbols)";

primitive!(completion_priority, args, _env, ctx, {
    exact_arity(args, 1)?;
    Ok(ELispExp::number(ctx.completion_priority(&args[0])))
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
         sources of the buffer's mode are asked, and the global ones unless the mode's are its \
         only ones, from the highest priority down: the first to answer decides which text is \
         completed, and later ones claiming the same text add their candidates. Those still to \
         be asked after a moment are left to a job, which asks one between each command and the \
         next and adds what it answers to the candidates on show, which can be chosen \
         meanwhile. They are matched against what is typed by `*completion-filter-function*', or \
         from the start when it is nil. One candidate is put in; several go to \
         `*completion-read-function*', or, when it is nil, as much as they all begin with is put \
         in and their number shown. C-M-i.";

primitive!(completion_at_point, _args, env, ctx, {
    // A new completion, so whatever the last one left for a late answer to
    // fill in no longer describes anything anybody is choosing for, nor what
    // it still had to ask.
    env.set_root_variable(REGION_VAR.into(), ELispExp::nil());
    env.set_root_variable(PENDING_VAR.into(), ELispExp::nil());
    let mode = current_mode(ctx);
    let mut sources = ctx.completion_sources(&mode).into_iter();

    let mut region: Option<(usize, usize)> = None;
    let mut candidates: Vec<ELispExp<B>> = Vec::new();

    // As many as answer within `completion-at-point-wait', which is most of
    // the time all of them, and then complete as if there were no job; the
    // rest are left to one, so that a slow source does not keep the keys.
    let wait = Duration::from_secs_f64(
        env.number_at_least(COMPLETION_AT_POINT_WAIT, 0.0)
            .unwrap_or(DEFAULT_COMPLETION_AT_POINT_WAIT)
            .min(1e9)
            / 1000.0,
    );
    let began = Instant::now();
    for source in sources.by_ref() {
        ask(&source, &mut region, &mut candidates, &env, ctx)?;
        if began.elapsed() >= wait {
            break;
        }
    }
    let rest: Vec<ELispExp<B>> = sources.collect();
    if !rest.is_empty() {
        let job = Asking {
            directory: scoped_directory(),
            id: None,
        };
        if ctx
            .work()
            .submit(Spec::new(JOB_NAME).on(ASKED_TOPIC), Box::new(job))
            .is_some()
        {
            let mut pending = Pending {
                buffer: ctx.get_current_buffer_name(),
                length: ctx.with_current_buffer(|buf| buf.text.len()),
                sources: rest,
                region,
                candidates,
                presented: false,
            };
            if let Err(error) = pending.show(&env, ctx) {
                ctx.work().cancel_named(JOB_NAME);
                return Err(error);
            }
            pending.write(&env);
            if !pending.presented {
                ctx.set_echo_message("Looking for completions...");
            }
            return Ok(ELispExp::t());
        }
        // No job can be started: asked here, as if they had all answered at
        // once.
        for source in rest {
            ask(&source, &mut region, &mut candidates, &env, ctx)?;
        }
    }

    let Some((start, end)) = region else {
        ctx.set_echo_message("No completion source has anything here");
        return Ok(ELispExp::nil());
    };
    offer(start, end, candidates, None, env.clone(), ctx)
});

/// Asks SOURCE what could go at point. The first answer makes its text
/// REGION; an answer claiming that text adds its candidates to CANDIDATES,
/// and one claiming other text is passed over.
fn ask<B: BufferTrait>(
    source: &ELispExp<B>,
    region: &mut Option<(usize, usize)>,
    candidates: &mut Vec<ELispExp<B>>,
    env: &Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> Result<(), EvalError<EditorState<B>>> {
    let answer = call_callable(source, &[], env.clone(), ctx)?;
    if answer.is_nil() {
        return Ok(());
    }
    // A malformed answer costs that source, not the command. A completion
    // list is configuration in the same way a grammar is: one broken entry
    // should not be the difference between completing and not.
    match parse_answer(&answer) {
        Err(why) => ctx.log_diagnostic(&format!("Completion source {source:?}: {why}")),
        Ok((start, end, items)) => match region {
            None => {
                *region = Some((start, end));
                candidates.extend(items);
            }
            Some(claimed) if *claimed == (start, end) => candidates.extend(items),
            Some(_) => {}
        },
    }
    Ok(())
}

/// A completion whose sources the job is still asking, kept in PENDING_VAR
/// between one and the next.
struct Pending<B: BufferTrait> {
    /// The buffer it completes in.
    buffer: String,
    /// How long that buffer was when REGION last held.
    length: usize,
    /// The sources still to ask, in the order to ask them.
    sources: Vec<ELispExp<B>>,
    region: Option<(usize, usize)>,
    /// What the sources asked so far have offered for REGION.
    candidates: Vec<ELispExp<B>>,
    /// Whether the presenter has been handed them.
    presented: bool,
}

impl<B: BufferTrait> Pending<B> {
    /// Returns what PENDING_VAR holds, or `None` when nothing is left to ask.
    fn read(env: &Arc<Env<EditorState<B>>>) -> Option<Self> {
        let parts = args::list(&env.get_variable(PENDING_VAR)?);
        let [buffer, length, sources, region, candidates, presented] = parts.as_slice() else {
            return None;
        };
        let ELispExp::String(buffer) = buffer else {
            return None;
        };
        let region = match args::list(region).as_slice() {
            [start, end] => Some((position(start)?, position(end)?)),
            _ => None,
        };
        Some(Self {
            buffer: buffer.to_string(),
            length: position(length)?,
            sources: args::list(sources),
            region,
            candidates: args::list(candidates),
            presented: presented.is_truthy(),
        })
    }

    /// Keeps it in PENDING_VAR, for the job's next call.
    fn write(&self, env: &Arc<Env<EditorState<B>>>) {
        let region = match self.region {
            Some((start, end)) => ELispExp::proper_list(vec![
                ELispExp::number(start as f64),
                ELispExp::number(end as f64),
            ]),
            None => ELispExp::nil(),
        };
        env.set_root_variable(
            PENDING_VAR.into(),
            ELispExp::proper_list(vec![
                ELispExp::string(self.buffer.clone()),
                ELispExp::number(self.length as f64),
                ELispExp::proper_list(self.sources.clone()),
                region,
                ELispExp::proper_list(self.candidates.clone()),
                ELispExp::boolean(self.presented),
            ]),
        );
    }

    /// Shows the candidates found so far, when there is a presenter and they
    /// are a choice: handed to it the first time, as [`offer`] hands them,
    /// and to `*completion-invalidated-function*` after that, so that the
    /// list on screen grows and what is on it can still be chosen. One alone
    /// is not shown, since a source still to be asked may make it a choice.
    fn show(
        &mut self,
        env: &Arc<Env<EditorState<B>>>,
        ctx: &EditorState<B>,
    ) -> Result<(), EvalError<EditorState<B>>> {
        let Some((start, end)) = self.region else {
            return Ok(());
        };
        let (_, unique) = matching(start, end, self.candidates.clone(), env, ctx)?;
        if self.presented {
            if let Some(update) = env.get_variable(COMPLETION_INVALIDATED_FUNCTION)
                && update.is_truthy()
            {
                call_callable(&update, &[ELispExp::proper_list(unique)], env.clone(), ctx)?;
            }
        } else if unique.len() > 1 {
            self.presented = present(start, end, unique, None, env, ctx)?;
        }
        Ok(())
    }
}

/// Forgets the sources left to ask, and stops the job that asks them.
fn stop_asking<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>, ctx: &EditorState<B>) {
    env.set_root_variable(PENDING_VAR.into(), ELispExp::nil());
    ctx.work().cancel_named(JOB_NAME);
}

/// The job that has the sources `completion-at-point` left over asked. It
/// asks nothing itself: a source is Lisp that reads the current buffer, which
/// is the one the thread that runs commands has made current, so each step
/// owes that thread a call to CONTINUE, which asks one, and waits until it
/// has.
struct Asking {
    /// The directory the completion was started in, which each call runs in.
    directory: Option<String>,
    id: Option<WorkId>,
}

impl<B: BufferTrait> Job<EditorState<B>> for Asking {
    fn step(&mut self, cx: &mut StepContext<'_, EditorState<B>>) -> Result<Step, JobError> {
        let id = *self.id.get_or_insert(cx.id);
        let _scope = DirectoryScope::enter(self.directory.clone());
        cx.state
            .owe_callback(JOB_NAME, id, ELispExp::symbol(CONTINUE.into()), Vec::new());
        // Woken by CONTINUE once it has asked, rather than stepping again at
        // once: one source each time that thread comes round, and the keys
        // typed in between handled in between.
        Ok(Step::Wait)
    }
}

const COMPLETION_AT_POINT_CONTINUE_DOC: &str = "(completion-at-point--continue): Ask the next of \
         the sources `completion-at-point' left to its job, and show what it adds; with the \
         last, complete as `completion-at-point' does. Return t while sources are left. The job \
         calls it between commands.";

primitive!(completion_at_point_continue, _args, env, ctx, {
    let Some(mut pending) = Pending::read(&env) else {
        // A later completion, or the end of this one, has taken what was left.
        ctx.work().cancel_named(JOB_NAME);
        return Ok(ELispExp::nil());
    };
    let (length, point) = ctx.with_current_buffer(|buf| (buf.text.len(), buf.text.cursor_pos_1d()));
    // Nobody is waiting for more once a candidate has been chosen, or point
    // has gone to another buffer or back past the text being completed.
    let chosen = pending.presented
        && env
            .get_variable(REGION_VAR)
            .is_none_or(|region| region.is_nil());
    let gone = ctx.get_current_buffer_name() != pending.buffer
        || pending.region.is_some_and(|(start, _)| point < start);
    if chosen || gone || pending.sources.is_empty() {
        stop_asking(&env, ctx);
        return Ok(ELispExp::nil());
    }
    // Carried forward over what was typed since, as REGION_VAR's end is.
    if let Some((start, end)) = pending.region {
        let end = (end + length).saturating_sub(pending.length).max(start);
        pending.region = Some((start, end));
    }
    pending.length = length;
    let source = pending.sources.remove(0);
    // A source that fails costs itself and not the ones after it: there is no
    // command left for the error to stop.
    if let Err(error) = ask(
        &source,
        &mut pending.region,
        &mut pending.candidates,
        &env,
        ctx,
    ) {
        ctx.log_diagnostic(&format!("Completion source {source:?}: {error:?}"));
    }
    if !pending.sources.is_empty() {
        let shown = pending.show(&env, ctx);
        pending.write(&env);
        ctx.work().topic(ASKED_TOPIC).emit();
        shown?;
        return Ok(ELispExp::t());
    }
    stop_asking(&env, ctx);
    if pending.presented {
        pending.show(&env, ctx)?;
        return Ok(ELispExp::nil());
    }
    match pending.region {
        Some((start, end)) => {
            offer(start, end, pending.candidates, None, env.clone(), ctx)?;
        }
        None => ctx.set_echo_message("No completion source has anything here"),
    }
    Ok(ELispExp::nil())
});

/// Offers CANDIDATES for START to END of the current buffer: matches them
/// against what is typed there, drops repeated values, and puts one in or
/// hands several to the presenter. ON_CHOSEN, when given, is called with the
/// candidate as offered once one has been put in.
fn offer<B: BufferTrait>(
    start: usize,
    end: usize,
    candidates: Vec<ELispExp<B>>,
    on_chosen: Option<ELispExp<B>>,
    env: Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let (pattern, unique) = matching(start, end, candidates, &env, ctx)?;
    let values: Vec<String> = unique.iter().filter_map(candidate_value).collect();

    match values.len() {
        0 => {
            ctx.set_echo_message(&format!("No completion for {pattern:?}"));
            Ok(ELispExp::nil())
        }
        1 => {
            if replace_region(ctx, start, end, &values[0]) {
                if let Some(on_chosen) = on_chosen {
                    call_callable(&on_chosen, &[unique[0].clone()], env.clone(), ctx)?;
                }
                Ok(ELispExp::t())
            } else {
                // Through the same reporter every refused edit goes through,
                // so a read-only buffer says the one thing it always says.
                Ok(edited(ctx, false))
            }
        }
        count => {
            if present(start, end, unique, on_chosen, &env, ctx)? {
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
}

/// Returns the text from START to END of the current buffer, and those of
/// CANDIDATES that match it, each value once.
fn matching<B: BufferTrait>(
    start: usize,
    end: usize,
    candidates: Vec<ELispExp<B>>,
    env: &Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> Result<(String, Vec<ELispExp<B>>), EvalError<EditorState<B>>> {
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
    Ok((pattern, unique))
}

/// Hands UNIQUE, the candidates for START to END, to
/// `*completion-read-function*`, and returns whether there is one to hand
/// them to. ON_CHOSEN is as for [`offer`].
fn present<B: BufferTrait>(
    start: usize,
    end: usize,
    unique: Vec<ELispExp<B>>,
    on_chosen: Option<ELispExp<B>>,
    env: &Arc<Env<EditorState<B>>>,
    ctx: &EditorState<B>,
) -> Result<bool, EvalError<EditorState<B>>> {
    let Some(present) = env
        .get_variable(COMPLETION_READ_FUNCTION)
        .filter(ELispExp::is_truthy)
    else {
        return Ok(false);
    };
    let length = ctx.with_current_buffer(|buf| buf.text.len());
    let mut recorded = vec![
        ELispExp::number(start as f64),
        ELispExp::number(end as f64),
        ELispExp::number(length as f64),
        ELispExp::string(ctx.get_current_buffer_name()),
    ];
    if let Some(on_chosen) = on_chosen {
        recorded.push(on_chosen);
        recorded.push(ELispExp::proper_list(unique.clone()));
    }
    env.set_root_variable(REGION_VAR.into(), ELispExp::proper_list(recorded));
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
    Ok(true)
}

pub const COMPLETION_OFFER_DOC: &str = "(completion-offer START END CANDIDATES &optional \
         ON-CHOSEN): Offer CANDIDATES for the text from START to END of the current buffer, as \
         `completion-at-point' offers what its sources answer: matched against what is typed, \
         one put in, several handed to `*completion-read-function*'. ON-CHOSEN is then called \
         with the candidate put in, as offered. For a source that answers later; the caller \
         checks that buffer and point are still the ones it asked about.\n\n\
         Example:\n\
         (completion-offer 0 2 '(\"abcd\" \"abce\") 'message)";

primitive!(completion_offer, args, env, ctx, {
    if !(3..=4).contains(&args.len()) {
        return Err(EvalError::WrongNumberOfArguments {
            expected: if args.len() < 3 { 3 } else { 4 },
            got: args.len(),
        });
    }
    let length = ctx.with_current_buffer(|buf| buf.text.len());
    let within = |exp: &ELispExp<B>| position(exp).filter(|at| *at <= length);
    let Some(start) = within(&args[0]) else {
        return Err(EvalError::WrongArgumentType {
            expected: "a position in the current buffer".into(),
            got: args[0].clone(),
        });
    };
    let Some(end) = within(&args[1]).filter(|end| *end >= start) else {
        return Err(EvalError::WrongArgumentType {
            expected: "a position in the current buffer, not before START".into(),
            got: args[1].clone(),
        });
    };
    let on_chosen = args.get(3).filter(|f| f.is_truthy()).cloned();
    // A new offer, as in `completion_at_point': whatever the last one left
    // describes nothing anybody is choosing for.
    env.set_root_variable(REGION_VAR.into(), ELispExp::nil());
    env.set_root_variable(PENDING_VAR.into(), ELispExp::nil());
    offer(
        start,
        end,
        args::list(&args[2]),
        on_chosen,
        env.clone(),
        ctx,
    )
});

pub const COMPLETION_AT_POINT_CHOOSE_DOC: &str = "(completion-at-point-choose VALUE): Put VALUE \
         in place of the text `completion-at-point' or `completion-offer' offered to complete, \
         and of what has been typed into it since, then call the function given to \
         `completion-offer', if any. Return VALUE, or nil when no completion is in progress. \
         `*completion-read-function*' is given it to call with the candidate chosen in a buffer.";

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
    // START and END are positions in the buffer offered for, not in this one.
    if region.get(3).and_then(candidate_value) != Some(ctx.get_current_buffer_name()) {
        ctx.log_diagnostic("completion-at-point-choose called in another buffer");
        return Ok(ELispExp::nil());
    }
    // Carried forward over whatever was typed into the word since the
    // candidates were offered -- see REGION_VAR.
    let length = ctx.with_current_buffer(|buf| buf.text.len());
    let end = (end + length).saturating_sub(offered_length).max(start);
    if !replace_region(ctx, start, end, &value) {
        return Ok(edited(ctx, false));
    }
    // Consumed before ON-CHOSEN runs, so that it may offer again.
    env.set_root_variable(REGION_VAR.into(), ELispExp::nil());
    if let Some(on_chosen) = region.get(4).filter(|f| f.is_truthy()) {
        // The candidate as offered, description and all, when the presenter
        // answered with one of them; what it answered otherwise.
        let offered = region
            .get(5)
            .map(args::list)
            .unwrap_or_default()
            .into_iter()
            .find(|item| candidate_value(item).as_deref() == Some(value.as_str()))
            .unwrap_or_else(|| args[0].clone());
        call_callable(on_chosen, &[offered], env.clone(), ctx)?;
    }
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
    into.function("completion-offer", completion_offer, COMPLETION_OFFER_DOC);
    into.function(
        CONTINUE,
        completion_at_point_continue,
        COMPLETION_AT_POINT_CONTINUE_DOC,
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
    into.function(
        "set-completion-priority",
        set_completion_priority,
        SET_COMPLETION_PRIORITY_DOC,
    );
    into.function(
        "completion-priority",
        completion_priority,
        COMPLETION_PRIORITY_DOC,
    );
    into.function("buffer-words", buffer_words, BUFFER_WORDS_DOC);
    into.function(
        "bounds-of-thing-at-point",
        bounds_of_thing_at_point,
        BOUNDS_OF_THING_AT_POINT_DOC,
    );
}
