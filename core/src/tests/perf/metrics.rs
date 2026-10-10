//! Measurement primitives for the isolated performance suite.
use std::time::{Duration, Instant};

const SAMPLES: usize = 9;

/// Summary of repeated measurements. Deltas and ratios use the median; the
/// range is shown so readers can see how stable the samples were.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Measurement {
    pub min: Duration,
    pub median: Duration,
    pub max: Duration,
}

impl Measurement {
    pub fn as_secs_f64(self) -> f64 {
        self.median.as_secs_f64()
    }

    pub fn ns_per_unit(self, units: u64) -> f64 {
        self.median.as_secs_f64() * 1e9 / units.max(1) as f64
    }

    pub fn ns_range_per_unit(self, units: u64) -> (f64, f64) {
        let units = units.max(1) as f64;
        (
            self.min.as_secs_f64() * 1e9 / units,
            self.max.as_secs_f64() * 1e9 / units,
        )
    }

    pub fn display_ns_per_unit(self, units: u64, label: &str) -> String {
        let (min, max) = self.ns_range_per_unit(units);
        format!("{:.2} {label} [{min:.2}–{max:.2}]", self.ns_per_unit(units))
    }

    pub fn display_ms(self) -> String {
        let min = self.min.as_secs_f64() * 1e3;
        let median = self.median.as_secs_f64() * 1e3;
        let max = self.max.as_secs_f64() * 1e3;
        format!("{median:.2} ms [{min:.2}–{max:.2}]")
    }
}

fn summarize(mut samples: Vec<Duration>) -> Measurement {
    samples.sort_unstable();
    Measurement {
        min: samples[0],
        median: samples[samples.len() / 2],
        max: *samples.last().expect("samples are non-empty"),
    }
}

/// Run a timing sample repeatedly and return its median plus observed range.
/// The first invocation is a warm-up and is excluded from the result.
pub(crate) fn time_median<F: FnMut()>(mut f: F) -> Measurement {
    repeat_elapsed(|| {
        let start = Instant::now();
        f();
        start.elapsed()
    })
}

/// Repeat a workload that returns its own timed interval. This lets benchmarks
/// create a fresh fixture and validate it outside the measured interval.
pub(crate) fn repeat_elapsed<F: FnMut() -> Duration>(mut sample: F) -> Measurement {
    let _ = sample();
    summarize((0..SAMPLES).map(|_| sample()).collect())
}

/// Repeat two related measurements from the same fresh fixture per sample.
pub(crate) fn repeat_pair<F: FnMut() -> (Duration, Duration)>(
    mut sample: F,
) -> (Measurement, Measurement) {
    let _ = sample();
    let (left, right): (Vec<_>, Vec<_>) = (0..SAMPLES).map(|_| sample()).unzip();
    (summarize(left), summarize(right))
}

pub(crate) fn per_unit_ns(total: Measurement, units: u64) -> f64 {
    total.ns_per_unit(units)
}

/// `large / small`, using the median duration from each measurement.
pub(crate) fn ratio(large: f64, small: f64) -> f64 {
    large / small.max(f64::EPSILON)
}
