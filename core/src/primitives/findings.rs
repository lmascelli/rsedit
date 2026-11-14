//! The findings from Lisp: a producer replaces what it says about a file with
//! `finding-set`, and anyone asks for findings by file, by producer or by
//! directory, each as a property list.
//!
//! Design: `doc/architettura-editor.typ, <segnalazioni>`.
use super::*;
use crate::editor::findings::Located;
use crate::managers::{Finding, LineColumn, Severity};
use crate::primitives::args;

/// Returns the wrong-type error for GOT, which should have been EXPECTED.
fn wrong<B: BufferTrait>(expected: &str, got: &ELispExp<B>) -> EvalError<EditorState<B>> {
    EvalError::WrongArgumentType {
        expected: expected.into(),
        got: got.clone(),
    }
}

/// Returns the value after KEY in the property list ITEMS, if KEY is there.
fn property<B: BufferTrait>(items: &[ELispExp<B>], key: &str) -> Option<ELispExp<B>> {
    items
        .chunks(2)
        .find(|pair| matches!(&pair[0], ELispExp::Symbol(name) if name.as_str() == key))
        .and_then(|pair| pair.get(1).cloned())
}

/// Returns EXP, a whole number not below 0, as a count.
fn count<B: BufferTrait>(exp: &ELispExp<B>) -> Option<usize> {
    match exp {
        ELispExp::Number(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 => Some(*n as usize),
        _ => None,
    }
}

/// Returns EXP, a `(LINE . COLUMN)` pair, as a line and column.
fn line_column<B: BufferTrait>(exp: &ELispExp<B>) -> Result<LineColumn, EvalError<EditorState<B>>> {
    let expected = "(LINE . COLUMN), two whole numbers not below 0";
    match exp {
        ELispExp::Cons(cell) => match (count(&cell.car), count(&cell.cdr)) {
            (Some(line), Some(column)) => Ok((line, column)),
            _ => Err(wrong(expected, exp)),
        },
        _ => Err(wrong(expected, exp)),
    }
}

/// Returns ENTRY, a finding's property list, as a finding.
fn finding<B: BufferTrait>(entry: &ELispExp<B>) -> Result<Finding, EvalError<EditorState<B>>> {
    let (items, tail) = entry.split_list();
    if items.is_empty() || !tail.is_nil() {
        return Err(wrong("a property list", entry));
    }
    let start = match property(&items, ":start") {
        Some(start) => line_column(&start)?,
        None => return Err(wrong("a :start", entry)),
    };
    let end = match property(&items, ":end") {
        Some(end) if !end.is_nil() => line_column(&end)?,
        _ => start,
    };
    let severity = match property(&items, ":severity") {
        Some(ELispExp::Symbol(name)) => Severity::from_name(&name),
        _ => None,
    };
    let Some(severity) = severity else {
        return Err(wrong("a :severity of error, warning, info or hint", entry));
    };
    let message = match property(&items, ":message") {
        Some(ELispExp::String(text)) => text.to_string(),
        Some(other) => return Err(wrong("String", &other)),
        None => return Err(wrong("a :message", entry)),
    };
    let code = match property(&items, ":code") {
        None => None,
        Some(code) if code.is_nil() => None,
        Some(ELispExp::String(text)) | Some(ELispExp::Symbol(text)) => Some(text.to_string()),
        Some(ELispExp::Number(n)) if n.fract() == 0.0 && n.abs() < 1e15 => {
            Some(format!("{}", n as i64))
        }
        Some(ELispExp::Number(n)) => Some(n.to_string()),
        Some(other) => return Err(wrong("String, Symbol or Number", &other)),
    };
    Ok(Finding {
        start,
        end,
        severity,
        message,
        code,
    })
}

/// Returns PAIR as `(LINE . COLUMN)`.
fn pair<B: BufferTrait>((line, column): LineColumn) -> ELispExp<B> {
    ELispExp::cons(
        ELispExp::number(line as f64),
        ELispExp::number(column as f64),
    )
}

/// Returns LOCATED as the property list the queries answer with.
fn plist<B: BufferTrait>(located: Located) -> ELispExp<B> {
    let key = |name: &str| ELispExp::symbol(name.into());
    let Located {
        file,
        producer,
        finding,
        stale,
    } = located;
    ELispExp::proper_list(vec![
        key(":file"),
        ELispExp::string(file),
        key(":producer"),
        ELispExp::symbol(producer),
        key(":start"),
        pair(finding.start),
        key(":end"),
        pair(finding.end),
        key(":severity"),
        key(finding.severity.name()),
        key(":message"),
        ELispExp::string(finding.message),
        key(":code"),
        finding
            .code
            .map(ELispExp::string)
            .unwrap_or_else(ELispExp::nil),
        key(":stale"),
        if stale {
            ELispExp::t()
        } else {
            ELispExp::nil()
        },
    ])
}

/// Returns the findings of SETS as a list of property lists, charging one
/// fuel unit for each.
fn answer<B: BufferTrait>(
    ctx: &EditorState<B>,
    sets: Vec<crate::managers::Found>,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let located = ctx.locate_findings(sets);
    ctx.consume_fuel(u32::try_from(located.len()).unwrap_or(u32::MAX))?;
    Ok(ELispExp::proper_list(
        located.into_iter().map(plist).collect(),
    ))
}

/// Returns the argument at AT, which must be a string, as an absolute path.
fn path_argument<B: BufferTrait>(
    ctx: &EditorState<B>,
    args: &[ELispExp<B>],
    at: usize,
) -> Result<String, EvalError<EditorState<B>>> {
    Ok(ctx.expand_path(&args::text(args.get(at))?))
}

/// Returns an error unless there are between MIN and MAX ARGS.
fn arity<B: BufferTrait>(
    args: &[ELispExp<B>],
    min: usize,
    max: usize,
) -> Result<(), EvalError<EditorState<B>>> {
    if args.len() < min || args.len() > max {
        return Err(EvalError::WrongNumberOfArguments {
            expected: if args.len() < min { min } else { max },
            got: args.len(),
        });
    }
    Ok(())
}

pub const FINDING_SET_DOC: &str = "(finding-set PRODUCER FILE ENTRIES): Replace what PRODUCER, \
         a symbol or a string, says about FILE with ENTRIES, and return how many there are now. \
         Each entry is a property list with :start and :end, (LINE . COLUMN) with lines from 1 \
         and columns from 0 in characters, :end being :start unless given; :severity, one of \
         `error', `warning', `info', `hint'; :message, a string; and :code, optional. nil \
         ENTRIES forgets FILE. In an open file the entries follow the edits.\n\n\
         Example:\n\
         (finding-set 'my-lint \"/tmp/a.c\"\n\
           '((:start (3 . 4) :end (3 . 9) :severity warning :message \"unused\" :code \"W1\")))";

primitive!(finding_set, args, _env, ctx, {
    arity(args, 3, 3)?;
    let producer = args::name(args.first())?;
    let file = path_argument(ctx, args, 1)?;
    let (entries, tail) = args[2].split_list();
    if !tail.is_nil() {
        return Err(wrong("a list of property lists", &args[2]));
    }
    ctx.consume_fuel(u32::try_from(entries.len()).unwrap_or(u32::MAX))?;
    let findings = entries.iter().map(finding).collect::<Result<Vec<_>, _>>()?;
    let count = findings.len();
    ctx.set_findings(&producer, &file, findings);
    Ok(ELispExp::number(count as f64))
});

pub const FINDING_CLEAR_DOC: &str = "(finding-clear PRODUCER &optional FILE): Forget what \
         PRODUCER says about FILE, or about every file when FILE is nil, and return nil.\n\n\
         Example:\n\
         (finding-clear 'my-lint)";

primitive!(finding_clear, args, _env, ctx, {
    arity(args, 1, 2)?;
    let producer = args::name(args.first())?;
    let file = match args.get(1) {
        Some(file) if !file.is_nil() => Some(path_argument(ctx, args, 1)?),
        _ => None,
    };
    ctx.clear_findings(&producer, file.as_deref());
    Ok(ELispExp::nil())
});

pub const FINDINGS_FOR_FILE_DOC: &str = "(findings-for-file FILE): Return the findings about \
         FILE, by producer: property lists with :file, :producer, :start, :end, :severity, \
         :message, :code and :stale, t when the file's text has changed since the producer \
         spoke.\n\n\
         Example:\n\
         (findings-for-file (buffer-file-name))";

primitive!(findings_for_file, args, _env, ctx, {
    arity(args, 1, 1)?;
    let file = path_argument(ctx, args, 0)?;
    let sets = ctx.findings(|all| all.for_file(&file));
    answer(ctx, sets)
});

pub const FINDINGS_OF_PRODUCER_DOC: &str = "(findings-of-producer PRODUCER): Return the \
         findings PRODUCER has made, by file, as `findings-for-file' does.\n\n\
         Example:\n\
         (length (findings-of-producer 'my-lint))";

primitive!(findings_of_producer, args, _env, ctx, {
    arity(args, 1, 1)?;
    let producer = args::name(args.first())?;
    let sets = ctx.findings(|all| all.of_producer(&producer));
    answer(ctx, sets)
});

pub const FINDINGS_UNDER_DOC: &str = "(findings-under DIRECTORY): Return the findings about the \
         files in DIRECTORY and below it, by file and then by producer, as `findings-for-file' \
         does. DIRECTORY is a whole name: /a/b holds /a/b/c.rs, not /a/bc.\n\n\
         Example:\n\
         (findings-under \"~/src\")";

primitive!(findings_under, args, _env, ctx, {
    arity(args, 1, 1)?;
    let directory = path_argument(ctx, args, 0)?;
    let sets = ctx.findings(|all| all.under(&directory));
    answer(ctx, sets)
});

/// Registers the primitives of the findings.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("finding-set", finding_set, FINDING_SET_DOC);
    into.function("finding-clear", finding_clear, FINDING_CLEAR_DOC);
    into.function(
        "findings-for-file",
        findings_for_file,
        FINDINGS_FOR_FILE_DOC,
    );
    into.function(
        "findings-of-producer",
        findings_of_producer,
        FINDINGS_OF_PRODUCER_DOC,
    );
    into.function("findings-under", findings_under, FINDINGS_UNDER_DOC);
}
