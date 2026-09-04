//! Measurement primitives shared by both reports.
//!
//! Two kinds of number are taken here, and the difference between them is the
//! organising idea of this whole suite:
//!
//! * **Cost** -- fuel units. One per reduction step, plus one per element for
//!   primitives that walk a list. Exact integers, identical on every machine,
//!   reproducible to the unit. Assertions are made on these.
//! * **Time** -- wall clock. A property of the machine as much as of the code.
//!   These are tracked and compared against the previous run; only complexity
//!   *classes* are asserted on, never absolute durations.
use std::time::{Duration, Instant};

/// Timed samples taken per measurement.
const SAMPLES: usize = 7;

/// Time `f` and return the **fastest** of [`SAMPLES`] runs.
///
/// Minimum rather than mean or median, because interference on a shared machine
/// is one-sided: a descheduled sample, a page fault, a competing process or a
/// frequency dip only ever *adds* time, never removes it. The fastest run is
/// therefore the sample least polluted by anything that is not the code under
/// test. It is also markedly steadier run to run -- moving these benchmarks
/// from median to minimum cut their observed spread from 1.57x to 0.40x.
///
/// The first call is discarded as a warm-up, which pays for first-touch page
/// faults, lazily built state and cold branch predictors once rather than
/// charging them to whichever size happens to be measured first.
pub(crate) fn time_fastest<F: FnMut()>(mut f: F) -> Duration {
    f();
    (0..SAMPLES)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .expect("SAMPLES is non-zero")
}

/// Nanoseconds of `total` attributable to one unit of work.
pub(crate) fn per_unit_ns(total: Duration, units: u64) -> f64 {
    total.as_secs_f64() * 1e9 / units.max(1) as f64
}

/// `large / small`, guarded against a denominator too small for the clock to
/// have resolved.
///
/// Fed two measurements of the same operation at two sizes, the result names
/// the complexity class: ~2x for linear when the size doubles, ~4x for
/// quadratic. Fed two different operations, it is their relative cost.
pub(crate) fn ratio(large: f64, small: f64) -> f64 {
    large / small.max(f64::EPSILON)
}

/// Time for one round of a fixed, machine-independent integer workload.
///
/// Every wall-clock figure in the timing report is also expressed as a multiple
/// of this, which is what makes a slowdown legible. If one benchmark moved and
/// this did not, the code got slower. If everything moved together with this,
/// the *machine* was slower -- a loaded runner, a different build profile, a
/// thermally throttled laptop -- and nothing about the code changed at all.
///
/// It is a good yardstick for CPU-bound work and a poor one for anything
/// dominated by allocation or cache misses, which is why the report says so
/// rather than presenting the normalised column as a hardware-free truth.
pub(crate) fn calibration_ns() -> f64 {
    const ROUNDS: u64 = 2_000_000;

    let elapsed = time_fastest(|| {
        // A chained multiply-xorshift: each round depends on the previous one,
        // so it cannot be vectorised or hoisted, and `black_box` stops the
        // whole loop being deleted as dead.
        let mut x: u64 = 0x243F_6A88_85A3_08D3;
        for _ in 0..ROUNDS {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            x ^= x >> 33;
        }
        std::hint::black_box(x);
    });
    per_unit_ns(elapsed, ROUNDS)
}

/// A cost expressed as a multiple of the calibration round, at a precision that
/// stays readable across the four orders of magnitude these costs span.
pub(crate) fn x_ref(ns: f64, calibration: f64) -> String {
    let n = ns / calibration.max(f64::EPSILON);
    if n >= 100.0 {
        format!("{n:.0} x ref")
    } else if n >= 10.0 {
        format!("{n:.1} x ref")
    } else {
        format!("{n:.2} x ref")
    }
}
