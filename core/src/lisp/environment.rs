//!
//!  Add a general description of the environment here
//!

use super::{LispContext, LispExp};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

// -------------------------------  Environment --------------------------------

#[derive(Debug)]
pub struct Env<T: LispContext> {
    pub variables: RwLock<HashMap<String, LispExp<T>>>,
    pub functions: RwLock<HashMap<String, LispExp<T>>>,
    pub macros: RwLock<HashMap<String, LispExp<T>>>,
    pub parent: Option<Arc<Env<T>>>,
}

impl<T: LispContext> Env<T> {
    pub fn new_root() -> Arc<Self> {
        Arc::new(Self {
            variables: RwLock::new(HashMap::new()),
            functions: RwLock::new(HashMap::new()),
            macros: RwLock::new(HashMap::new()),
            parent: None,
        })
    }

    pub fn new_child(parent: &Arc<Env<T>>) -> Arc<Self> {
        Arc::new(Self {
            variables: RwLock::new(HashMap::new()),
            functions: RwLock::new(HashMap::new()),
            macros: RwLock::new(HashMap::new()),
            parent: Some(parent.clone()),
        })
    }

    pub fn get_variable(&self, name: &str) -> Option<LispExp<T>> {
        if let Some(val) = self
            .variables
            .read()
            .expect("Failed to acquire read lock on env")
            .get(name)
        {
            return Some(val.clone());
        }

        if let Some(parent) = &self.parent {
            return parent.get_variable(name);
        }

        None
    }

    pub fn update_variable(&self, name: &str, val: LispExp<T>) -> bool {
        if self
            .variables
            .read()
            .expect("Failed to acquire read lock on env")
            .contains_key(name)
        {
            self.variables
                .write()
                .expect("Failed to acquire write lock on env")
                .insert(name.to_string(), val);
            return true;
        }
        if let Some(parent) = &self.parent {
            return parent.update_variable(name, val);
        }
        false
    }

    pub fn get_function(&self, name: &str) -> Option<LispExp<T>> {
        if let Some(val) = self
            .functions
            .read()
            .expect("Failed to acquire read lock on env")
            .get(name)
        {
            return Some(val.clone());
        }

        if let Some(parent) = &self.parent {
            return parent.get_function(name);
        }

        None
    }

    pub fn get_macro(&self, name: &str) -> Option<LispExp<T>> {
        if let Some(val) = self
            .macros
            .read()
            .expect("Failed to acquire read lock on env")
            .get(name)
        {
            return Some(val.clone());
        }

        if let Some(parent) = &self.parent {
            return parent.get_macro(name);
        }

        None
    }

    pub fn set_macro(&self, name: String, val: LispExp<T>) {
        let mut map = self
            .macros
            .write()
            .expect("Failed to acquire write lock on env");
        map.insert(name, val);
    }

    pub fn set_variable(&self, name: String, val: LispExp<T>) {
        let mut map = self
            .variables
            .write()
            .expect("Failed to acquire write lock on env");
        map.insert(name, val);
    }

    pub fn set_function(&self, name: String, val: LispExp<T>) {
        let mut map = self
            .functions
            .write()
            .expect("Failed to acquire write lock on env");
        map.insert(name, val);
    }
}

/// Environments compare by **identity**, not by contents.
///
/// This exists only because `Lambda` derives `PartialEq` and holds an
/// `Arc<Env>`. It used to be `unreachable!()`, which made
/// `(equal (lambda (x) x) (lambda (x) x))` panic and take the editor with it --
/// reachable from a one-line program.
///
/// Identity is also the right answer rather than merely a safe one: two
/// environments with equal bindings are still different scopes, and comparing
/// them structurally would mean walking two parent chains and every binding in
/// them, which is expensive, and cyclic once a closure captures the scope that
/// holds it.
impl<T: LispContext> PartialEq for Env<T> {
    fn eq(&self, other: &Env<T>) -> bool {
        std::ptr::eq(self, other)
    }
}
