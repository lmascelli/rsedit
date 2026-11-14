//! What the editor carries from one run to the next -- the answers given at
//! the prompts, the files visited, where point was left in each, and the
//! values Lisp code keeps by name -- in one file of the state directory, read by [`EditorState::load_session`] at start
//! and written by [`EditorState::save_session`] at exit and, in between, by
//! the [`SessionSaver`](crate::jobs::session_saver::SessionSaver). Each write
//! first takes in what another editor has added to the file.
//!
//! Design: `doc/architettura-editor.typ, <memoria>`.

use super::*;
use crate::editor::visits::{recent_files_length, saving_places};
use crate::feature::minibuffer::history_length;
use crate::paths::DirError;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The session file's name, in the state directory.
pub(crate) const SESSION_FILE: &str = "session";

/// The name of the Lisp variable that turns keeping the prompts' answers from
/// one run to the next off.
pub(crate) const SAVE_HISTORY: &str = "save-history";

/// The name of the Lisp variable holding how often the session is written
/// while the editor runs, in seconds.
pub(crate) const SESSION_SAVE_INTERVAL: &str = "session-save-interval";

/// How often the session is written when nothing says otherwise.
pub const DEFAULT_SESSION_SAVE_INTERVAL: f64 = 60.0;

/// The session file's first line: what it is, and the version of what follows.
const HEADER: &str = "rsedit-session 1";

/// Returns whether the prompts' answers are kept from one run to the next, as
/// Lisp has it now; unset counts as on.
pub(crate) fn saving_history<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(SAVE_HISTORY, true)
}

/// What a session file holds, each list newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Session {
    /// Each prompt's key, with its answers.
    pub history: Vec<(String, Vec<String>)>,
    /// Absolute paths.
    pub recent: Vec<String>,
    /// Absolute paths, each with point in characters.
    pub places: Vec<(String, usize)>,
    /// Names, each with the text `read` turns back into its value.
    pub values: BTreeMap<String, String>,
}

impl Session {
    /// Returns what THEIRS, the file as it is now, holds that neither BASE,
    /// the file as this editor last read or wrote it, nor OURS has: what
    /// another editor added meanwhile, in the file's order. What is in BASE
    /// and not in OURS was forgotten here, and is not news. A value is news
    /// when THEIRS changed it and OURS did not: OURS holds what BASE holds.
    pub(crate) fn news(theirs: &Session, base: &Session, ours: &Session) -> Session {
        fn ring<'a>(session: &'a Session, key: &str) -> &'a [String] {
            session
                .history
                .iter()
                .find(|(known, _)| known == key)
                .map_or(&[], |(_, entries)| entries.as_slice())
        }
        let mut news = Session::default();
        for (key, entries) in &theirs.history {
            let (base, ours) = (ring(base, key), ring(ours, key));
            let mut added: Vec<String> = Vec::new();
            for entry in entries {
                if !base.contains(entry) && !ours.contains(entry) && !added.contains(entry) {
                    added.push(entry.clone());
                }
            }
            if !added.is_empty() {
                news.history.push((key.clone(), added));
            }
        }
        for path in &theirs.recent {
            if !base.recent.contains(path) && !ours.recent.contains(path) {
                news.recent.push(path.clone());
            }
        }
        for place in &theirs.places {
            if !base.places.contains(place) && !ours.places.iter().any(|(path, _)| *path == place.0)
            {
                news.places.push(place.clone());
            }
        }
        for (name, text) in &theirs.values {
            let known = base.values.get(name);
            if known != Some(text) && ours.values.get(name) == known {
                news.values.insert(name.clone(), text.clone());
            }
        }
        news
    }

    /// Returns whether it holds nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.history.is_empty()
            && self.recent.is_empty()
            && self.places.is_empty()
            && self.values.is_empty()
    }

    /// Returns the file's text: the header, then a line for each answer,
    /// file, place and value, its fields apart by tabs and each field
    /// escaped.
    pub(crate) fn render(&self) -> String {
        let mut text = format!("{HEADER}\n");
        for (key, entries) in &self.history {
            for entry in entries {
                text.push_str(&format!("history\t{}\t{}\n", escape(key), escape(entry)));
            }
        }
        for path in &self.recent {
            text.push_str(&format!("recent\t{}\n", escape(path)));
        }
        for (path, point) in &self.places {
            text.push_str(&format!("place\t{}\t{point}\n", escape(path)));
        }
        for (name, value) in &self.values {
            text.push_str(&format!("value\t{}\t{}\n", escape(name), escape(value)));
        }
        text
    }

    /// Returns what TEXT holds, and how many of its lines were not
    /// understood and left out; `None` when its first line is not
    /// [`HEADER`], which a file of another version, or no session file,
    /// would have.
    pub(crate) fn parse(text: &str) -> Option<(Session, usize)> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return None;
        }
        let mut session = Session::default();
        let mut skipped = 0;
        for line in lines {
            let fields: Vec<String> = line.split('\t').map(unescape).collect();
            match fields.as_slice() {
                [kind, key, entry] if kind == "history" => match session.history.last_mut() {
                    Some((last, entries)) if last == key => entries.push(entry.clone()),
                    _ => session.history.push((key.clone(), vec![entry.clone()])),
                },
                [kind, path] if kind == "recent" => session.recent.push(path.clone()),
                [kind, path, point] if kind == "place" => match point.parse() {
                    Ok(point) => session.places.push((path.clone(), point)),
                    Err(_) => skipped += 1,
                },
                [kind, name, value] if kind == "value" => {
                    session.values.insert(name.clone(), value.clone());
                }
                [blank] if blank.is_empty() => {}
                _ => skipped += 1,
            }
        }
        Some((session, skipped))
    }
}

/// Returns TEXT with the backslash, the tab, the line feed and the carriage
/// return written as `\\`, `\t`, `\n` and `\r`: one field on one line.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// Returns what [`escape`] made TEXT from. A backslash before anything else
/// stands for itself.
fn unescape(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            plain.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => plain.push('\t'),
            Some('n') => plain.push('\n'),
            Some('r') => plain.push('\r'),
            Some(other) => plain.push(other),
            None => plain.push('\\'),
        }
    }
    plain
}

/// Writes TEXT to PATH whole or not at all. A file made here is readable by
/// its owner only, since the answers at a prompt can be anything typed; one
/// that is there keeps its permissions.
fn write_session_file(path: &Path, text: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    if !path.exists() {
        use std::os::unix::fs::OpenOptionsExt;
        // Made empty first so that the atomic write below, which gives the
        // new file the old one's permissions, gives it these.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            // Made meanwhile by another editor, whose permissions stand.
            Err(why) if why.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(why) => return Err(why),
        }
    }
    crate::buffer::disk::write_atomically(path, text.as_bytes())
}

impl<B: BufferTrait> EditorState<B> {
    /// Reads the session file in STATE, the state directory, and gives back
    /// what it holds -- the answers, the recent files, the places -- as
    /// `save-history`, `recent-files-length` and `save-place` say now. A
    /// value the file keeps under a name replaces what this run gave it.
    /// Returns whether a file was read.
    ///
    /// No directory, or no file, is the first run, and says nothing; any
    /// other failure, and lines not understood, are logged.
    pub fn load_session(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        state: Result<PathBuf, DirError>,
    ) -> bool {
        let path = match state {
            Ok(state) => state.join(SESSION_FILE),
            Err(DirError::NotFound(_)) => return false,
            Err(why) => {
                self.log_diagnostic(&format!("[WARNING] No session read: {why}"));
                return false;
            }
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => return false,
            Err(why) => {
                self.log_diagnostic(&format!(
                    "[WARNING] No session read from {}: {why}",
                    path.display()
                ));
                return false;
            }
        };
        let Some((session, skipped)) = Session::parse(&text) else {
            self.log_diagnostic(&format!(
                "[WARNING] {} is not a session file this editor reads; it will be written \
                 over at exit",
                path.display()
            ));
            return false;
        };
        if skipped > 0 {
            self.log_diagnostic(&format!(
                "[WARNING] {}: {skipped} lines not understood, left out",
                path.display()
            ));
        }
        // What the file held when it was read, whatever the settings keep of
        // it: what a later write compares the file with.
        *self
            .session_base
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = session.clone();
        self.restore_session(env, session);
        true
    }

    /// Gives back what SESSION holds, as the settings say now.
    fn restore_session(&self, env: &Arc<Env<EditorState<B>>>, session: Session) {
        if saving_history(env) {
            let limit = history_length(env);
            self.history_mut(|history| {
                for (key, entries) in session.history {
                    history.restore(&key, entries, limit);
                }
            });
        }
        let places = if saving_places(env) {
            session.places
        } else {
            Vec::new()
        };
        let limit = recent_files_length(env);
        self.visits_mut(|visits| visits.restore(session.recent, places, limit));
        self.kept_values_mut(|kept| kept.restore(session.values));
    }

    /// Writes the session file in STATE, the state directory, with what the
    /// settings say to keep: the answers given at the prompts when
    /// `save-history` is on, the recent files, and, when `save-place` is on,
    /// where point is in each buffer visiting a file and was in those
    /// closed, and the values kept by name. What another editor added to the
    /// file since this one last read or wrote it is taken in first, behind
    /// what this one knows; a value another editor changed replaces this
    /// one's only when this one has not changed it.
    /// Returns whether the file holds the session; when it does not, the log
    /// says why.
    pub fn save_session(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        state: Result<PathBuf, DirError>,
    ) -> bool {
        match self.write_session(env, state) {
            Ok(_) => true,
            Err(why) => {
                self.log_diagnostic(&format!("[WARNING] {why}"));
                false
            }
        }
    }

    /// Does what [`Self::save_session`] does, saying nothing: returns whether
    /// the file was written, false when it already held the session, or why
    /// it could not be.
    ///
    /// One write at a time, the exit's or the job's: the file as this editor
    /// last knew it is held for the whole of it. No lock of the editor's is
    /// held across the read or the write, which are a disk's, whose time
    /// nobody controls.
    pub(crate) fn write_session(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        state: Result<PathBuf, DirError>,
    ) -> Result<bool, String> {
        let path = state
            .map_err(|why| format!("No session written: {why}"))?
            .join(SESSION_FILE);
        let mut base = self
            .session_base
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A file that is not there, or not a session, has nothing to take in.
        let on_disk = std::fs::read_to_string(&path).ok();
        let theirs = on_disk
            .as_deref()
            .and_then(Session::parse)
            .map(|(session, _)| session)
            .unwrap_or_default();
        let mut session = self.session_to_save(env);
        let news = Session::news(&theirs, &base, &session);
        if !news.is_empty() {
            self.restore_session(env, news);
            session = self.session_to_save(env);
        }
        let text = session.render();
        let written = on_disk.as_deref() != Some(text.as_str());
        if written {
            write_session_file(&path, &text)
                .map_err(|why| format!("No session written to {}: {why}", path.display()))?;
        }
        *base = session;
        Ok(written)
    }

    /// Starts the job that writes the session file in STATE every
    /// `session-save-interval` seconds, so that an editor that stops without
    /// being asked loses a few minutes of its memory and not the whole run.
    /// For the host, after [`Self::load_session`].
    pub fn start_session_saver(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        state: Result<PathBuf, DirError>,
    ) {
        let saver = crate::jobs::session_saver::SessionSaver::new(env.clone(), state.ok());
        self.work.submit(saver.spec(), Box::new(saver));
    }

    /// Returns what [`Self::save_session`] writes, the open buffers' places
    /// recorded first.
    fn session_to_save(&self, env: &Arc<Env<EditorState<B>>>) -> Session {
        let history = if saving_history(env) {
            self.history(|history| history.rings())
        } else {
            Vec::new()
        };
        let places = if saving_places(env) {
            self.record_every_place(env);
            self.visits(|visits| visits.places())
        } else {
            Vec::new()
        };
        let limit = recent_files_length(env);
        let mut recent = self.visits(|visits| visits.recent());
        recent.truncate(limit);
        Session {
            history,
            recent,
            places,
            values: self.kept_values(|kept| kept.all()),
        }
    }
}
