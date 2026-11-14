//! The values Lisp code keeps from one run to the next by name, each held as
//! the text `read` turns back into it: what `session-value` reads and
//! `set-session-value` writes.
//!
//! Design: `doc/architettura-editor.typ, <memoria>`.

use std::collections::BTreeMap;

/// Each name with the text of its value, in the order of the names.
#[derive(Default)]
pub struct KeptValues {
    values: BTreeMap<String, String>,
}

impl KeptValues {
    /// Returns the text of the value kept under NAME.
    pub fn get(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }

    /// Keeps TEXT, a value written readably, under NAME, in place of what
    /// NAME held.
    pub fn set(&mut self, name: &str, text: String) {
        self.values.insert(name.to_string(), text);
    }

    /// Forgets the value kept under NAME, and returns whether there was one.
    pub fn forget(&mut self, name: &str) -> bool {
        self.values.remove(name).is_some()
    }

    /// Returns every name with the text of its value, in the order of the
    /// names.
    pub fn all(&self) -> BTreeMap<String, String> {
        self.values.clone()
    }

    /// Keeps each of VALUES, in place of what its name held.
    pub fn restore(&mut self, values: BTreeMap<String, String>) {
        self.values.extend(values);
    }
}
