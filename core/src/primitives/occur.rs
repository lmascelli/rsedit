//! Searching a buffer, or the files under a directory, for a pattern, and
//! listing what is found: the producers whose view is `occur.lisp`.
//!
//! Design: `doc/architettura-editor.typ, <ricerche>`.
use super::*;
use crate::primitives::args;
use crate::primitives::results::attach;
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

/// Directories a tree search never goes into: the ones `find-file-recursive`
/// skips, large and generated, where a search would spend its limit before it
/// reached a line of source.
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

/// Returns the text of the buffer visiting PATH, when one is open with
/// changes the file does not have. A saved buffer is left to the read of its
/// file, which gives the same text without the buffer's lock.
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
    /// Changes the results attached to the buffer, only while it is still
    /// the writer's. See `EditorState::append_as_writer`.
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

    /// Appends TEXT to the buffer, as its writer when there is one.
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

/// Scans TEXT, adds what it finds to the set of LISTING's buffer, writes each
/// line into the buffer, and returns how many were added: none when the
/// buffer has gone, closed while the search ran, or is no longer the writer's.
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

/// A search of a tree, which runs on a thread of its own.
pub(crate) struct TreeSearch {
    pub(crate) pattern: Pattern,
    pub(crate) root: PathBuf,
    pub(crate) buffer: String,
    pub(crate) limit: usize,
}

impl TreeSearch {
    /// Walks the tree and lists what matches in the buffer, until the walk is
    /// done or STOP is raised. ME is the thread's number, which the buffer
    /// knows its writer by. Stopped, it ends as a search at its limit does:
    /// what was found stands, and the trailer says it stopped early.
    pub(crate) fn run<B: BufferTrait>(self, state: &EditorState<B>, stop: &Stop, me: WorkId) {
        let listing = Listing {
            buffer: &self.buffer,
            writer: Some(me),
        };
        let prune: HashSet<String> = PRUNE.iter().map(|name| name.to_string()).collect();
        let (paths, walk_truncated) =
            crate::primitives::filesystem::walk_files(&self.root, &prune, &[], FILE_LIMIT, &|| {
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
            // The buffer first, when one is open and ahead of the file.
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

/// Marks the set done and writes the trailer, once every line of the set is
/// in the buffer: a view that redrew a set marked done with lines still to
/// come would show an incomplete list as complete.
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

// TODO(RIC-7): occur and grep are two producers, so that a list of the one
// does not take the strip of the other.
/// Returns the buffer a new listing goes in, with its header written, RESULTS
/// attached and the buffer read-only, or `None` when there is none to have.
fn open_results<B: BufferTrait>(
    ctx: &EditorState<B>,
    producer: &str,
    stem: &str,
    mode: &str,
    results: Results,
    protected: Option<&str>,
    env: &Arc<Env<EditorState<B>>>,
) -> Option<String> {
    let name = ctx.prepare_results_buffer(producer, stem, mode, protected, env)?;

    ctx.append_to_buffer(&name, &format!("{} in {}\n", results.pattern, results.over));

    attach(ctx, &name, results);

    ctx.with_buffer_mut(&name, |buf| {
        buf.read_only = true;
    });

    Some(name)
}

pub const OCCUR_SCAN_DOC: &str = "(occur--scan PATTERN &optional REGEXP MODE): List every line of \
         this buffer matching PATTERN in an output buffer, in MODE or `occur-mode', and return its \
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

    let Some(buffer) = open_results(
        ctx,
        "occur",
        "*Occur*",
        &mode,
        Results::new(&source_pattern, &source),
        Some(&source),
        &env,
    ) else {
        return Ok(ELispExp::nil());
    };
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
         searching the files under DIRECTORY, or the working directory, for \
         PATTERN, and return the name of the output buffer, in MODE or `occur-mode', that the \
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
    let root = PathBuf::from(ctx.expand_path(&directory));
    if !root.is_dir() {
        ctx.set_echo_message(&format!("{directory} is not a directory"));
        return Ok(ELispExp::nil());
    }

    let Some(buffer) = open_results(
        ctx,
        "grep",
        "*Grep*",
        &mode,
        Results::new(&source_pattern, &root.to_string_lossy()),
        None,
        &env,
    ) else {
        return Ok(ELispExp::nil());
    };
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
        // Refused once the editor has shut down, or when the system gives no
        // thread.
        // TODO(DOC-1): the message speaks of a shutdown in both cases.
        ctx.set_echo_message("The editor's background work has shut down");
        return Ok(ELispExp::nil());
    }
    ctx.set_current_results(Some(buffer.clone()));
    Ok(ELispExp::string(buffer))
});

/// Registers the primitives of searches that make a list.
///
/// Called by [`super::install_primitives`].
pub(super) fn install<B: BufferTrait>(into: &Registry<B>) {
    into.function("occur--scan", occur_scan, OCCUR_SCAN_DOC);
    into.function("grep--scan", grep_scan, GREP_SCAN_DOC);
}
