//! The values that tune the scheduler, changeable while the editor runs.
//!
//! Lisp will reach them with `work-option` and `set-work-option`; the core
//! reads them at every choice it makes, so a change counts from the next step.
//! Times are in milliseconds. The defaults start from the 40 ms turn that
//! colouring and the renderer run at today.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::job::Class;

/// One option: its name as Lisp spells it, its default and the values it
/// accepts.
struct Def {
    name: &'static str,
    default: u64,
    min: u64,
    max: u64,
}

const HIGH_STEPS: usize = 0;
const EVENT_DELAY: usize = 1;
const LONG_STEP: usize = 2;
const LATE_HIGH: usize = 3;
const LATE_NORMAL: usize = 4;
const STUCK_STEP: usize = 5;
const BUSY_SHARE: usize = 6;
const SUMMARY_EVERY: usize = 7;
const WATCHDOG_EVERY: usize = 8;

/// Every option, in the order they are listed. The index of each is the
/// constant above with its name.
const DEFS: [Def; 9] = [
    // High-class steps in a row before a ready normal job gets its turn.
    Def {
        name: "high-steps",
        default: 4,
        min: 1,
        max: 1_000,
    },
    // How long after the first unseen event a waiting job is woken: what
    // gathers a burst of keystrokes into one step.
    Def {
        name: "event-delay",
        default: 20,
        min: 0,
        max: 60_000,
    },
    // A step longer than this is reported.
    Def {
        name: "long-step",
        default: 40,
        min: 1,
        max: 3_600_000,
    },
    // A ready job that waits longer than this is reported, by class: a normal
    // job waits for high-class steps by design.
    Def {
        name: "late-high",
        default: 40,
        min: 1,
        max: 3_600_000,
    },
    Def {
        name: "late-normal",
        default: 200,
        min: 1,
        max: 3_600_000,
    },
    // The watchdog reports a step still running after this long, and again
    // each time it has run twice as long; and a dedicated thread still
    // running this long after it was asked to stop.
    Def {
        name: "stuck-step",
        default: 1_000,
        min: 1,
        max: 3_600_000,
    },
    // An executor busy for more than this share of a second, in percent, is
    // reported.
    Def {
        name: "busy-share",
        default: 50,
        min: 1,
        max: 100,
    },
    // At most one summary per job and rule in this long.
    Def {
        name: "summary-every",
        default: 10_000,
        min: 1,
        max: 3_600_000,
    },
    // How often the watchdog wakes.
    Def {
        name: "watchdog-every",
        default: 250,
        min: 1,
        max: 60_000,
    },
];

/// Why an option could not be set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionError {
    /// No option has this name.
    Unknown(String),
    /// The value is outside what the option accepts.
    OutOfRange {
        name: &'static str,
        min: u64,
        max: u64,
    },
}

/// The scheduler's options.
///
/// Atomics rather than a lock: the core reads one at every choice it makes,
/// and a change made from Lisp must take effect without making the scheduler
/// wait for anybody.
pub struct Options {
    values: [AtomicU64; DEFS.len()],
}

impl Default for Options {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|i| AtomicU64::new(DEFS[i].default)),
        }
    }
}

impl Options {
    /// The value of NAME, or `None` when there is no such option.
    pub fn get(&self, name: &str) -> Option<u64> {
        Self::index(name).map(|i| self.value(i))
    }

    /// Set NAME to VALUE. Refused, with nothing changed, for a name that does
    /// not exist or a value outside the option's range.
    pub fn set(&self, name: &str, value: u64) -> Result<(), OptionError> {
        let i = Self::index(name).ok_or_else(|| OptionError::Unknown(name.to_string()))?;
        let def = &DEFS[i];
        if value < def.min || value > def.max {
            return Err(OptionError::OutOfRange {
                name: def.name,
                min: def.min,
                max: def.max,
            });
        }
        self.values[i].store(value, Ordering::Relaxed);
        Ok(())
    }

    /// Every option with its value, in a fixed order.
    pub fn all(&self) -> Vec<(&'static str, u64)> {
        DEFS.iter()
            .enumerate()
            .map(|(i, def)| (def.name, self.value(i)))
            .collect()
    }

    pub(crate) fn high_steps(&self) -> u64 {
        self.value(HIGH_STEPS)
    }

    pub(crate) fn event_delay(&self) -> Duration {
        self.millis(EVENT_DELAY)
    }

    pub(crate) fn long_step(&self) -> Duration {
        self.millis(LONG_STEP)
    }

    /// The longest a ready job of CLASS may wait before it is reported.
    pub(crate) fn late(&self, class: Class) -> Duration {
        match class {
            Class::High => self.millis(LATE_HIGH),
            Class::Normal => self.millis(LATE_NORMAL),
        }
    }

    pub(crate) fn stuck_step(&self) -> Duration {
        self.millis(STUCK_STEP)
    }

    /// In percent.
    pub(crate) fn busy_share(&self) -> u64 {
        self.value(BUSY_SHARE)
    }

    pub(crate) fn summary_every(&self) -> Duration {
        self.millis(SUMMARY_EVERY)
    }

    pub(crate) fn watchdog_every(&self) -> Duration {
        self.millis(WATCHDOG_EVERY)
    }

    fn index(name: &str) -> Option<usize> {
        DEFS.iter().position(|def| def.name == name)
    }

    fn value(&self, i: usize) -> u64 {
        self.values[i].load(Ordering::Relaxed)
    }

    fn millis(&self, i: usize) -> Duration {
        Duration::from_millis(self.value(i))
    }
}
