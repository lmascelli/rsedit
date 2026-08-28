//! Execution metering for the interpreter.
//!
//! The evaluator charges the host on every step through
//! `LispContext::consume_fuel`, but how large a budget is, and when it
//! refills, is host *policy*. This module supplies the *mechanism* that policy
//! needs, so that each embedder does not have to re-derive the thread-local
//! bookkeeping, the nesting rules, and the arithmetic edge cases for itself.
//!
//! It knows nothing about editors, buffers or keystrokes. A host decides what
//! constitutes one metered unit of work by wrapping it in [`FuelMeter::begin`].

use super::EvalError;
use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

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

    pub fn consume(&self, amount: u32) -> Result<(), EvalError> {
        FUEL.with(|fuel| match fuel.get().checked_sub(amount) {
            Some(remaining) => {
                fuel.set(remaining);
                Ok(())
            }
            None => Err(EvalError::OutOfFuel),
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
