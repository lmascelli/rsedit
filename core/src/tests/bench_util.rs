//! Timing helpers shared by the two performance suites.
//!
//! # The rule these suites follow
//!
//! **Every assertion is relative; absolute timings are printed, never
//! asserted.**
//!
//! A wall-clock threshold ("this must finish in under 50ms") encodes the speed
//! of whatever machine happened to write it. It goes red on a slower laptop, on
//! a loaded CI runner, and in a debug build, while staying green through a
//! genuine complexity regression on a fast machine -- failing in exactly the
//! situations it should not and passing in the one it should not.
//!
//! So instead every assertion here compares two measurements taken on the *same
//! machine in the same run*, which cancels the machine out:
//!
//! * **Complexity ratios** -- run the same operation at size N and 2N. Linear
//!   work grows ~2x, quadratic ~4x. That factor is a property of the algorithm,
//!   not of the CPU.
//! * **Relative cost** -- run operation A against operation B. "A function call
//!   costs under 8x an inline primitive" stays true whatever the clock speed.
//! * **Correctness invariants** -- "all 10,000 keystrokes reached the buffer" is
//!   timing-independent, and is what stops a benchmark from silently measuring
//!   a fast failure path instead of the work it claims to.
//!
//! Because of that these run in **debug** builds, where they are useful during
//! development. The printed numbers are pessimistic -- release is roughly 5x
//! faster -- but the ratios hold in both, so a suite that is green in debug is
//! green in release.
use std::time::{Duration, Instant};

/// Timed samples collected per measurement.
const SAMPLES: usize = 5;

/// Run `f` once un-timed to warm up (first-touch page faults, branch
/// predictors, lazily built state), then time it [`SAMPLES`] times and return
/// the **median**.
///
/// Median rather than a single shot or a mean: these tests share a machine with
/// whatever else is running, and one descheduled sample wrecks a mean while
/// barely moving a median. That is most of what keeps the ratios stable enough
/// to assert on.
pub(crate) fn time_median<F: FnMut()>(mut f: F) -> Duration {
    f();

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        f();
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    samples[SAMPLES / 2]
}

/// `large / small`, guarded against a zero denominator on a timer too coarse to
/// resolve the smaller measurement.
///
/// Feed this two measurements of the *same* operation at two sizes and the
/// result names the complexity class: ~2x for linear, ~4x for quadratic when
/// the size doubles.
pub(crate) fn growth(large: Duration, small: Duration) -> f64 {
    large.as_secs_f64() / small.as_secs_f64().max(f64::EPSILON)
}
