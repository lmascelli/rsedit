//! ========================================================================== //
//!               +------------------------------------------+
//!               |  Execution metering for the interpreter. |
//!               +------------------------------------------+
//! The evaluator charges the host on every step through
//! `LispContext::consume_fuel`, but how large a budget is, and when it
//! refills, is host *policy*. This module supplies the *mechanism* that policy
//! needs, so that each embedder does not have to re-derive the thread-local
//! bookkeeping, the nesting rules, and the arithmetic edge cases for itself.
//!
//! It knows nothing about context. A host decides what constitutes
//! one metered unit of work by wrapping it in [`FuelMeter::begin`].
//! ========================================================================== //

use std::{
    cell::Cell,
    sync::atomic::{AtomicU32, Ordering},
};

/// Default steps one metered scope may run before it is aborted.
///
/// This is a runaway-loop guard, not a resource quota: it exists so a mistyped
/// `(while t ...)` cannot hang its host forever. Sized from the interpreter
/// performance suite's measured cost per eval step -- roughly 100ns in release
/// and 500ns in debug -- so a runaway scope costs about a second of
/// unresponsiveness in release and a few seconds in debug before control comes
/// back. Ordinary work uses a vanishing fraction of it.
pub const DEFAULT_FUEL: u32 = 10_000_000;

thread_local! {
    /// Steps remaining for the metered scope running on *this* thread.
    ///
    /// Thread-local rather than shared state on the meter, for two reasons.
    /// Correctness: the `(spawn ...)` special form evaluates on a second
    /// thread while sharing the host context, so a shared counter would let
    /// that thread spend the foreground scope's budget -- and let a new
    /// foreground scope refill it underneath a running background one. Speed:
    /// only the owning thread can reach its own copy, so no synchronisation is
    /// required at all, making this a plain `Cell` load/store rather than an
    /// atomic read-modify-write on a line other cores may be contending for.
    ///
    /// The `const` initialiser avoids the hidden "has this thread initialised
    /// it yet?" branch a non-const one would add to every access. It starts at
    /// `DEFAULT_FUEL` rather than zero so that a thread which never opens a
    /// scope is still *bounded* -- failing safe -- instead of either dying on
    /// its first step or running unmetered.
    static FUEL: Cell<u32> = const { Cell::new(DEFAULT_FUEL) };

    /// Nesting depth of [`FuelMeter::begin`] on this thread. Only the 0 -> 1
    /// transition refills, so code that re-enters the evaluator cannot hand
    /// itself a fresh budget partway through an existing scope.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Guard returned by [`FuelMeter::begin`]; closes the scope when dropped.
///
/// RAII rather than paired begin/end calls so that an early return or a `?`
/// cannot leave the depth stuck above zero, which would permanently prevent
/// any further refill.
#[must_use = "the metered scope ends as soon as this guard is dropped"]
pub struct FuelScope<'a> {
    _meter: &'a FuelMeter,
}

impl Drop for FuelScope<'_> {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// A refillable execution budget shared by every thread of one host.
///
/// The configured budget lives here (shared, read once per scope); the
/// remaining count lives in thread-local storage (private, touched every
/// step). That split is deliberate: the hot counter never needs
/// synchronisation, while the setting stays global and adjustable.
/// What [`FuelMeter::consume`] reports when the budget is gone.
///
/// Deliberately not an `EvalError`: the meter is host-agnostic machinery and
/// has no business naming the interpreter's error type, which is generic over
/// the context. Callers map it to `EvalError::OutOfFuel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exhausted;

#[derive(Debug)]
pub struct FuelMeter {
    budget: AtomicU32,
}

impl FuelMeter {
    pub fn new(budget: u32) -> Self {
        Self {
            budget: AtomicU32::new(budget),
        }
    }

    pub fn consume(&self, amount: u32) -> Result<(), Exhausted> {
        FUEL.with(|fuel| match fuel.get().checked_sub(amount) {
            Some(remaining) => {
                fuel.set(remaining);
                Ok(())
            }
            None => Err(Exhausted),
        })
    }

    /// Open a metered scope, refilling this thread's budget if it is the
    /// outermost one. Nested calls only deepen the count.
    pub fn begin(&self) -> FuelScope<'_> {
        DEPTH.with(|depth| {
            if depth.get() == 0 {
                FUEL.set(self.budget.load(Ordering::Relaxed));
            }
            depth.set(depth.get() + 1);
        });
        FuelScope { _meter: self }
    }

    /// Arm the calling thread's budget without opening a scope, for a freshly
    /// spawned thread that has no enclosing scope to nest inside. Without this
    /// such a thread would run on the compile-time `DEFAULT_FUEL` rather than
    /// the host's configured budget.
    pub fn arm_thread(&self) {
        FUEL.set(self.budget.load(Ordering::Relaxed));
    }

    /// Set the budget future scopes receive, and top the current thread's
    /// remaining fuel up to it -- so code that knows it will be expensive can
    /// raise its own ceiling as its first act rather than having to restart.
    pub fn set_budget(&self, budget: u32) {
        self.budget.store(budget, Ordering::Relaxed);
        FUEL.set(budget);
    }
}

/// Steps left in the calling thread's current scope.
///
/// Exposed so that a host can *count* the work an evaluation did, rather than
/// only be told when it ran out. See [`measure`].
pub fn remaining() -> u32 {
    FUEL.get()
}

/// Run `body` with an effectively unlimited budget and report how much fuel it
/// spent, alongside its result.
///
/// # Why this exists
///
/// Wall-clock timings describe the machine that took them. Fuel counts describe
/// the *program*: one unit per reduction step, plus one per element for
/// primitives that walk a list. They are exact integers, identical on every
/// machine and reproducible to the unit, which makes them the only measurement
/// in this codebase that can be committed to git and diffed to show a
/// regression.
///
/// # Why it holds a scope of its own
///
/// `body` typically re-enters the host, and a host opens a metered scope per
/// command. Were this to run at depth zero, that inner `begin()` would see
/// depth 0, refill `FUEL` to the configured budget, and destroy the accounting
/// mid-measurement. Holding a scope for the whole measurement makes every
/// nested `begin()` a no-op refill-wise, so the counter only ever goes down.
///
/// The thread's real remaining fuel is saved and restored, so measuring cannot
/// hand the surrounding scope a larger budget than it started with.
pub fn measure<T, F: FnOnce() -> T>(meter: &FuelMeter, body: F) -> (T, u64) {
    let saved = FUEL.get();
    let scope = meter.begin();
    FUEL.set(u32::MAX);
    let value = body();
    let spent = u64::from(u32::MAX - FUEL.get());
    drop(scope);
    FUEL.set(saved);
    (value, spent)
}
