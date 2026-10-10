//! Standalone performance characterization for editor-owned work.
//!
//! Run only with:
//! `cargo test --manifest-path core/Cargo.toml tests::perf::editor_performance_suite -- --ignored --exact --test-threads=1`
//!
//! Interpreter-only benchmarks live in the `risp` repository. The fuel report
//! here measures Lisp fuel charged along the editor command path; it does not
//! account for Rust work such as buffer movement, layout or rendering.
mod editor;
mod metrics;
mod report;

use report::{Kind, Report};

#[test]
#[ignore = "run explicitly with the documented performance command"]
fn editor_performance_suite() {
    let mut cost = Report::new(
        Kind::Cost,
        "performance-cost.txt",
        "rsedit performance: Lisp fuel accounting",
        "Fuel units describe only work charged by the interpreter. They are exact and machine-independent; they do not measure the Rust cost of editor operations.".into(),
    );
    editor::cost(&mut cost);
    let mut failures = cost.finish();

    let mut timing = Report::new(
        Kind::Timing,
        "performance-timing.txt",
        "rsedit performance: elapsed time",
        "Each duration is the median of nine measured repetitions, with the observed min–max range shown. Ratios use medians. Timing targets are advisory; only functional and fuel-accounting checks fail the command.".into(),
    );
    editor::timing(&mut timing);
    failures.extend(timing.finish());

    assert!(
        failures.is_empty(),
        "{} behavioral or fuel-accounting check(s) failed; see performance-cost.txt and performance-timing.txt:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}
