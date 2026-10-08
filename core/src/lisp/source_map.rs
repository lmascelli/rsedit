//! Where the reader found each form, so that an error can say where it is.
//!
//! # Why a table beside the forms and not a field in them
//!
//! A location in every `LispExp` would make every value larger and every
//! clone of one dearer, in the mode that never looks at it. A form is an
//! `Arc<Vec<LispExp>>`, though, and an `Arc` has an address that does not
//! change and that every clone shares -- the body `defun` keeps is the very
//! allocation the reader made. So the address is enough to find a form
//! again, and the locations live here, in a table only debug mode fills and
//! only an error consults.
//!
//! # Why the address cannot be mistaken for another form's
//!
//! Each entry holds a `Weak` to the form it describes. A `Weak` keeps the
//! allocation -- not the form in it -- alive, so while the entry exists no
//! other form can be given that address. A freed form leaves an entry that
//! answers nothing and is swept away later.
use super::{LispContext, LispExp};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};

/// A place in a source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// The file the text was read from, or a name in angle brackets for text
    /// that has none: `<eval-string>`, `<M-:>`.
    pub file: Arc<str>,
    /// Where the `(` is. Counting from 1.
    pub line: u32,
    /// Counting from 1, in characters.
    pub column: u32,
    /// Where the `)` is. Counting from 1.
    pub end_line: u32,
    pub end_column: u32,
}

/// `file:line:column`, the shape compilers print and `next-error` reads.
impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

#[derive(Default)]
struct Table {
    /// Address of a form's allocation -> the form, weakly, and where it was.
    forms: HashMap<usize, (Weak<dyn Send + Sync>, Location)>,
    /// How large `forms` may grow before the entries of freed forms are
    /// swept out of it.
    sweep_at: usize,
}

/// The locations of the forms read in debug mode.
#[derive(Default)]
pub struct SourceMap {
    table: Mutex<Table>,
}

fn address<T: LispContext>(form: &Arc<Vec<LispExp<T>>>) -> usize {
    Arc::as_ptr(form) as *const () as usize
}

impl SourceMap {
    /// Remember that FORM was read at AT.
    pub fn record<T: LispContext>(&self, form: &Arc<Vec<LispExp<T>>>, at: Location) {
        let weak = Arc::downgrade(form);
        let weak: Weak<dyn Send + Sync> = weak;
        let mut table = self.table.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if table.forms.len() >= table.sweep_at {
            table.forms.retain(|_, (weak, _)| weak.strong_count() > 0);
            // Twice what is still alive, so a sweep costs a constant amount
            // per form recorded however many are kept.
            table.sweep_at = (table.forms.len() * 2).max(1024);
        }
        table.forms.insert(address(form), (weak, at));
    }

    /// Where FORM was read, if it was read in debug mode.
    pub fn locate<T: LispContext>(&self, form: &Arc<Vec<LispExp<T>>>) -> Option<Location> {
        let table = self.table.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (weak, at) = table.forms.get(&address(form))?;
        (weak.strong_count() > 0).then(|| at.clone())
    }
}