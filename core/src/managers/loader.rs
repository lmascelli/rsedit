//! What has been loaded: the features provided, each with the file it came
//! from, the `require`s in progress, the functions waiting for a feature, and
//! the plugins found at start. F is a function waiting, which this keeps and
//! never calls.
//!
//! Design: `doc/architettura-editor.typ, <moduli-lisp>`.
use std::collections::HashMap;
use std::path::PathBuf;

/// How a feature's load went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeatureState {
    /// Provided, and in force.
    Loaded,
    /// A `require` of it failed, for the reason given. Not in force: the next
    /// `require` tries again.
    Failed(String),
}

/// A feature the registry knows of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feature {
    /// The name it is provided and required under.
    pub name: String,
    /// The file it came from, or `None` for one provided from no file: typed
    /// at `M-:`, or evaluated from a string.
    pub file: Option<PathBuf>,
    pub state: FeatureState,
}

/// Where a `require` stands that the registry has been told of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequireMark {
    /// How many features were known when it began: what a failure rolls back
    /// to.
    known: usize,
}

/// A plugin found at start: its directory, and what its `plugin.lisp` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    /// Its name, which is also its directory's.
    pub name: String,
    pub version: Option<String>,
    /// The features it needs, as its descriptor lists them. Not loaded for it:
    /// its own files `require` what they use.
    pub requires: Vec<String>,
    pub description: Option<String>,
    pub directory: PathBuf,
}

/// The features, the loads in progress and the plugins: see the module.
pub struct Loader<F> {
    /// Every feature provided or failed, oldest first. A feature provided
    /// again keeps its place.
    known: Vec<Feature>,
    /// The features being required, outermost first, each with its file.
    requiring: Vec<(String, PathBuf)>,
    /// The files being evaluated, outermost first: the last is the one a
    /// `provide` comes from.
    files: Vec<PathBuf>,
    /// The functions waiting for each feature, in the order they were added.
    after_load: HashMap<String, Vec<F>>,
    plugins: Vec<Plugin>,
}

impl<F> Default for Loader<F> {
    fn default() -> Self {
        Self {
            known: Vec::new(),
            requiring: Vec::new(),
            files: Vec::new(),
            after_load: HashMap::new(),
            plugins: Vec::new(),
        }
    }
}

impl<F> Loader<F> {
    // ------------------------------------------------------------------
    // What is loaded
    // ------------------------------------------------------------------

    /// Returns the feature NAME, if the registry knows of it.
    pub fn feature(&self, name: &str) -> Option<&Feature> {
        self.known.iter().find(|feature| feature.name == name)
    }

    /// Returns whether NAME has been provided, and is in force.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.feature(name)
            .is_some_and(|feature| feature.state == FeatureState::Loaded)
    }

    /// Returns whether NAME is being required: its file is being evaluated.
    pub fn is_requiring(&self, name: &str) -> bool {
        self.requiring.iter().any(|(being, _)| being == name)
    }

    /// Returns the names of the features in force, the newest first, as
    /// Emacs's `features' lists them.
    pub fn loaded(&self) -> Vec<String> {
        self.known
            .iter()
            .rev()
            .filter(|feature| feature.state == FeatureState::Loaded)
            .map(|feature| feature.name.clone())
            .collect()
    }

    // ------------------------------------------------------------------
    // Loading
    // ------------------------------------------------------------------

    /// Notes that FILE is being evaluated, until [`Self::leave_file`].
    pub fn enter_file(&mut self, file: PathBuf) {
        self.files.push(file);
    }

    /// Notes that the innermost file being evaluated is done.
    pub fn leave_file(&mut self) {
        self.files.pop();
    }

    /// Begins the `require` of NAME from FILE, and returns the mark that
    /// [`Self::end_require`] takes.
    ///
    /// # Errors
    ///
    /// When NAME is being required already: the chain of the `require`s, from
    /// the outermost to NAME again, which a file requiring itself, directly or
    /// not, would otherwise follow for ever.
    pub fn begin_require(&mut self, name: &str, file: PathBuf) -> Result<RequireMark, Vec<String>> {
        if self.is_requiring(name) {
            let mut chain: Vec<String> = self
                .requiring
                .iter()
                .map(|(being, _)| being.clone())
                .collect();
            chain.push(name.to_string());
            return Err(chain);
        }
        self.requiring.push((name.to_string(), file));
        Ok(RequireMark {
            known: self.known.len(),
        })
    }

    /// Ends the `require` of NAME that MARK began, failed for the reason
    /// FAILURE gives, or not.
    ///
    /// A failure is recorded against NAME, and takes away every feature first
    /// provided since MARK: a file stopped half-way through is not in force,
    /// and nor is what it loaded on its way, as Emacs has it. The failures
    /// recorded since are kept, for the log to be read against.
    pub fn end_require(&mut self, name: &str, mark: RequireMark, failure: Option<String>) {
        let file = match self.requiring.pop() {
            Some((being, file)) if being == name => Some(file),
            Some(other) => {
                // Not the innermost: ended out of order, which the editor
                // never does. Put back, and leave the stack as it was.
                self.requiring.push(other);
                None
            }
            None => None,
        };
        let Some(why) = failure else {
            return;
        };
        let mut index = 0;
        self.known.retain(|feature| {
            index += 1;
            index <= mark.known || feature.state != FeatureState::Loaded
        });
        let failed = Feature {
            name: name.to_string(),
            file,
            state: FeatureState::Failed(why),
        };
        match self.known.iter_mut().find(|feature| feature.name == name) {
            Some(feature) => *feature = failed,
            None => self.known.push(failed),
        }
    }

    /// Records NAME as provided, from the file being evaluated if any, and
    /// returns the functions that were waiting for it, for the caller to call.
    pub fn provide(&mut self, name: &str) -> Vec<F> {
        let file = self.files.last().cloned();
        let provided = Feature {
            name: name.to_string(),
            file,
            state: FeatureState::Loaded,
        };
        match self.known.iter_mut().find(|feature| feature.name == name) {
            Some(feature) => *feature = provided,
            None => self.known.push(provided),
        }
        self.after_load.remove(name).unwrap_or_default()
    }

    /// Keeps FUNCTION until NAME is provided; or, when it is already, hands it
    /// back for the caller to call now.
    pub fn after_load(&mut self, name: &str, function: F) -> Option<F> {
        if self.is_loaded(name) {
            return Some(function);
        }
        self.after_load
            .entry(name.to_string())
            .or_default()
            .push(function);
        None
    }

    // ------------------------------------------------------------------
    // Plugins
    // ------------------------------------------------------------------

    /// Replaces the plugins known with PLUGINS.
    pub fn set_plugins(&mut self, plugins: Vec<Plugin>) {
        self.plugins = plugins;
    }

    /// Returns the plugins found at start, in the order of their names.
    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
    }
}
