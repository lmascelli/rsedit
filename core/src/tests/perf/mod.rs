//! The performance suite.
//!
//! # What this replaces, and why
//!
//! There used to be two independent suites -- one for the interpreter, one for
//! the editor -- of fourteen `#[test]`s between them, each timing something and
//! asserting a ratio. Three of them rebuilt and re-timed the same `while` loop;
//! two typed the same keystrokes through the same path; one measured list
//! accumulation by subtracting two nearly equal timings, which is textbook
//! catastrophic cancellation and duly flaked. Running concurrently under
//! `cargo test`, they also measured each other.
//!
//! Worse, none of them could answer the question you actually have while
//! optimising: *did that change make things faster or slower?* A ratio bound
//! either holds or does not; it says nothing about direction or magnitude.
//!
//! So this suite is organised around three ideas.
//!
//! ## 1. Two kinds of number, two files
//!
//! * `performance-cost.txt` -- **fuel units**. One per reduction step, plus one
//!   per element for primitives that walk a list. Exact integers, identical on
//!   every machine, reproducible to the unit. Commit this file: `git diff` on
//!   it *is* the regression report, and every assertion in the suite is made
//!   against numbers from it.
//! * `performance-timing.txt` -- **wall clock**, for the parts that run inside
//!   Rust and are never driven by the interpreter, where there is nothing to
//!   count. Machine-dependent, so these are tracked and compared, and only
//!   complexity *classes* are asserted on.
//!
//! ## 2. Every report compares against the previous run
//!
//! Each file ends with a machine-readable block of its own numbers. The next
//! run parses that block before overwriting it and prints the delta beside each
//! row, so a regression shows up as `(+34.0% SLOWER)` rather than as a bound
//! that happens to still hold. The timing report also records a fingerprint of
//! the machine and build profile and refuses to compare across a mismatch.
//!
//! ## 3. Benchmarks are classified by the operation they measure
//!
//! Sections are operations -- the evaluator's core loop, calls, list
//! primitives, the parser, the gap buffer, layout, the command path,
//! concurrency, the execution budget -- and each operation is measured once, in
//! one place, in whichever of the two currencies actually describes it.
//!
//! ## One test, run in order
//!
//! The whole suite is a single `#[test]`, which is what lets it emit one
//! complete report per run and stops the timing benchmarks from polluting each
//! other by running in parallel. Every check still runs; failures are collected
//! and reported together, named, at the end.
#[cfg(test)]
mod editor;
#[cfg(test)]
mod interpreter;
#[cfg(test)]
mod metrics;
#[cfg(test)]
mod report;

#[cfg(test)]
mod suite {
    use super::{editor, interpreter, metrics, report};
    use crate::lisp::{Env, EvalError, FuelMeter, LispContext, LispExp, Parser, eval, measure};
    use report::{Kind, Report};
    use std::sync::Arc;

    /// The smallest host that can be metered: it counts fuel and does nothing
    /// else.
    ///
    /// Interpreter benchmarks run against this rather than against
    /// `EditorState`, so what they measure is the evaluator and nothing else --
    /// no buffers, no keymaps, no diagnostic log, no call-frame bookkeeping.
    /// When a number in the interpreter sections moves, it moved because the
    /// interpreter changed.
    #[derive(Debug, Clone)]
    pub(super) struct Meter {
        fuel: Arc<FuelMeter>,
    }

    impl Meter {
        fn new() -> Self {
            // Generous: measurement runs are deliberately large, and `measure`
            // overrides the remaining count anyway. This only has to be big
            // enough that nothing trips before the measurement starts.
            Self {
                fuel: Arc::new(FuelMeter::new(u32::MAX)),
            }
        }
    }

    impl PartialEq for Meter {
        fn eq(&self, other: &Self) -> bool {
            Arc::ptr_eq(&self.fuel, &other.fuel)
        }
    }

    impl LispContext for Meter {
        fn consume_fuel(&self, amount: u32) -> Result<(), EvalError<Meter>> {
            self.fuel.consume(amount).map_err(|_| EvalError::OutOfFuel)
        }

        fn log_diagnostic(&self, _msg: &str) {}
    }

    /// A parsed-and-ready interpreter benchmark environment.
    pub(super) struct Bench {
        pub ctx: Meter,
        pub env: Arc<Env<Meter>>,
    }

    impl Bench {
        pub fn new() -> Self {
            let env = Env::new_root();
            crate::lisp::setup_base_env(env.clone());
            Self {
                ctx: Meter::new(),
                env,
            }
        }

        /// Parse ahead of any measurement, so evaluation benchmarks measure
        /// evaluation and not parsing.
        pub fn parse(&self, src: &str) -> LispExp<Meter> {
            Parser::new(&format!("(progn {src})"))
                .next()
                .unwrap_or_else(|e| panic!("benchmark source failed to parse: {e:?}\n{src}"))
        }

        pub fn run(&self, ast: &LispExp<Meter>) -> LispExp<Meter> {
            eval(ast, self.env.clone(), &self.ctx).expect("benchmark script failed to evaluate")
        }

        /// Fuel spent evaluating `ast`. Exact, and identical on every machine.
        pub fn cost(&self, ast: &LispExp<Meter>) -> u64 {
            measure(&self.ctx.fuel, || {
                self.run(ast);
            })
            .1
        }
    }

    /// One operation measured at `n`, `2n` and `4n`.
    ///
    /// Three sizes rather than two, because the third turns a fuzzy ratio into
    /// an exact statement. For any affine cost `c(n) = a*n + b`, the first
    /// difference is `a*n` and the second is `2*a*n`, so [`Self::shape`] is
    /// **exactly 2.0** -- the constant term `b`, which is what makes a plain
    /// `c(2n)/c(n)` ratio drift at small sizes, cancels completely. Quadratic
    /// cost gives exactly 4.0 by the same arithmetic.
    ///
    /// That is why the cost report can assert equality-grade bounds where a
    /// wall-clock suite has to leave slack for noise.
    pub(super) struct Scaling {
        pub n: usize,
        pub c1: u64,
        pub c2: u64,
        pub c4: u64,
    }

    impl Scaling {
        /// Measure `build(size)` at `n`, `2n` and `4n`.
        pub fn measure(bench: &Bench, n: usize, build: impl Fn(usize) -> String) -> Self {
            let cost_at = |size: usize| bench.cost(&bench.parse(&build(size)));
            Self {
                n,
                c1: cost_at(n),
                c2: cost_at(n * 2),
                c4: cost_at(n * 4),
            }
        }

        /// Marginal cost of one more unit of work, with the fixed setup cost
        /// differenced away.
        pub fn per_unit(&self) -> f64 {
            (self.c2 as f64 - self.c1 as f64) / self.n as f64
        }

        /// 2.0 for linear, 4.0 for quadratic. See the type docs.
        pub fn shape(&self) -> f64 {
            let first = self.c2 as f64 - self.c1 as f64;
            (self.c4 as f64 - self.c2 as f64) / first.max(f64::EPSILON)
        }

        /// Cost measured at the three sizes, for the report's detail line.
        pub fn counts(&self) -> String {
            format!(
                "{} at {}, {} at {}, {} at {}",
                self.c1,
                self.n,
                self.c2,
                self.n * 2,
                self.c4,
                self.n * 4
            )
        }
    }

    /// How far [`Scaling::shape`] may sit from the ideal before a linear
    /// operation is called non-linear.
    ///
    /// Tight because the inputs are exact integers: the only slack needed is
    /// for costs that are affine in *two* variables at once (a list primitive
    /// charging per element while the loop around it charges per step).
    pub(super) const SHAPE_TOLERANCE: f64 = 0.02;

    /// The whole performance suite: measures everything in a fixed order,
    /// writes both reports, then fails naming every check that did not hold.
    #[test]
    fn performance_suite_writes_both_reports() {
        let mut failures = Vec::new();

        // Cost first: it does no timing, so it cannot be disturbed by, or
        // disturb, anything measured with a clock.
        let mut cost = Report::new(
            Kind::Cost,
            "performance-cost.txt",
            "rsedit -- cost report (exact, machine-independent)",
            COST_PREAMBLE.to_string(),
        );
        interpreter::cost(&mut cost);
        editor::cost(&mut cost);
        failures.extend(cost.finish());

        let calibration = metrics::calibration_ns();
        let mut timing = Report::new(
            Kind::Timing,
            "performance-timing.txt",
            "rsedit -- timing report (wall clock, machine-dependent)",
            format!(
                "{TIMING_PREAMBLE}\n\
                 reference:   one calibration round takes {calibration:.3} ns on this machine; \
                 the\n\
                 {:13}'x ref' column is each cost divided by that.",
                ""
            ),
        );
        timing.calibrate(calibration);
        timing.section(
            "CALIBRATION",
            "A fixed integer workload, timed the same way as everything below. It is\n\
             the control: if a row got slower and this did not, the code got slower;\n\
             if this moved too, the machine did.",
            vec![report::Row::timed(
                report::CALIBRATION_KEY,
                "reference round",
                calibration,
                format!("{calibration:.3} ns"),
                "the unit the 'x ref' column is in",
            )],
        );
        interpreter::timing(&mut timing, calibration);
        editor::timing(&mut timing, calibration);
        failures.extend(timing.finish());

        assert!(
            failures.is_empty(),
            "{} performance check(s) failed -- see performance-cost.txt and \
             performance-timing.txt for the full picture:\n  - {}",
            failures.len(),
            failures.join("\n  - ")
        );
    }

    const COST_PREAMBLE: &str = "\
Costs here are FUEL UNITS: one per reduction step, plus one per element for
primitives that walk a list. They are exact integers describing the work the
program does, so they are the same on every machine and reproducible to the
unit. Nothing in this file is a stopwatch reading.

That makes this file worth committing. A diff on it is a complete, noise-free
account of how a change altered the interpreter's workload -- which is the one
question a pass/fail suite cannot answer.

Scaling rows are measured at n, 2n and 4n. Differencing twice cancels the
fixed setup cost exactly, so a linear operation reports 2.000 and a quadratic
one 4.000, with no tolerance needed for noise.";

    const TIMING_PREAMBLE: &str = "\
These operations run inside Rust and are never driven by the interpreter, so
no fuel is charged and there is nothing exact to count. What follows is wall
clock, and wall clock describes the machine as much as the code.

Two things make it useful anyway. Deltas compare against the previous run in
the same environment, so a change of direction is visible. And every figure is
also given as a multiple of a calibration workload timed in the same run, so a
machine-wide slowdown can be told apart from a real regression.

Only complexity classes are asserted on here. Absolute durations are tracked,
never used as a pass/fail threshold: a wall-clock bound encodes the speed of
whatever machine wrote it, and goes red on a slower one while staying green
through a genuine regression on a faster one.";
}
