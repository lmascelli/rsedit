//! Searching a buffer, a file or a tree, and putting what was found where a
//! view can show it.
//!
//! # Why the tree search runs on a thread of its own
//!
//! For the reason a shell command does: it takes as long as it takes. A few
//! thousand files is a moment, a home directory is not, and the difference is
//! not knowable before starting. So a search over files gets a dedicated
//! thread of the WorkManager, named after its listing, and the editor carries
//! on -- and results appear in the buffer as they are found, which is also how
//! you tell a slow search from a stuck one. `stop-worker` with the listing's
//! name stops it, and so does closing the listing.
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
use risp::eval;
use crate::primitives::args;
use crate::text::results::{Entry, KIND_BUFFER, KIND_FILE, RESULTS_KEY, Results};
use crate::text::search::Pattern;
use crate::work::{Stop, WorkId};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// The buffer a search writes into, and who writes: the thread searching a
/// tree, or nobody for a search done here, at once.
#[derive(Clone, Copy)]
struct Listing<'a> {
    buffer: &'a str,
    writer: Option<WorkId>,
}

impl Listing<'_> {
    /// [`with_results_mut`], only while the buffer is still the writer's. See
    /// `EditorState::append_as_writer`.
    fn with_results_mut<B: BufferTrait, R>(
        self,
        ctx: &EditorState<B>,
        body: impl FnOnce(&mut Results) -> R,
    ) -> Option<R> {
        ctx.with_buffer_mut(self.buffer, |buf| {
            if self.writer.is_some_and(|writer| buf.writer != Some(writer)) {
                return None;
            }
            buf.data_mut::<Results>(RESULTS_KEY).map(body)
        })
        .flatten()
    }

    /// Append TEXT to the buffer, as its writer when there is one.
    fn write<B: BufferTrait>(self, ctx: &EditorState<B>, text: &str) {
        match self.writer {
            Some(writer) => {
                if ctx.append_as_writer(self.buffer, writer, text) {
                    ctx.changed_in_background();
                }
            }
            None => {
                ctx.append_to_buffer(self.buffer, text);
            }
        }
    }
}

/// Scan TEXT and add what it finds to the set attached to the LISTING's
/// buffer, writing each line into it as it goes.
///
/// Returns how many were added. Nothing is added when the buffer has gone --
/// the user closed it while the search was running, which is a reasonable
/// thing to have done and the signal to stop -- or when it is no longer the
/// writer's.
fn collect<B: BufferTrait>(
    ctx: &EditorState<B>,
    listing: Listing,
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
    let stored = listing.with_results_mut(ctx, |results| {
        results.entries.extend(entries);
        results.truncated |= scan.truncated;
    });
    if stored.is_none() {
        return 0;
    }
    listing.write(ctx, &rendered);
    added
}

/// Searching a tree, on a thread of its own.
pub(crate) struct TreeSearch {
    pub(crate) pattern: Pattern,
    pub(crate) root: PathBuf,
    pub(crate) buffer: String,
    pub(crate) limit: usize,
}

impl TreeSearch {
    /// Walk the tree and list what matches in the buffer, until the walk is
    /// done or STOP is raised. ME is the thread's own number: what the buffer
    /// knows its writer by.
    ///
    /// Stopped, it ends as a search that hit its limit does: what was found
    /// stands, and the trailer says the search stopped early.
    pub(crate) fn run<B: BufferTrait>(self, state: &EditorState<B>, stop: &Stop, me: WorkId) {
        let listing = Listing {
            buffer: &self.buffer,
            writer: Some(me),
        };
        let prune: HashSet<String> = PRUNE.iter().map(|name| name.to_string()).collect();
        let (paths, walk_truncated) =
            crate::primitives::io::walk_files(&self.root, &prune, &[], FILE_LIMIT, &|| {
                !stop.is_cancelled()
            });
        if walk_truncated {
            listing.with_results_mut(state, |results| results.truncated = true);
        }
        let mut found = 0usize;
        for relative in paths {
            // Between two files: where a search can be put down.
            if stop.is_cancelled() {
                listing.with_results_mut(state, |results| results.truncated = true);
                break;
            }
            if found >= self.limit {
                listing.with_results_mut(state, |results| results.truncated = true);
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
                listing,
                &self.pattern,
                KIND_FILE,
                &display,
                &text,
                self.limit - found,
            );
        }
        finish(state, listing, found);
    }
}

/// Write the trailer and mark the set finished.
///
/// Last, and in this order: the trailer is what a reader sees, and marking a
/// set done before its last line is in the buffer would let a view redraw an
/// incomplete list and call it complete.
fn finish<B: BufferTrait>(ctx: &EditorState<B>, listing: Listing, found: usize) {
    let truncated = listing.with_results_mut(ctx, |results| {
        results.done = true;
        results.truncated
    });
    let Some(truncated) = truncated else {
        return;
    };
    let trailer = match (found, truncated) {
        (0, false) => "--- nothing found ---\n".to_string(),
        (0, true) => "--- nothing found, and the search stopped early ---\n".to_string(),
        (1, false) => "--- 1 match ---\n".to_string(),
        (1, true) => "--- 1 match, and the search stopped early ---\n".to_string(),
        (n, false) => format!("--- {n} matches ---\n"),
        (n, true) => format!("--- {n} matches, and the search stopped early ---\n"),
    };
    listing.write(ctx, &trailer);
}

/// Open the buffer called NAME that a search will write into, with its header
/// line in place. NAME is free already: the caller chose it.
fn open_results<B: BufferTrait>(
    ctx: &EditorState<B>,
    name: String,
    mode: &str,
    results: Results,
    env: &Arc<Env<EditorState<B>>>,
) -> String {
    ctx.new_buffer(&name, None, Some(mode.to_string()), env);
    ctx.append_to_buffer(&name, &format!("{} in {}\n", results.pattern, results.over));
    attach(ctx, &name, results);
    ctx.with_buffer_mut(&name, |buf| buf.read_only = true);
    name
}

pub const OCCUR_SCAN_DOC: &str = "(occur--scan PATTERN &optional REGEXP MODE): List every line of \
         this buffer matching PATTERN in a new buffer, in MODE or `occur-mode', and return its \
         name.";

primitive!(occur_scan, args, env, ctx, {
    let source_pattern = args::text(args.first())?;
    let regexp = args.get(1).is_some_and(|value| value.is_truthy());
    let mode = args::name_or(args.get(2), "occur-mode");
    let pattern = Pattern::new(
        &source_pattern,
        regexp,
        crate::feature::isearch::case_fold(&env),
    )
    .map_err(EvalError::RuntimeMessage)?;
    let source = ctx.get_current_buffer_name();
    let text = ctx.with_current_buffer(|buf| buf.text.to_string());
    ctx.consume_fuel(u32::try_from(text.chars().count()).unwrap_or(u32::MAX))?;

    let buffer = open_results(
        ctx,
        ctx.free_buffer_name("*Occur*"),
        &mode,
        Results::new(&source_pattern, &source),
        &env,
    );
    let listing = Listing {
        buffer: &buffer,
        writer: None,
    };
    let found = collect(
        ctx,
        listing,
        &pattern,
        KIND_BUFFER,
        &source,
        &text,
        DEFAULT_LIMIT,
    );
    finish(ctx, listing, found);
    ctx.set_current_results(Some(buffer.clone()));
    Ok(ELispExp::string(buffer))
});

pub const GREP_SCAN_DOC: &str = "(grep--scan PATTERN &optional DIRECTORY REGEXP MODE): Start \
         searching the files under DIRECTORY, or the directory the editor was started in, for \
         PATTERN, and return the name of the new buffer, in MODE or `occur-mode', that the \
         matches go into as they are found.";

primitive!(grep_scan, args, env, ctx, {
    let source_pattern = args::text(args.first())?;
    let directory = match args.get(1) {
        Some(ELispExp::String(path)) if !path.is_empty() => path.to_string(),
        _ => ".".to_string(),
    };
    let regexp = args.get(2).is_some_and(|value| value.is_truthy());
    let mode = args::name_or(args.get(3), "occur-mode");
    let pattern = Pattern::new(
        &source_pattern,
        regexp,
        crate::feature::isearch::case_fold(&env),
    )
    .map_err(EvalError::RuntimeMessage)?;
    let root = PathBuf::from(crate::primitives::io::expand_path(&directory));
    if !root.is_dir() {
        ctx.set_echo_message(&format!("{directory} is not a directory"));
        return Ok(ELispExp::nil());
    }

    // Named so that the search, which takes the name too, cannot stop
    // whatever work already had it.
    let buffer = open_results(
        ctx,
        ctx.free_name_for_work("*Grep*"),
        &mode,
        Results::new(&source_pattern, &root.to_string_lossy()),
        &env,
    );
    let search = TreeSearch {
        pattern,
        root,
        buffer: buffer.clone(),
        limit: DEFAULT_LIMIT,
    };
    if ctx
        .spawn_writer(&buffer, move |state, stop, me| search.run(state, stop, me))
        .is_none()
    {
        // Only as the editor shuts down.
        ctx.set_echo_message("The editor's background work has shut down");
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
        let (start, end) = crate::primitives::edits::line_bounds(&buf.text, line);
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
        // Said rather than opened: a path that is not there was written by a
        // build run somewhere else, and an empty buffer of the same name would
        // look like the file.
        if !Path::new(&crate::primitives::io::expand_path(&entry.source)).exists() {
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
    // A place is not opened over its own list: from the window showing the
    // list, it opens in the next one, so the list stays in view.
    if ctx.focused_window_buffer().as_deref() == Some(buffer) && ctx.window_count() > 1 {
        ctx.focus_other_window(1);
    }
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
    let here = ctx.get_current_buffer_name();
    ctx.with_buffer_mut(buffer, |buf| {
        buf.text.cursor_move(row, 0);
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

pub const RESULTS_VISIT_DOC: &str = "(results-visit N &optional BUFFER): Go to entry N of the \
         list in BUFFER: open what it is in, put point on its line, and mark the line. Return the \
         entry, or nil. `next-error' goes on from N.";

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

pub const NEXT_ERROR_DOC: &str = "(next-error): Go to the next place in the newest list -- a \
         search's, a compilation's -- opening it in the selected window, and return its entry. At \
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

/// Register this module's primitives: lists of places, and walking them.
///
/// Called by [`super::install_primitives`]. Here rather than there because a
/// primitive's name, its implementation and its argument spec are one fact in
/// three pieces, and they were two files apart.
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    // Lists of places, and walking them. See `primitives::results`.
    into.function("occur--scan", occur_scan, OCCUR_SCAN_DOC);
    into.function("grep--scan", grep_scan, GREP_SCAN_DOC);
    into.command("next-error", next_error, &[], NEXT_ERROR_DOC);
    into.command("previous-error", previous_error, &[], PREVIOUS_ERROR_DOC);
    into.function("results-count", results_count, RESULTS_COUNT_DOC);
    into.function("results-entry", results_entry, RESULTS_ENTRY_DOC);
    into.function("results-state", results_state, RESULTS_STATE_DOC);
    into.function("results-visit", results_visit, RESULTS_VISIT_DOC);
    into.function("results-buffer", results_buffer, RESULTS_BUFFER_DOC);
    into.command("results-select", results_select, &[], RESULTS_SELECT_DOC);
    into.function("results-put", results_put, RESULTS_PUT_DOC);
}
