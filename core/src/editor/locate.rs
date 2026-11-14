//! Looking upwards from a directory for the one that holds a name: the
//! answers already given, the climbs still out, and the wait that keeps a
//! disk that does not answer from stopping the caller.
//!
//! Design: `doc/architettura-editor.typ, <risalita>`.
use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// The variable that says how long a climb is waited for.
pub(crate) const TIMEOUT: &str = "locate-dominating-file-timeout";

/// How long a climb is waited for unless [`TIMEOUT`] says otherwise.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_millis(250);

/// How many climbs may be out at once. One more is refused rather than
/// started, so a disk that stopped answering holds this many threads, and no
/// more however many times it is asked.
pub(crate) const CLIMBS: usize = 4;

/// How many answers are kept before they are all dropped to start again.
const ANSWERS: usize = 4096;

/// What a climb is asked: where it starts, the names it looks for, and which
/// of the directories that hold one it answers with.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Question {
    /// The directory the climb starts in, absolute.
    pub(crate) start: String,
    pub(crate) names: Vec<String>,
    /// The highest directory that holds a name, rather than the nearest.
    pub(crate) topmost: bool,
}

/// What [`EditorState::locate`] answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Located {
    /// The directory, or `None` when no directory up to the root holds a
    /// name.
    Known(Option<String>),
    /// The disk has not answered yet, or too many climbs are out.
    Unanswered,
}

/// The answers given, and the questions whose climb has not come back.
#[derive(Default)]
pub(crate) struct Locate {
    state: Mutex<LocateState>,
}

#[derive(Default)]
struct LocateState {
    answers: HashMap<Question, Option<String>>,
    climbing: HashSet<Question>,
    /// Moved by every forgetting: a climb that started before one does not
    /// record what it found.
    generation: u64,
}

impl Locate {
    /// Runs F on the state. The lock is never held across the disk.
    fn with<R>(&self, f: impl FnOnce(&mut LocateState) -> R) -> R {
        f(&mut self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

/// Returns how long a climb is waited for: what [`TIMEOUT`] says, in seconds
/// and a hundredth at least, or [`DEFAULT_TIMEOUT`].
pub(crate) fn timeout<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Duration {
    match env.get_variable(TIMEOUT) {
        Some(ELispExp::Number(n)) if n.is_finite() && n >= 0.01 => {
            Duration::try_from_secs_f64(n).unwrap_or(DEFAULT_TIMEOUT)
        }
        _ => DEFAULT_TIMEOUT,
    }
}

/// Returns the nearest directory, from START upwards and START included,
/// that holds one of NAMES, or the highest when TOPMOST; `Some(None)` when
/// none does, and `None` when KEEP_GOING said no before the answer was known.
///
/// HOLDS is what asks the disk whether a path is there, and the only thing
/// here that can be slow; KEEP_GOING is asked before each directory. The
/// climb is by name: a symbolic link is not resolved, so the answer is a
/// prefix of START.
pub(crate) fn dominating_directory(
    question: &Question,
    holds: &dyn Fn(&std::path::Path) -> bool,
    keep_going: &dyn Fn() -> bool,
) -> Option<Option<String>> {
    let mut found = None;
    for directory in std::path::Path::new(&question.start).ancestors() {
        if !keep_going() {
            return None;
        }
        if question
            .names
            .iter()
            .any(|name| holds(&directory.join(name)))
        {
            found = Some(crate::paths::as_directory(&directory.to_string_lossy()));
            if !question.topmost {
                break;
            }
        }
    }
    Some(found)
}

impl<B: BufferTrait> EditorState<B> {
    /// Answers QUESTION from what is remembered, or climbs for it on a thread
    /// of its own and waits for that at most WAIT.
    ///
    /// A climb that outlasts WAIT goes on, and records its answer when it
    /// comes: until then the same question is [`Located::Unanswered`] at
    /// once, without a second thread or a second wait, and so is any
    /// question while [`CLIMBS`] climbs are out.
    pub(crate) fn locate(
        &self,
        question: Question,
        wait: Duration,
        holds: impl Fn(&std::path::Path) -> bool + Send + 'static,
    ) -> Located {
        enum Start {
            Known(Option<String>),
            Refused,
            Climb(u64),
        }
        let start = self.locate.with(|state| {
            if let Some(answer) = state.answers.get(&question) {
                Start::Known(answer.clone())
            } else if state.climbing.contains(&question) || state.climbing.len() >= CLIMBS {
                Start::Refused
            } else {
                state.climbing.insert(question.clone());
                Start::Climb(state.generation)
            }
        });
        let generation = match start {
            Start::Known(answer) => return Located::Known(answer),
            Start::Refused => return Located::Unanswered,
            Start::Climb(generation) => generation,
        };
        let (answered, answer) = std::sync::mpsc::sync_channel(1);
        let asked = question.clone();
        let started = self.spawn_work("locate", move |state, stop, _| {
            let found = dominating_directory(&asked, &holds, &|| !stop.is_cancelled());
            state.locate.with(|locate| {
                locate.climbing.remove(&asked);
                if let Some(found) = &found
                    && locate.generation == generation
                {
                    if locate.answers.len() >= ANSWERS {
                        locate.answers.clear();
                    }
                    locate.answers.insert(asked, found.clone());
                }
            });
            if let Some(found) = found {
                // Nobody listens once the wait is over.
                let _ = answered.send(found);
            }
        });
        if started.is_none() {
            self.locate.with(|state| state.climbing.remove(&question));
            return Located::Unanswered;
        }
        match answer.recv_timeout(wait) {
            Ok(found) => Located::Known(found),
            Err(_) => Located::Unanswered,
        }
    }

    /// Forgets the answers to the questions that start in DIRECTORY or under
    /// it, or every answer when there is none, and returns how many they
    /// were. A climb still out does not record what it finds.
    pub(crate) fn forget_located(&self, directory: Option<&str>) -> usize {
        let under = directory.map(std::path::Path::new);
        self.locate.with(|state| {
            state.generation += 1;
            let before = state.answers.len();
            match under {
                Some(under) => state.answers.retain(|question, _| {
                    !std::path::Path::new(&question.start).starts_with(under)
                }),
                None => state.answers.clear(),
            }
            before - state.answers.len()
        })
    }

    /// Returns how many climbs have not come back.
    #[cfg(test)]
    pub(crate) fn locate_climbs(&self) -> usize {
        self.locate.with(|state| state.climbing.len())
    }
}
