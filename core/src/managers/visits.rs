//! The files visited, most recent first, and where point was in each when it
//! was last left: what `recent-files` lists and what `find-file` puts point
//! back to.
//!
//! Design: `doc/architettura-editor.typ, <memoria>`.

/// How many files `recent-files` lists when nothing says otherwise.
pub const DEFAULT_RECENT_FILES_LENGTH: usize = 50;

/// How many places are kept, whatever `recent-files-length` says: a place is
/// worth keeping for a file long gone from the recent list, and a cap keeps
/// the state file small.
pub const PLACES_LIMIT: usize = 400;

/// The recent files and the places, each a list with a path at most once,
/// newest first.
#[derive(Default)]
pub struct Visits {
    /// Absolute paths, newest first.
    recent: Vec<String>,
    /// Absolute paths with point in characters, newest first.
    places: Vec<(String, usize)>,
}

impl Visits {
    /// Records PATH as the file visited last, keeping LIMIT files.
    pub fn visited(&mut self, path: &str, limit: usize) {
        self.recent.retain(|known| known != path);
        self.recent.insert(0, path.to_string());
        self.recent.truncate(limit);
    }

    /// Returns the files visited, newest first.
    pub fn recent(&self) -> Vec<String> {
        self.recent.clone()
    }

    /// Forgets every file visited, and returns whether there was one; the
    /// places stay.
    pub fn forget_recent(&mut self) -> bool {
        let had = !self.recent.is_empty();
        self.recent.clear();
        had
    }

    /// Records that point was at POINT when PATH was left. Point at the start
    /// is where a file opens anyway, and is forgotten instead of kept.
    pub fn left_at(&mut self, path: &str, point: usize) {
        self.places.retain(|(known, _)| known != path);
        if point > 0 {
            self.places.insert(0, (path.to_string(), point));
            self.places.truncate(PLACES_LIMIT);
        }
    }

    /// Returns where point was when PATH was last left, if it is remembered.
    pub fn place(&self, path: &str) -> Option<usize> {
        self.places
            .iter()
            .find(|(known, _)| known == path)
            .map(|(_, point)| *point)
    }

    /// Returns every place remembered, newest first.
    pub fn places(&self) -> Vec<(String, usize)> {
        self.places.clone()
    }

    /// Puts RECENT and PLACES, newest first, behind what is already known, as
    /// [`crate::managers::History::restore`] does: a path already there keeps
    /// the newer entry. RECENT keeps LIMIT files.
    pub fn restore(&mut self, recent: Vec<String>, places: Vec<(String, usize)>, limit: usize) {
        for path in recent {
            if !self.recent.contains(&path) {
                self.recent.push(path);
            }
        }
        self.recent.truncate(limit);
        for (path, point) in places {
            if point > 0 && self.place(&path).is_none() {
                self.places.push((path, point));
            }
        }
        self.places.truncate(PLACES_LIMIT);
    }
}
