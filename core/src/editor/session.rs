//! What the editor carries from one run to the next -- the answers given at
//! the prompts, the files visited and where point was left in each -- in one
//! file of the state directory, read by [`EditorState::load_session`] at start
//! and written by [`EditorState::save_session`] at exit.
//!
//! Design: `doc/architettura-editor.typ, <memoria>`.

use super::*;
use crate::editor::visits::{recent_files_length, saving_places};
use crate::feature::minibuffer::history_length;
use crate::paths::DirError;
use std::path::{Path, PathBuf};

/// The session file's name, in the state directory.
pub(crate) const SESSION_FILE: &str = "session";

/// The name of the Lisp variable that turns keeping the prompts' answers from
/// one run to the next off.
pub(crate) const SAVE_HISTORY: &str = "save-history";

/// The session file's first line: what it is, and the version of what follows.
const HEADER: &str = "rsedit-session 1";

/// Returns whether the prompts' answers are kept from one run to the next, as
/// Lisp has it now; unset counts as on.
pub(crate) fn saving_history<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> bool {
    env.flag(SAVE_HISTORY, true)
}

/// What a session file holds, each list newest first.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Session {
    /// Each prompt's key, with its answers.
    pub history: Vec<(String, Vec<String>)>,
    /// Absolute paths.
    pub recent: Vec<String>,
    /// Absolute paths, each with point in characters.
    pub places: Vec<(String, usize)>,
}

impl Session {
    /// Returns the file's text: the header, then a line for each answer, file
    /// and place, its fields apart by tabs and each field escaped.
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
    /// `save-history`, `recent-files-length` and `save-place` say now.
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
    }

    /// Writes the session file in STATE, the state directory, with what the
    /// settings say to keep: the answers given at the prompts when
    /// `save-history` is on, the recent files, and, when `save-place` is on,
    /// where point is in each buffer visiting a file and was in those
    /// closed. Returns whether it was written; when it was not, the log
    /// says why.
    pub fn save_session(
        &self,
        env: &Arc<Env<EditorState<B>>>,
        state: Result<PathBuf, DirError>,
    ) -> bool {
        let session = self.session_to_save(env);
        let path = match state {
            Ok(state) => state.join(SESSION_FILE),
            Err(why) => {
                self.log_diagnostic(&format!("[WARNING] No session written: {why}"));
                return false;
            }
        };
        // Every lock given back: the write is a disk's, whose time nobody
        // controls.
        match write_session_file(&path, &session.render()) {
            Ok(()) => true,
            Err(why) => {
                self.log_diagnostic(&format!(
                    "[WARNING] No session written to {}: {why}",
                    path.display()
                ));
                false
            }
        }
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
        }
    }
}
