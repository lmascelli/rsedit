//! The files under a directory, found by a walk on a thread of its own and
//! kept per directory: the walk leaves out what its patterns and the ignore
//! files it meets name, and what it has found is there to be asked for while
//! it goes on.
//!
//! Design: `doc/architettura-editor.typ, <indice-file>`.
use super::ignore::{self, Pattern, Verdict};
use super::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

/// The variable that says how old an index may be before it is walked again.
pub(crate) const MAX_AGE: &str = "file-index-max-age";

/// How old an index may be unless [`MAX_AGE`] says otherwise.
pub(crate) const DEFAULT_MAX_AGE: Duration = Duration::from_secs(10);

/// How many files an index holds unless it is told otherwise.
pub(crate) const DEFAULT_LIMIT: usize = 100_000;

/// How many indexes are kept: one more drops the one asked for least lately.
pub(crate) const INDEXES: usize = 8;

/// How many walks may be out at once. An index that would need one more
/// waits for the next time it is asked for, so a disk that stopped answering
/// holds this many threads and no more.
pub(crate) const WALKS: usize = 4;

/// How many files a first walk finds before it shows them, at most.
const BATCH: usize = 512;

/// How long a first walk goes without showing what it found, at most.
const BATCH_EVERY: Duration = Duration::from_millis(20);

/// What a walk leaves out, and where it stops.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Rules {
    /// Lines in the syntax of a .gitignore, for the root of the walk, below
    /// what the ignore files say.
    pub(crate) exclude: Vec<String>,
    /// The names of the files whose lines are patterns for the directory
    /// they are in and the ones under it.
    pub(crate) ignore_files: Vec<String>,
    /// How many files the walk keeps before it stops.
    pub(crate) limit: usize,
}

/// How far an index has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    /// Its first walk is out, or waits to be: more may come.
    Walking,
    /// Every file the rules keep.
    Complete,
    /// The walk stopped at the limit.
    Truncated,
}

/// What a walk ended with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Walked {
    Complete,
    Truncated,
    /// KEEP_GOING said no.
    Stopped,
    /// The root is not a directory that can be read.
    NotADirectory,
}

/// The patterns of one directory of a walk, and those of the directories
/// above it, nearest first.
struct Scope {
    /// The directory, relative to the root and written with `/`; empty for
    /// the root.
    base: String,
    patterns: Vec<Pattern>,
    above: Option<Arc<Scope>>,
}

impl Scope {
    /// Whether PATH, relative to the root, whose last part is NAME, is left
    /// out: the nearest directory whose patterns name it decides.
    fn excludes(self: &Arc<Self>, path: &str, name: &str, directory: bool) -> bool {
        let mut scope = Some(self);
        while let Some(here) = scope {
            let relative = if here.base.is_empty() {
                Some(path)
            } else {
                path.strip_prefix(here.base.as_str())
                    .and_then(|rest| rest.strip_prefix('/'))
            };
            if let Some(relative) = relative
                && let Some(verdict) = ignore::verdict(&here.patterns, relative, name, directory)
            {
                return verdict == Verdict::Excluded;
            }
            scope = here.above.as_ref();
        }
        false
    }
}

/// Collects the files under ROOT that RULES keep, breadth first, as paths
/// relative to it written with `/`, and returns them sorted, with how the
/// walk ended.
///
/// A directory a pattern leaves out is not entered, so a `!` line cannot
/// bring back what is under it. KEEP_GOING is asked before each directory is
/// read; SHOW is given the files found since it was last called, at most
/// every [`BATCH`] files or [`BATCH_EVERY`], and at the end. Symbolic links
/// are listed, not followed, and a directory that cannot be read is skipped.
pub(crate) fn walk(
    root: &std::path::Path,
    rules: &Rules,
    keep_going: &dyn Fn() -> bool,
    show: &mut dyn FnMut(&[Arc<str>]),
) -> (Vec<Arc<str>>, Walked) {
    let mut found: Vec<Arc<str>> = Vec::new();
    let mut shown = 0;
    let mut last_shown = Instant::now();
    let given = Arc::new(Scope {
        base: String::new(),
        patterns: rules
            .exclude
            .iter()
            .filter_map(|line| ignore::parse_line(line))
            .collect(),
        above: None,
    });
    let mut pending = VecDeque::from([(root.to_path_buf(), String::new(), given)]);
    let mut first = true;
    let outcome = loop {
        let Some((directory, prefix, scope)) = pending.pop_front() else {
            break Walked::Complete;
        };
        if !keep_going() {
            break Walked::Stopped;
        }
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) if first => break Walked::NotADirectory,
            Err(_) => continue,
        };
        first = false;
        let mut entries: Vec<_> = entries
            .flatten()
            .filter_map(|entry| {
                let kind = entry.file_type().ok()?;
                Some((entry.file_name().to_string_lossy().to_string(), kind, entry))
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut patterns = Vec::new();
        for (name, kind, entry) in &entries {
            if !kind.is_dir() && rules.ignore_files.iter().any(|file| file == name) {
                // Read as bytes: an ignore file that is not UTF-8 still has
                // its ASCII lines.
                if let Ok(bytes) = std::fs::read(entry.path()) {
                    patterns.extend(ignore::parse(&String::from_utf8_lossy(&bytes)));
                }
            }
        }
        let scope = if patterns.is_empty() {
            scope
        } else {
            Arc::new(Scope {
                base: prefix.clone(),
                patterns,
                above: Some(scope),
            })
        };
        let mut truncated = false;
        for (name, kind, entry) in entries {
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let directory = kind.is_dir();
            if scope.excludes(&relative, &name, directory) {
                continue;
            }
            if directory {
                pending.push_back((entry.path(), relative, scope.clone()));
            } else if found.len() >= rules.limit {
                truncated = true;
                break;
            } else {
                found.push(relative.into());
            }
        }
        if found.len() - shown >= BATCH
            || (found.len() > shown && last_shown.elapsed() >= BATCH_EVERY)
        {
            show(&found[shown..]);
            shown = found.len();
            last_shown = Instant::now();
        }
        if truncated {
            break Walked::Truncated;
        }
    };
    if shown < found.len() {
        show(&found[shown..]);
    }
    found.sort();
    (found, outcome)
}

/// The indexes kept, and the walks out for them.
#[derive(Default)]
pub(crate) struct FileIndex {
    state: Mutex<IndexState>,
}

#[derive(Default)]
struct IndexState {
    indexes: HashMap<(String, Rules), Index>,
    /// How many walks have not come back, whichever index they were for.
    walks_out: usize,
    /// The number the next walk is given.
    next_walk: u64,
    /// Moved by every question: what says which index was asked for last.
    clock: u64,
}

struct Index {
    /// What the last walk found, sorted once it has ended; what the first
    /// walk has found so far while it is out.
    paths: Vec<Arc<str>>,
    /// How far `paths` has got.
    progress: Progress,
    /// When the walk that found `paths` began, once it has ended.
    walked_at: Option<Instant>,
    walk: Option<Walk>,
    asked: u64,
}

impl Index {
    /// Whether walk NUMBER is the one out for this index.
    fn walked_by(&self, number: u64) -> bool {
        self.walk.as_ref().is_some_and(|walk| walk.number == number)
    }
}

/// A walk out for an index.
struct Walk {
    number: u64,
    /// Lowered when the index is dropped: the walk stops at the next
    /// directory.
    wanted: Arc<AtomicBool>,
    /// Whether what it finds is shown as it goes, which only a first walk
    /// does: a walk again shows the old list until it ends.
    shows: bool,
}

/// What [`EditorState::file_index`] answers: how far the index has got, and
/// its files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Indexed {
    pub(crate) progress: Progress,
    pub(crate) paths: Vec<Arc<str>>,
}

impl FileIndex {
    /// Runs F on the state. The lock is never held across the disk.
    fn with<R>(&self, f: impl FnOnce(&mut IndexState) -> R) -> R {
        f(&mut self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

/// Returns how old an index may be before it is walked again: what
/// [`MAX_AGE`] says, in seconds, never for nil, and [`DEFAULT_MAX_AGE`] for
/// anything else.
pub(crate) fn max_age<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
    match env.get_variable(MAX_AGE) {
        Some(ELispExp::Number(n)) if n.is_finite() && n >= 0.0 => {
            Some(Duration::try_from_secs_f64(n).unwrap_or(DEFAULT_MAX_AGE))
        }
        Some(exp) if exp.is_nil() => None,
        _ => Some(DEFAULT_MAX_AGE),
    }
}

impl<B: BufferTrait> EditorState<B> {
    /// Answers with the index of ROOT, an absolute directory, under RULES, at
    /// once and without the disk: what is known of it so far.
    ///
    /// The first question starts a walk; a question about an index whose
    /// walk began MAX_AGE ago or more starts another, and until that one ends
    /// the answer is the old list. No walk starts while [`WALKS`] are out:
    /// the next question tries again.
    pub(crate) fn file_index(
        &self,
        root: &str,
        rules: &Rules,
        max_age: Option<Duration>,
    ) -> Indexed {
        let key = (root.to_string(), rules.clone());
        let (answer, start, dropped) = self.file_index.with(|state| {
            state.clock += 1;
            let asked = state.clock;
            let mut dropped = Vec::new();
            if !state.indexes.contains_key(&key) && state.indexes.len() >= INDEXES {
                let oldest = state
                    .indexes
                    .iter()
                    .min_by_key(|(_, index)| index.asked)
                    .map(|(key, _)| key.clone());
                if let Some(index) = oldest.and_then(|key| state.indexes.remove(&key)) {
                    dropped.extend(index.walk);
                }
            }
            let index = state.indexes.entry(key.clone()).or_insert_with(|| Index {
                paths: Vec::new(),
                progress: Progress::Walking,
                walked_at: None,
                walk: None,
                asked,
            });
            index.asked = asked;
            let due = index.walk.is_none()
                && match index.walked_at {
                    None => true,
                    Some(at) => max_age.is_some_and(|age| at.elapsed() >= age),
                };
            let start = if due && state.walks_out < WALKS {
                let walk = Walk {
                    number: state.next_walk,
                    wanted: Arc::new(AtomicBool::new(true)),
                    shows: index.walked_at.is_none(),
                };
                if walk.shows {
                    index.paths.clear();
                }
                let start = (walk.number, walk.wanted.clone(), walk.shows);
                index.walk = Some(walk);
                state.next_walk += 1;
                state.walks_out += 1;
                Some(start)
            } else {
                None
            };
            let answer = Indexed {
                progress: index.progress,
                paths: index.paths.clone(),
            };
            (answer, start, dropped)
        });
        for walk in dropped {
            walk.wanted.store(false, Ordering::Relaxed);
        }
        if let Some((number, wanted, shows)) = start {
            self.start_walk(key, number, wanted, shows);
        }
        answer
    }

    /// Starts walk NUMBER for the index KEY names, on a thread of its own.
    fn start_walk(&self, key: (String, Rules), number: u64, wanted: Arc<AtomicBool>, shows: bool) {
        let walking = key.clone();
        let started = self.spawn_work("file-index", move |state, stop, _| {
            let began = Instant::now();
            let keep_going = || wanted.load(Ordering::Relaxed) && !stop.is_cancelled();
            let mut show = |batch: &[Arc<str>]| {
                if shows {
                    state.file_index.with(|state| {
                        if let Some(index) = state.indexes.get_mut(&walking)
                            && index.walked_by(number)
                        {
                            index.paths.extend_from_slice(batch);
                        }
                    });
                    state.changed_in_background();
                }
            };
            let (root, rules) = &walking;
            let (paths, walked) = walk(std::path::Path::new(root), rules, &keep_going, &mut show);
            if walked == Walked::NotADirectory {
                state.log_diagnostic(&format!("Cannot index {root}: not a readable directory"));
            }
            state.file_index.with(|state| {
                state.walks_out -= 1;
                let Some(index) = state.indexes.get_mut(&walking) else {
                    return;
                };
                if !index.walked_by(number) {
                    return;
                }
                index.walk = None;
                let progress = match walked {
                    Walked::Complete | Walked::NotADirectory => Progress::Complete,
                    Walked::Truncated => Progress::Truncated,
                    // Asked to stop with nobody dropping the index: the next
                    // question walks again.
                    Walked::Stopped => return,
                };
                index.paths = paths;
                index.progress = progress;
                index.walked_at = Some(began);
            });
        });
        if started.is_none() {
            self.file_index.with(|state| {
                state.walks_out -= 1;
                if let Some(index) = state.indexes.get_mut(&key)
                    && index.walked_by(number)
                {
                    index.walk = None;
                }
            });
        }
    }

    /// Drops the indexes of DIRECTORY and of the directories under it, or
    /// every index when there is none, stopping their walks, and returns how
    /// many they were.
    pub(crate) fn forget_file_index(&self, directory: Option<&str>) -> usize {
        let under = directory.map(std::path::Path::new);
        let dropped = self.file_index.with(|state| {
            let mut dropped = Vec::new();
            state.indexes.retain(|(root, _), index| {
                let keep =
                    under.is_some_and(|under| !std::path::Path::new(root).starts_with(under));
                if !keep {
                    dropped.push(index.walk.take());
                }
                keep
            });
            dropped
        });
        let forgotten = dropped.len();
        for walk in dropped.into_iter().flatten() {
            walk.wanted.store(false, Ordering::Relaxed);
        }
        forgotten
    }

    /// Returns how many walks have not come back.
    #[cfg(test)]
    pub(crate) fn file_index_walks(&self) -> usize {
        self.file_index.with(|state| state.walks_out)
    }
}
