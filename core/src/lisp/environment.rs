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

impl<T: LispContext> Clone for Env<T> {
    fn clone(&self) -> Self {
        unreachable!()
    }
}

impl<T: LispContext> PartialEq for Env<T> {
    fn eq(&self, _: &Env<T>) -> bool {
        unreachable!()
    }
}
