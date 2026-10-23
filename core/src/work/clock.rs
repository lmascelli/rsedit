//! Where the core gets the time from: the real clock, or one the tests move.
//! With a [`ManualClock`] a deadline passes when the test says so, and a step
//! "takes" a hundred milliseconds by advancing the clock from inside it.
//!
//! Design: `doc/architettura-editor.typ, <prove-sfondo>`.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A source of the current time.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A clock that moves only when told to.
pub struct ManualClock {
    now: Mutex<Instant>,
}

impl ManualClock {
    /// Returns a clock standing at the moment it was made.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Instant::now()),
        })
    }

    /// Moves the clock forward by BY.
    pub fn advance(&self, by: Duration) {
        *self.now.lock().expect("manual clock") += by;
    }

    /// Moves the clock forward to TO. Never backward: a monotonic clock that
    /// went back would make every measurement of the core meaningless.
    pub fn advance_to(&self, to: Instant) {
        let mut now = self.now.lock().expect("manual clock");
        if to > *now {
            *now = to;
        }
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("manual clock")
    }
}
