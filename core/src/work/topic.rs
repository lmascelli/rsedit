//! Topics: events as counters, with no data in them. Emitting one says "go
//! and look", and what the job looks at, the state, is the source of truth: an
//! emission is an atomic increment, and a burst of them is one number that
//! grew.
//!
//! Design: `doc/architettura-editor.typ, <argomenti>`.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// What an emission calls besides counting: how a sleeping executor is woken.
/// A no-op in the core's own tests, which look at the counts when they choose
/// to.
type Nudge = Arc<dyn Fn() + Send + Sync>;

/// The topics, by name.
pub struct Topics {
    by_name: RwLock<HashMap<String, Topic>>,
    nudge: Nudge,
}

/// One topic. Cheap to clone: every clone is the same counter.
#[derive(Clone)]
pub struct Topic(Arc<Inner>);

/// [`Topic::detached`].
impl Default for Topic {
    fn default() -> Self {
        Self::detached()
    }
}

struct Inner {
    name: String,
    count: AtomicU64,
    nudge: Nudge,
}

impl Default for Topics {
    fn default() -> Self {
        Self::with_nudge(|| {})
    }
}

impl Topics {
    /// Returns topics whose every emission also calls NUDGE.
    pub fn with_nudge(nudge: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            by_name: RwLock::new(HashMap::new()),
            nudge: Arc::new(nudge),
        }
    }

    /// Returns the topic called NAME, made on first use.
    ///
    /// A name resolved once, and the [`Topic`] kept, makes emitting through it
    /// touch no lock at all.
    pub fn get(&self, name: &str) -> Topic {
        if let Some(topic) = self.by_name.read().expect("topics").get(name) {
            return topic.clone();
        }
        self.by_name
            .write()
            .expect("topics")
            .entry(name.to_string())
            .or_insert_with(|| {
                Topic(Arc::new(Inner {
                    name: name.to_string(),
                    count: AtomicU64::new(0),
                    nudge: self.nudge.clone(),
                }))
            })
            .clone()
    }
}

impl Topic {
    /// Returns a topic in no registry: it counts, and wakes nobody. What a
    /// buffer has until it joins an editor, which gives it the editor's own.
    /// Nameless, since nothing can look it up.
    pub fn detached() -> Self {
        Topic(Arc::new(Inner {
            name: String::new(),
            count: AtomicU64::new(0),
            nudge: Arc::new(|| {}),
        }))
    }

    /// Says that something this topic stands for has changed.
    pub fn emit(&self) {
        // Release, paired with the Acquire in `count`: whatever was changed
        // before the emission is visible to a job that has seen the new count.
        self.0.count.fetch_add(1, Ordering::Release);
        (self.0.nudge)();
    }

    /// Returns how many times the topic has been emitted.
    pub fn count(&self) -> u64 {
        self.0.count.load(Ordering::Acquire)
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }
}
