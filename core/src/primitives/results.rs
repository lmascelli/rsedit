//! Searching a buffer, a file or a tree, and putting what was found where a
//! view can show it.
//!
//! # Why the tree search runs on the worker
//!
//! For the reason a shell command does: it takes as long as it takes. A few
//! thousand files is a moment, a home directory is not, and the difference is
//! not knowable before starting. So a search over files is handed to the
//! background worker and the editor carries on -- and results appear in the
//! buffer as they are found, which is also how you tell a slow search from a
//! stuck one.
//!
//! A search of one buffer is not: the text is already in memory and the scan
//! is one pass over it. Handing that to another thread would cost more than it
//! saves and would make `occur` flicker for no reason.
//!
//! # Why a buffer is searched even when it has a file
//!
//! Because the buffer is what is true. A file open with unsaved changes
//! disagrees with its disk copy, and a search that read the disk would report
//! text that is no longer there -- and, worse, offsets that land somewhere
//! else entirely when you jump to them.
use super::*;
use crate::lisp::eval;
use crate::results::{Entry, KIND_BUFFER, KIND_FILE, RESULTS_KEY, Results};
use crate::search::Pattern;
use crate::task::{ImmediateTask, WorkerMessage};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How many matches a search collects before it stops looking.
const DEFAULT_LIMIT: usize = 10_000;

/// How many files a tree search walks before it stops.
const FILE_LIMIT: usize = 20_000;

/// Directories a tree search never descends into.
///
/// The same names `find-file-recursive` skips, and for the same reason: they
/// are large *and* generated, so what is in them is either a copy of something
/// else or rebuildable. A search that walked `.git` would spend its whole
/// budget on objects before reaching a line of source.
const PRUNE: [&str; 17] = [
    ".git",
    ".hg",
    ".svn",
    ".jj",
    "target",
    "build",
    "dist",
    "out",
    "node_modules",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
];

/// Attach RESULTS to the buffer called NAME, replacing what was there.
pub(crate) fn attach<B: BufferTrait>(ctx: &EditorState<B>, name: &str, results: Results) {
    ctx.with_buffer_mut(name, |buf| buf.put_data(RESULTS_KEY, results));
}

/// Do something with the results attached to NAME.
pub(crate) fn with_results<B: BufferTrait, R>(
    ctx: &EditorState<B>,
    name: &str,
    body: impl FnOnce(&Results) -> R,
) -> Option<R> {
    ctx.with_buffer(name, |buf| buf.data::<Results>(RESULTS_KEY).map(body))
        .flatten()
}

/// Change the results attached to NAME.
pub(crate) fn with_results_mut<B: BufferTrait, R>(
    ctx: &EditorState<B>,
    name: &str,
    body: impl FnOnce(&mut Results) -> R,
) -> Option<R> {
    ctx.with_buffer_mut(name, |buf| buf.data_mut::<Results>(RESULTS_KEY).map(body))
        .flatten()
}

/// Append TEXT to the buffer called NAME, whether or not it is read-only.
///
/// The same door shell output goes through -- a results buffer is read-only
/// for the same reason a transcript is, and the thing writing it is the thing
/// that made it read-only.
fn append<B: BufferTrait>(ctx: &EditorState<B>, name: &str, text: &str) {
    let written = ctx.with_buffer_mut(name, |buf| {
        let was_read_only = buf.read_only;
        buf.read_only = false;
        let at = buf.text.len();
        let written = crate::primitives::edits::insert_text(buf, at, text);
        buf.read_only = was_read_only;
        written
    });
    if written == Some(false) {
        ctx.log_diagnostic(&format!("Could not write results into {name}"));
    }
}

/// The text of the buffer visiting PATH, when one is open and has changes the
/// file does not.
///
/// This is what makes a search of a tree agree with what is on screen. A saved
/// buffer is left to the disk read, which is the same bytes and avoids holding
/// the buffer lock for every file in a repository.
fn unsaved_text<B: BufferTrait>(ctx: &EditorState<B>, path: &Path) -> Option<String> {
    let wanted = path.to_string_lossy().to_string();
    for name in ctx.buffer_names() {
        let hit = ctx.with_buffer(&name, |buf| {
            if buf.is_modified && buf.file_path.as_deref() == Some(wanted.as_str()) {
                Some(buf.text.to_string())
            } else {
                None
            }
        });
        if let Some(Some(text)) = hit {
            return Some(text);
        }
    }
    None
}

/// Scan TEXT and add what it finds to the set attached to BUFFER, writing each
/// line into it as it goes.
///
/// Returns how many were added. Nothing is added when the buffer has gone --
/// the user killed it while the search was running, which is a reasonable
/// thing to have done and the signal to stop.
fn collect<B: BufferTrait>(
    ctx: &EditorState<B>,
    buffer: &str,
    pattern: &Pattern,
    kind: &str,
    source: &str,
    text: &str,
    room: usize,
) -> usize {
    if room == 0 {
        return 0;
    }
    let scan = pattern.scan(text, room);
    if scan.found.is_empty() {
        return 0;
    }
    let entries: Vec<Entry> = scan
        .found
        .iter()
        .map(|found| Entry::new(kind, source, found))
        .collect();
    let mut rendered = String::new();
    for entry in &entries {
        rendered.push_str(&entry.rendered());
        rendered.push('\n');
    }
    let added = entries.len();
    let stored = with_results_mut(ctx, buffer, |results| {
        results.entries.extend(entries);
        results.truncated |= scan.truncated;
    });
    if stored.is_none() {
        return 0;
    }
    append(ctx, buffer, &rendered);
    added
}

/// Searching a tree, on the worker thread.
struct TreeSearch {
    pattern: Pattern,
    root: PathBuf,
    buffer: String,
    limit: usize,
}

impl<B: BufferTrait> ImmediateTask<B> for TreeSearch {
    fn execute(self: Box<Self>, state: &EditorState<B>) {
        let prune: HashSet<String> = PRUNE.iter().map(|name| name.to_string()).collect();
        let (paths, walk_truncated) =
            crate::primitives::io::walk_files(&self.root, &prune, &[], FILE_LIMIT);
        if walk_truncated {
            with_results_mut(state, &self.buffer, |results| results.truncated = true);
        }
        let mut found = 0usize;
        for relative in paths {
            // The buffer going away is the user saying stop.
            if !state.has_buffer(&self.buffer) {
                state.finish_background_work();
                return;
            }
            if found >= self.limit {
                with_results_mut(state, &self.buffer, |results| results.truncated = true);
                break;
            }
            let path = self.root.join(&relative);
            let display = path.to_string_lossy().to_string();
            // The buffer first, when one is open and ahead of the file. See
            // the module header.
            let text = match unsaved_text(state, &path) {
                Some(text) => Some(text),
                // A file that is not text is skipped rather than reported:
                // every repository has some, and a page of "cannot read"
                // between the matches would be worse than silence.
                None => std::fs::read_to_string(&path).ok(),
            };
            let Some(text) = text else {
                continue;
            };
            found += collect(
                state,
                &self.buffer,
                &self.pattern,
                KIND_FILE,
                &display,
                &text,
                self.limit - found,
            );
        }
        finish(state, &self.buffer, found);
        state.finish_background_work();
    }
}

/// Write the trailer and mark the set finished.
///
/// Last, and in this order: the trailer is what a reader sees, and marking a
/// set done before its last line is in the buffer would let a view redraw an
/// incomplete list and call it complete.
fn finish<B: BufferTrait>(ctx: &EditorState<B>, buffer: &str, found: usize) {
    let truncated = with_results_mut(ctx, buffer, |results| {
        results.done = true;
        results.truncated
    });
    let Some(truncated) = truncated else {
        return;
    };
    let trailer = match (found, truncated) {
        (0, _) => "--- nothing found ---\n".to_string(),
        (1, false) => "--- 1 match ---\n".to_string(),
        (n, false) => format!("--- {n} matches ---\n"),
        (n, true) => format!("--- {n} matches, and the search stopped early ---\n"),
    };
    append(ctx, buffer, &trailer);
}

/// A name no buffer currently has: NAME, then `NAME<2>`, `NAME<3>`...
fn free_name<B: BufferTrait>(ctx: &EditorState<B>, name: &str) -> String {
    if !ctx.has_buffer(name) {
        return name.to_string();
    }
    for n in 2.. {
        let candidate = format!("{name}<{n}>");
        if !ctx.has_buffer(&candidate) {
            return candidate;
        }
    }
    unreachable!("the loop above returns")
}

/// Open the buffer a search will write into, with its header line in place.
fn open_results<B: BufferTrait>(
    ctx: &EditorState<B>,
    name: &str,
    mode: &str,
    results: Results,
) -> String {
    let name = free_name(ctx, name);
    ctx.new_buffer(&name, None, Some(mode.to_string()));
    append(
        ctx,
        &name,
        &format!("{} in {}\n", results.pattern, results.over),
    );
    attach(ctx, &name, results);
    ctx.with_buffer_mut(&name, |buf| buf.read_only = true);
    name
}

fn pattern_arg<B: BufferTrait>(args: &[ELispExp<B>]) -> Result<String, EvalError<EditorState<B>>> {
    match args.first() {
        Some(ELispExp::String(text)) => Ok(text.to_string()),
        other => Err(EvalError::WrongArgumentType {
            expected: "String".into(),
            got: other.cloned().unwrap_or_else(ELispExp::nil),
        }),
    }
}

fn mode_arg<B: BufferTrait>(args: &[ELispExp<B>], index: usize, fallback: &str) -> String {
    match args.get(index) {
        Some(ELispExp::Symbol(name)) | Some(ELispExp::String(name)) if !name.is_empty() => {
            name.to_string()
        }
        _ => fallback.to_string(),
    }
}

pub const OCCUR_SCAN_DOC: &str = "(occur--scan PATTERN &optional REGEXP MODE): Find every line of this \
         buffer matching PATTERN and list them in a buffer of their own. Returns that buffer's \
         name.\n\n\
         Each line is written as `SOURCE:LINE: TEXT', and the whole list is also attached to \
         the buffer as a *result set*: `next-error', `previous-error' and anything else that \
         walks places read it from there rather than parsing the text back, so a list can say \
         exactly where it means.\n\n\
         With REGEXP non-nil, PATTERN is a regular expression. Case is folded as \
         `case-fold-search' says. MODE is the major mode the listing opens in, and defaults to \
         `occur-mode' -- a module gives the listing its own keys and colouring by passing one, \
         the way `compile' does for a shell command.\n\n\
         Done at once rather than on the worker: the text is already in memory and the scan is \
         one pass over it.\n\n\
         What `occur' in the `occur' module is built on: this finds and attaches, that shows \
         the listing and takes you to it.\n\n\
         Example:\n\
         (occur--scan \"TODO\")\n\
         (occur--scan \"fn [a-z_]+\" t)";

primitive!(occur_scan, args, env, ctx, {
    let source_pattern = pattern_arg(args)?;
    let regexp = args.get(1).is_some_and(|value| value.is_truthy());
    let mode = mode_arg(args, 2, "occur-mode");
    let pattern = Pattern::new(&source_pattern, regexp, crate::isearch::case_fold(&env))
        .map_err(EvalError::RuntimeMessage)?;
    let source = ctx.get_current_buffer_name();
    let text = ctx.with_current_buffer(|buf| buf.text.to_string());
    ctx.consume_fuel(u32::try_from(text.chars().count()).unwrap_or(u32::MAX))?;

    let buffer = open_results(
        ctx,
        "*Occur*",
        &mode,
        Results::new(&source_pattern, &source),
    );
    let found = collect(
        ctx,
        &buffer,
        &pattern,
        KIND_BUFFER,
        &source,
        &text,
        DEFAULT_LIMIT,
    );
    finish(ctx, &buffer, found);
    ctx.set_current_results(Some(buffer.clone()));
    Ok(ELispExp::string(buffer))
});

pub const GREP_SCAN_DOC: &str = "(grep--scan PATTERN &optional DIRECTORY REGEXP MODE): Find every line under \
         DIRECTORY -- the current directory when it is omitted -- matching PATTERN, and list \
         them in a buffer of their own. Returns that buffer's name at once, while the search is \
         still running.\n\n\
         The search is done by the background worker, so the editor stays usable, and results \
         appear as they are found -- which is also how you tell a slow search from a stuck one. \
         A `--- 12 matches ---' trailer is written when it finishes.\n\n\
         A file open in a buffer with unsaved changes is searched *as the buffer has it*, not as \
         the disk does, so what is reported is what is on screen. Files that are not text are \
         skipped without comment; every repository has some.\n\n\
         `.git', `target', `node_modules' and the rest of the large generated directories are \
         not descended into, the same list `find-file-recursive' skips.\n\n\
         Entries are attached to the buffer as a result set, so `next-error' walks them. Killing \
         the buffer stops the search, which is the only way to call one off.\n\n\
         What `grep' in the `occur' module is built on.\n\n\
         Example:\n\
         (grep--scan \"TODO\")\n\
         (grep--scan \"fn [a-z_]+\" \"src\" t)";

primitive!(grep_scan, args, env, ctx, {
    let source_pattern = pattern_arg(args)?;
    let directory = match args.get(1) {
        Some(ELispExp::String(path)) if !path.is_empty() => path.to_string(),
        _ => ".".to_string(),
    };
    let regexp = args.get(2).is_some_and(|value| value.is_truthy());
    let mode = mode_arg(args, 3, "occur-mode");
    let pattern = Pattern::new(&source_pattern, regexp, crate::isearch::case_fold(&env))
        .map_err(EvalError::RuntimeMessage)?;
    let root = PathBuf::from(crate::primitives::io::expand_path(&directory));
    if !root.is_dir() {
        ctx.set_echo_message(&format!("{directory} is not a directory"));
        return Ok(ELispExp::nil());
    }

    let buffer = open_results(
        ctx,
        "*Grep*",
        &mode,
        Results::new(&source_pattern, &root.to_string_lossy()),
    );
    // Counted before the task is sent, as the shell command's is: the worker
    // may not reach it for a moment, and a frame drawn in that moment would
    // decide nothing was running and go back to sleep until a key was pressed.
    ctx.begin_background_work();
    if !ctx.send_to_worker(WorkerMessage::RunNow(Box::new(TreeSearch {
        pattern,
        root,
        buffer: buffer.clone(),
        limit: DEFAULT_LIMIT,
    }))) {
        ctx.finish_background_work();
        ctx.set_echo_message("The background worker has gone");
        return Ok(ELispExp::nil());
    }
    ctx.set_current_results(Some(buffer.clone()));
    Ok(ELispExp::string(buffer))
});

// ---------------------------------------------------------------------------
// Walking a set
// ---------------------------------------------------------------------------
//
// Every command below works on *the current set* -- the listing most recently
// produced -- rather than on the buffer it is called in. That is what `M-g n`
// means: it is pressed in the file being fixed, not in the listing, so the
// listing cannot be read off the screen.
//
// These are primitives rather than a module's functions so that every producer
// of a list gets them without depending on the module that happens to have
// been written first. A compilation, a search and a linter each build a set
// and walk it with the same three keys.

/// The overlay marking the line being visited, in the file.
const TARGET_CATEGORY: &str = "results-target";
/// The overlay marking the entry being visited, in the listing.
const HERE_CATEGORY: &str = "results-here";
/// Where these marks sit among overlapping ones. Above a manual page's
/// emphasis (10), the same height compilation's own mark used.
const MARK_PRIORITY: i32 = 20;

/// Which buffer's set the walking commands work on.
fn current_buffer_of_results<B: BufferTrait>(ctx: &EditorState<B>) -> Option<String> {
    let name = ctx.current_results()?;
    // A listing that has been killed is not the current one any more. Asked
    // here rather than remembered, because nothing tells this when a buffer
    // goes.
    ctx.has_buffer(&name).then_some(name)
}

/// Mark the line at POINT of the current buffer under CATEGORY.
///
/// Silently does nothing when no face of that name exists: the marks are a
/// view's business, and a view that has not said how they look has not asked
/// for them. That is also what keeps these commands working with no module
/// loaded at all.
fn mark_line<B: BufferTrait>(ctx: &EditorState<B>, category: &str) {
    let Some(face) = crate::ui::Face::named(category) else {
        return;
    };
    ctx.with_current_buffer_mut(|buf| {
        buf.overlays.remove_category(Some(category));
        let point = buf.text.cursor_pos_1d();
        let line = buf.text.cursor_1d_to_2d(point).0;
        let start = buf.text.cursor_2d_to_1d(line, 0);
        let end = start + crate::primitives::edits::line_length(&buf.text, line);
        if end > start {
            buf.overlays
                .add(start, end, face, MARK_PRIORITY, category.into());
        }
    });
}

/// Take the mark off whatever was marked last under CATEGORY.
fn unmark<B: BufferTrait>(ctx: &EditorState<B>, buffer: &str, category: &str) {
    ctx.with_buffer_mut(buffer, |buf| {
        buf.overlays.remove_category(Some(category));
    });
}

/// Go to ENTRY: open what it is in, put point on it, and mark the line.
fn visit<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    entry: &Entry,
) -> Result<bool, EvalError<EditorState<B>>> {
    if entry.kind == KIND_FILE {
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
            .min(crate::primitives::edits::line_length(&buf.text, line));
        buf.text.cursor_move(line, column);
    });
    mark_line(ctx, TARGET_CATEGORY);
    Ok(true)
}

/// Visit entry INDEX of the set in BUFFER, and remember that we did.
fn visit_index<B: BufferTrait>(
    ctx: &EditorState<B>,
    env: &std::sync::Arc<Env<EditorState<B>>>,
    buffer: &str,
    index: usize,
) -> Result<ELispExp<B>, EvalError<EditorState<B>>> {
    let Some(entry) = with_results(ctx, buffer, |results| results.get(index).cloned()).flatten()
    else {
        return Ok(ELispExp::nil());
    };
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
    // The listing's own mark, put on the line for this entry. The header line
    // is why the offset is one more than the index.
    let here = ctx.get_current_buffer_name();
    ctx.with_buffer_mut(buffer, |buf| {
        buf.text.cursor_move(index + 1, 0);
    });
    if ctx.has_buffer(buffer) {
        let was = ctx.get_current_buffer_name();
        ctx.switch_to_buffer(buffer);
        mark_line(ctx, HERE_CATEGORY);
        ctx.switch_to_buffer(&was);
    }
    let _ = here;
    if !visit(ctx, env, &entry)? {
        return Ok(ELispExp::nil());
    }
    Ok(entry_form(&entry))
}

/// One entry as Lisp sees it. The shape is `primitives::scan`'s, said once.
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

/// Which buffer a query was asked about: the named one, the current listing,
/// or the buffer it was asked in.
fn asked_about<B: BufferTrait>(
    args: &[ELispExp<B>],
    index: usize,
    ctx: &EditorState<B>,
) -> Option<String> {
    match args.get(index) {
        Some(ELispExp::String(name)) if !name.is_empty() => Some(name.to_string()),
        Some(ELispExp::Symbol(name)) => Some(name.to_string()),
        _ => current_buffer_of_results(ctx)
            .or_else(|| Some(ctx.get_current_buffer_name()).filter(|name| has_results(ctx, name))),
    }
}

fn has_results<B: BufferTrait>(ctx: &EditorState<B>, name: &str) -> bool {
    with_results(ctx, name, |_| ()).is_some()
}

pub const RESULTS_COUNT_DOC: &str = "(results-count &optional BUFFER): How many entries the \
         result set in BUFFER has, or nil when it has none.\n\n\
         BUFFER defaults to the listing `next-error' is walking, and failing that to the \
         current buffer. Nil for a buffer that is not a listing at all.\n\n\
         Example:\n\
         (results-count) => 12";

primitive!(results_count, args, _env, ctx, {
    let Some(buffer) = asked_about(args, 0, ctx) else {
        return Ok(ELispExp::nil());
    };
    Ok(match with_results(ctx, &buffer, |results| results.len()) {
        Some(count) => ELispExp::number(count as f64),
        None => ELispExp::nil(),
    })
});

pub const RESULTS_ENTRY_DOC: &str = "(results-entry N &optional BUFFER): Entry N of the result \
         set in BUFFER, counting from 0, or nil.\n\n\
         An entry is (KIND SOURCE LINE COLUMN OFFSET TEXT MATCH-START MATCH-END) -- the same \
         shape `scan-buffer' answers in, so a view written for one list works for every \
         list.\n\n\
         Read one at a time rather than the lot: a search of a large tree has thousands, and a \
         view shows a screenful.\n\n\
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

pub const RESULTS_STATE_DOC: &str = "(results-state &optional BUFFER): What the result set in \
         BUFFER is, as (PATTERN OVER COUNT DONE TRUNCATED CURRENT), or nil when there is \
         none.\n\n\
         DONE is nil while a search is still running, which is how a view says \"still \
         looking\" rather than looking empty. TRUNCATED is t when a limit stopped it. CURRENT is \
         the index last visited, or nil.\n\n\
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

pub const RESULTS_VISIT_DOC: &str = "(results-visit N &optional BUFFER): Go to entry N: open what \
         it is in, put point on the line, and mark it. Returns the entry, or nil.\n\n\
         The walk's position moves to N, so `next-error' carries on from there. Marking is done \
         through the faces `results-target' (in what was opened) and `results-here' (in the \
         listing); a view that has not named those faces simply gets no marks.\n\n\
         Example:\n\
         (results-visit 0)";

primitive!(results_visit, args, env, ctx, {
    let index = match args.first() {
        Some(ELispExp::Number(n)) if *n >= 0.0 => *n as usize,
        _ => return Ok(ELispExp::nil()),
    };
    let Some(buffer) = asked_about(args, 1, ctx) else {
        return Ok(ELispExp::nil());
    };
    visit_index(ctx, &env, &buffer, index)
});

/// The name a listing files its refresher under. See [`refresh`].
const REFRESH_KEY: &str = "lisp:results-refresh";

/// Give a listing whose contents are still arriving a chance to say what it
/// has now.
///
/// # Why a list may be out of date at all
///
/// A search knows everything it found the moment it finishes. A *compilation*
/// does not: its output arrives line by line from a process, and the list of
/// complaints grows with it. Rebuilding that list on every line would be a
/// parse per line of output; not rebuilding it would mean `M-g n` walking what
/// the compiler had said thirty seconds ago.
///
/// So a producer whose list can grow attaches a function under
/// `results-refresh`, with `buffer-put`, and it is called -- with the
/// listing's name -- immediately before each walk. A producer whose list is
/// complete attaches nothing and pays nothing.
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

/// `next-error` and `previous-error`, which differ by a sign.
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
    visit_index(ctx, env, &buffer, next)
}

pub const NEXT_ERROR_DOC: &str = "(next-error): Go to the next entry of the current result set. \
         Returns it, or nil at the end.\n\n\
         The list it walks is whichever was made last -- a search, a compilation, anything that \
         built one -- and *not* the buffer this is called in: the point of the key is that it \
         is pressed in the file being fixed.\n\n\
         The end of the list is said rather than wrapped around to the start.\n\n\
         Example:\n\
         (define-key nil \"M-g n\" 'next-error)";

primitive!(next_error, _args, env, ctx, {
    step_and_visit(ctx, &env, 1)
});

pub const PREVIOUS_ERROR_DOC: &str =
    "(previous-error): Go to the entry before the current one. Returns it, or nil at the start.";

primitive!(previous_error, _args, env, ctx, {
    step_and_visit(ctx, &env, -1)
});

pub const RESULTS_BUFFER_DOC: &str = "(results-buffer): The name of the listing `next-error' is \
         walking, or nil when there is none or it has been killed.\n\n\
         Example:\n\
         (results-buffer) => \"*Grep*\"";

primitive!(results_buffer, _args, _env, ctx, {
    Ok(match current_buffer_of_results(ctx) {
        Some(name) => ELispExp::string(name),
        None => ELispExp::nil(),
    })
});

pub const RESULTS_SELECT_DOC: &str = "(results-select &optional BUFFER): Make BUFFER -- the \
         current one when it is omitted -- the listing `next-error' walks. Returns its name, or \
         nil when it holds no result set.\n\n\
         What `C-x `' would be: going back to an older listing and walking that one instead.\n\n\
         Example:\n\
         (results-select \"*Grep*\")";

primitive!(results_select, args, _env, ctx, {
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
         ENTRIES to BUFFER as a result set, and make it the one `next-error' walks. Returns how \
         many were attached.\n\n\
         How anything that is not a search joins in: a compilation parses its own output, a \
         linter reads its own format, and each attaches what it found in the shape every view \
         and every key already understands. Nothing is written into the buffer -- whatever \
         produced the text has already done that.\n\n\
         A list that is still growing -- a compilation's, while it runs -- puts a function \
         under `results-refresh' with `buffer-put', and it is called with the buffer's name \
         before each walk, so `next-error' sees what has arrived since.\n\n\
         ENTRIES is a list of (KIND SOURCE LINE COLUMN OFFSET TEXT MATCH-START MATCH-END). \
         Everything after SOURCE may be omitted: LINE defaults to 1 and the rest to 0 or to the \
         empty string, so a producer that knows only a file and a line says just that.\n\n\
         Example:\n\
         (results-put \"cargo build\" \"the project\"\n\
                       '((\"file\" \"src/main.rs\" 12)))";

primitive!(results_put, args, _env, ctx, {
    if args.len() < 3 {
        return Err(EvalError::WrongNumberOfArguments {
            expected: 3,
            got: args.len(),
        });
    }
    let pattern = match &args[0] {
        ELispExp::String(text) => text.to_string(),
        other => crate::lisp::lisp_display(other),
    };
    let over = match &args[1] {
        ELispExp::String(text) => text.to_string(),
        other => crate::lisp::lisp_display(other),
    };
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
        });
    }
    let count = results.len();
    results.done = true;
    // Where the walk had got to survives being replaced. A list that grows --
    // a compilation's, rebuilt before each step -- is attached again every
    // time, and a walk that started over each time would offer the first
    // complaint for ever while looking like it was working.
    //
    // The index rather than the entry, because a growing list grows at the
    // end: entry three is still entry three. A producer that reorders its
    // list will find the walk approximately where it was, which is better
    // than at the top.
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
