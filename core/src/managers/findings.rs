//! The findings: what producers have said about files -- a span, a severity,
//! a message, a code -- held by producer and by file, each producer's
//! findings in a file replaced as a whole, and asked for by file, by producer
//! or by directory.
//!
//! Design: `doc/architettura-editor.typ, <segnalazioni>`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

/// How much a finding matters, from most to least.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    /// Every severity, from most to least.
    pub const ALL: [Severity; 4] = [
        Severity::Error,
        Severity::Warning,
        Severity::Info,
        Severity::Hint,
    ];

    /// Returns the name Lisp gives the severity.
    pub fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        }
    }

    /// Returns the severity Lisp calls NAME.
    pub fn from_name(name: &str) -> Option<Severity> {
        Severity::ALL
            .into_iter()
            .find(|severity| severity.name() == name)
    }
}

/// A line, from 1, and a column, from 0, in characters.
pub type LineColumn = (usize, usize);

/// One thing a producer said about a file, where the producer said it.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub start: LineColumn,
    pub end: LineColumn,
    pub severity: Severity,
    pub message: String,
    pub code: Option<String>,
}

/// The markers that follow a set of findings in the buffer visiting their
/// file: one pair, start and end, for each finding, in the same order.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub markers: Vec<(u64, u64)>,
    /// The buffer's version when the markers were made: once it moves, the
    /// text has changed since the producer looked at it.
    pub version: u64,
}

impl Placed {
    /// Returns every marker id, starts and ends.
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.markers.iter().flat_map(|&(start, end)| [start, end])
    }
}

/// One producer's findings in one file.
#[derive(Clone, Debug)]
pub struct Set {
    pub findings: Vec<Finding>,
    /// Which replacement made this set: markers made for an older one are
    /// not this set's.
    pub generation: u64,
    pub placed: Option<Placed>,
    /// Whether the file's buffer was edited, and then closed, after the
    /// producer spoke: the markers that would say so are gone.
    pub edited: bool,
}

/// A set copied out, with the file and the producer it belongs to.
#[derive(Clone, Debug)]
pub struct Found {
    pub file: String,
    pub producer: String,
    pub set: Set,
}

/// What a replacement did: the generation of the new set, if there is one,
/// and the markers of the set it took the place of, for the caller to remove.
#[derive(Debug)]
pub struct Replaced {
    pub generation: Option<u64>,
    pub removed: Option<Placed>,
}

/// Every producer's findings in every file: by file, then by producer, with
/// the files of each producer beside them.
///
/// Files are absolute paths, made so before they come in.
#[derive(Default)]
pub struct Findings {
    by_file: BTreeMap<String, BTreeMap<String, Set>>,
    by_producer: HashMap<String, BTreeSet<String>>,
    next_generation: u64,
}

impl Findings {
    /// Puts FINDINGS in place of what PRODUCER said about FILE; none forgets
    /// it.
    pub fn replace(&mut self, producer: &str, file: &str, findings: Vec<Finding>) -> Replaced {
        if findings.is_empty() {
            return Replaced {
                generation: None,
                removed: self.remove(producer, file).and_then(|set| set.placed),
            };
        }
        self.next_generation += 1;
        let generation = self.next_generation;
        let old = self.by_file.entry(file.to_string()).or_default().insert(
            producer.to_string(),
            Set {
                findings,
                generation,
                placed: None,
                edited: false,
            },
        );
        self.by_producer
            .entry(producer.to_string())
            .or_default()
            .insert(file.to_string());
        Replaced {
            generation: Some(generation),
            removed: old.and_then(|set| set.placed),
        }
    }

    /// Forgets what PRODUCER said about FILE, or about every file when FILE
    /// is `None`, and returns the markers that went with it, by file.
    pub fn clear(&mut self, producer: &str, file: Option<&str>) -> Vec<(String, Placed)> {
        let files: Vec<String> = match file {
            Some(file) => vec![file.to_string()],
            None => self
                .by_producer
                .get(producer)
                .map(|files| files.iter().cloned().collect())
                .unwrap_or_default(),
        };
        files
            .into_iter()
            .filter_map(|file| {
                let placed = self.remove(producer, &file)?.placed?;
                Some((file, placed))
            })
            .collect()
    }

    /// Removes PRODUCER's set for FILE, and the entries that held only it.
    fn remove(&mut self, producer: &str, file: &str) -> Option<Set> {
        let sets = self.by_file.get_mut(file)?;
        let set = sets.remove(producer)?;
        if sets.is_empty() {
            self.by_file.remove(file);
        }
        if let Some(files) = self.by_producer.get_mut(producer) {
            files.remove(file);
            if files.is_empty() {
                self.by_producer.remove(producer);
            }
        }
        Some(set)
    }

    /// Records PLACED as the markers of PRODUCER's set for FILE, if the set
    /// is still the one of GENERATION; otherwise hands PLACED back, for the
    /// caller to remove: a newer replacement came in meanwhile.
    pub fn place(
        &mut self,
        producer: &str,
        file: &str,
        generation: u64,
        placed: Placed,
    ) -> Result<(), Placed> {
        match self
            .by_file
            .get_mut(file)
            .and_then(|sets| sets.get_mut(producer))
        {
            Some(set) if set.generation == generation => {
                set.placed = Some(placed);
                Ok(())
            }
            _ => Err(placed),
        }
    }

    /// Forgets the markers of FILE's sets, whose buffer is closing at
    /// VERSION: a set whose markers were made at another version keeps that
    /// it was edited.
    pub fn unplace(&mut self, file: &str, version: u64) {
        for set in self
            .by_file
            .get_mut(file)
            .into_iter()
            .flat_map(|sets| sets.values_mut())
        {
            if let Some(placed) = set.placed.take() {
                set.edited |= placed.version != version;
            }
        }
    }

    /// Returns the sets for FILE, by producer.
    pub fn for_file(&self, file: &str) -> Vec<Found> {
        self.by_file
            .get(file)
            .map(|sets| found(file, sets))
            .unwrap_or_default()
    }

    /// Returns PRODUCER's sets, by file.
    pub fn of_producer(&self, producer: &str) -> Vec<Found> {
        let Some(files) = self.by_producer.get(producer) else {
            return Vec::new();
        };
        files
            .iter()
            .filter_map(|file| {
                let set = self.by_file.get(file)?.get(producer)?;
                Some(Found {
                    file: file.clone(),
                    producer: producer.to_string(),
                    set: set.clone(),
                })
            })
            .collect()
    }

    /// Returns the sets for the files in DIRECTORY or below it, by file and
    /// then by producer. DIRECTORY is a whole path: `/a/b` holds `/a/b/c`,
    /// not `/a/bc`.
    pub fn under(&self, directory: &str) -> Vec<Found> {
        let directory = directory.trim_end_matches(std::path::is_separator);
        let prefix = format!("{directory}{}", std::path::MAIN_SEPARATOR);
        self.by_file
            .range(prefix.clone()..)
            .take_while(|(file, _)| file.starts_with(&prefix))
            .flat_map(|(file, sets)| found(file, sets))
            .collect()
    }
}

/// Returns the sets of FILE copied out, by producer.
fn found(file: &str, sets: &BTreeMap<String, Set>) -> Vec<Found> {
    sets.iter()
        .map(|(producer, set)| Found {
            file: file.to_string(),
            producer: producer.clone(),
            set: set.clone(),
        })
        .collect()
}
