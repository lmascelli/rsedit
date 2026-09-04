//! Report model: rows, sections, and the writer that renders a file and
//! compares it against the previous run's copy of itself.
//!
//! # Why the reports compare against themselves
//!
//! A pass/fail suite answers "is it still correct?". It cannot answer "did that
//! commit make things faster or slower?", which is the question you actually
//! have while optimising. So every row carries a stable key, each report ends
//! with a machine-readable block of `key<TAB>value` lines, and the next run
//! parses that block before overwriting it. What you get back is the delta.
//!
//! The two files exist because the two kinds of number need different
//! treatment. A fuel count that moves by one unit is a real change worth
//! explaining; a wall-clock figure that moves by 8% is very likely the weather.
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// How much a wall-clock number may drift between runs on the same machine
/// before it is worth looking at. Below this the report says "noise" instead of
/// implying a regression.
const TIMING_NOISE: f64 = 0.10;

/// How much an exact cost may drift before it is called out. Cost figures are
/// integers computed from integers, so anything non-zero is a genuine change;
/// the tolerance exists only for costs derived through a division.
const COST_NOISE: f64 = 0.0;

/// One tracked measurement.
pub(crate) struct Row {
    /// Stable identifier used to line this row up with the previous run.
    /// Renaming it resets the row's history, so treat it as an API.
    pub key: String,
    /// Left-hand label as it appears in the table.
    pub label: String,
    /// The tracked number, in whatever unit `display` names.
    pub value: f64,
    /// The value as it should read, unit included.
    pub display: String,
    /// Short trailing commentary: "linear", "exact", "must stay under 3.0x".
    pub note: String,
    /// Whether this value scales with how fast the machine is.
    ///
    /// A duration does; a ratio between two durations taken in the same run
    /// does not. Only the former is worth correcting against the calibration
    /// round before a delta is computed -- see [`Report::finish`].
    pub machine_scaled: bool,
}

impl Row {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        value: f64,
        display: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value,
            display: display.into(),
            note: note.into(),
            machine_scaled: false,
        }
    }

    /// A row whose value is a duration, and therefore moves with the machine.
    pub fn timed(
        key: impl Into<String>,
        label: impl Into<String>,
        value: f64,
        display: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            machine_scaled: true,
            ..Self::new(key, label, value, display, note)
        }
    }
}

/// A group of rows measuring one operation, with a paragraph saying what the
/// operation is and why its cost matters.
pub(crate) struct Section {
    pub title: String,
    pub blurb: &'static str,
    pub rows: Vec<Row>,
}

/// A check that ran, and whether it held.
pub(crate) struct Verdict {
    pub label: String,
    pub detail: String,
    pub ok: bool,
}

/// Which report is being written, and therefore how a delta should be read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Exact fuel counts. Machine-independent; any movement is real.
    Cost,
    /// Wall clock. Machine-dependent; small movements are noise.
    Timing,
}

impl Kind {
    fn tolerance(self) -> f64 {
        match self {
            Kind::Cost => COST_NOISE,
            Kind::Timing => TIMING_NOISE,
        }
    }

    /// What has to match before two runs can be compared.
    ///
    /// For timings that is the machine and the build profile. For costs it is
    /// nothing at all: they are exact counts of work the program does, so a
    /// debug run and a release run on different hardware produce byte-identical
    /// numbers, and refusing to compare them would throw away the one property
    /// that makes the cost report worth keeping.
    fn fingerprint(self) -> String {
        match self {
            Kind::Cost => "any -- these counts are exact and machine-independent".into(),
            Kind::Timing => {
                let profile = if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                };
                let cores = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(0);
                format!(
                    "{}-{} {profile} {cores}-cpu",
                    std::env::consts::ARCH,
                    std::env::consts::OS,
                )
            }
        }
    }
}

pub(crate) struct Report {
    kind: Kind,
    path: PathBuf,
    title: &'static str,
    preamble: String,
    fingerprint: String,
    sections: Vec<Section>,
    verdicts: Vec<Verdict>,
    /// Cost of one calibration round in this run, when one was taken.
    calibration: Option<f64>,
}

/// Key of the calibration row, looked up in the baseline to correct every other
/// duration for how fast the machine was on each of the two runs.
pub(crate) const CALIBRATION_KEY: &str = "calibration/round";

/// Marker introducing the machine-readable trailer.
const DATA_MARKER: &str = "### baseline -- parsed by the next run; do not edit";
/// Marker introducing the environment line inside that trailer.
const FINGERPRINT_MARKER: &str = "### fingerprint\t";

impl Report {
    pub fn new(kind: Kind, file_name: &str, title: &'static str, preamble: String) -> Self {
        Self {
            kind,
            // `CARGO_MANIFEST_DIR` is `core/`, so the reports land beside the
            // workspace `Cargo.toml` rather than inside the crate, and stay put
            // wherever the suite is invoked from.
            path: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(file_name),
            title,
            preamble,
            fingerprint: kind.fingerprint(),
            sections: Vec::new(),
            verdicts: Vec::new(),
            calibration: None,
        }
    }

    /// Record this run's calibration round, which lets [`Self::finish`] tell a
    /// slower machine apart from slower code.
    pub fn calibrate(&mut self, ns: f64) {
        self.calibration = Some(ns);
    }

    /// Add a section. Numbering is assigned here rather than written into the
    /// titles, because the sections of one report are contributed by two
    /// independent modules and hand-numbering them collides the moment either
    /// one grows a section.
    pub fn section(&mut self, title: &str, blurb: &'static str, rows: Vec<Row>) {
        let n = self.sections.len() + 1;
        self.sections.push(Section {
            title: format!("{n}. {title}"),
            blurb,
            rows,
        });
    }

    pub fn verdict(&mut self, ok: bool, label: impl Into<String>, detail: impl Into<String>) {
        self.verdicts.push(Verdict {
            label: label.into(),
            detail: detail.into(),
            ok,
        });
    }

    /// Render the report, overwrite the file, and return every failed verdict.
    pub fn finish(self) -> Vec<String> {
        let previous = Baseline::load(&self.path);
        let comparable = previous.fingerprint.as_deref() == Some(self.fingerprint.as_str());

        // How much faster or slower the machine itself is than it was last
        // time. Durations are corrected by this before their delta is taken, so
        // a run that was uniformly 9% slower reports 9% on the calibration row
        // and ~0% everywhere else, instead of flagging every row as a
        // regression. Ratios are left alone: they already cancel the machine.
        let machine_drift = match (
            previous.get(CALIBRATION_KEY),
            self.calibration.filter(|_| comparable),
        ) {
            (Some(before), Some(now)) if before > 0.0 => now / before,
            _ => 1.0,
        };

        let mut out = String::new();
        let _ = writeln!(out, "{}\n{}\n", self.title, "=".repeat(self.title.len()));
        let _ = writeln!(out, "{}", self.preamble);
        let _ = writeln!(out, "environment: {}", self.fingerprint);
        match (&previous.fingerprint, comparable) {
            (None, _) => {
                let _ = writeln!(
                    out,
                    "baseline:    none -- this is the first run, so no deltas are shown."
                );
            }
            (Some(_), true) => {
                let _ = writeln!(
                    out,
                    "baseline:    {}",
                    match self.kind {
                        // Explicit padding rather than a wrapped literal: rustfmt
                        // rewrites a `\`-continued string into a raw multi-line one
                        // and bakes the source indentation into the output.
                        Kind::Cost => concat!(
                            "the previous run, wherever it happened; every delta\n",
                            "             below is a real change in the work done."
                        ),
                        Kind::Timing => concat!(
                            "the previous run in this same environment, so the\n",
                            "             deltas below are meaningful."
                        ),
                    }
                );
                if self.kind == Kind::Timing {
                    let _ = writeln!(
                        out,
                        "{:13}Durations are corrected for how fast the machine was on each run\n\
                         {:13}before their delta is taken, so a busy machine shows up as noise\n\
                         {:13}on the calibration row instead of as a regression on every\n\
                         {:13}other one. Ratios need no correction; they cancel it already.",
                        "", "", "", ""
                    );
                }
            }
            (Some(old), false) => {
                let _ = writeln!(
                    out,
                    "baseline:    a DIFFERENT environment ({old}), so nothing here can be\n\
                     {:13}compared against it. Deltas are suppressed; this run is now the\n\
                     {:13}baseline for the next one.",
                    "", ""
                );
            }
        }

        for section in &self.sections {
            let _ = writeln!(
                out,
                "\n{}\n{}",
                section.title,
                "-".repeat(section.title.len())
            );
            let _ = writeln!(out, "{}", section.blurb);
            let deltas: Vec<String> = section
                .rows
                .iter()
                .map(|row| {
                    if !comparable {
                        return String::new();
                    }
                    // The calibration row is the machine indicator itself, so
                    // it is always reported raw.
                    let scale = if row.machine_scaled && row.key != CALIBRATION_KEY {
                        machine_drift
                    } else {
                        1.0
                    };
                    describe_delta(
                        self.kind,
                        previous.get(&row.key).map(|v| v * scale),
                        row.value,
                    )
                })
                .collect();
            let width = |lengths: &mut dyn Iterator<Item = usize>| lengths.max().unwrap_or(0);
            let label_w = width(&mut section.rows.iter().map(|r| r.label.len()));
            let value_w = width(&mut section.rows.iter().map(|r| r.display.len()));
            // Zero when no baseline applies, so a first run has no dead column.
            let delta_w = width(&mut deltas.iter().map(|d| d.len()));
            for (row, delta) in section.rows.iter().zip(&deltas) {
                let _ = writeln!(
                    out,
                    "  {:<label_w$}   {:>value_w$}   {:<delta_w$}{}{}",
                    row.label,
                    row.display,
                    delta,
                    if delta_w == 0 { "" } else { "  " },
                    row.note,
                );
            }
        }

        let failures: Vec<String> = self
            .verdicts
            .iter()
            .filter(|v| !v.ok)
            .map(|v| format!("{}: {}", v.label, v.detail))
            .collect();

        let _ = writeln!(out, "\nVERDICT\n=======\n");
        for v in &self.verdicts {
            let mark = if v.ok { "PASS" } else { "FAIL" };
            let _ = writeln!(out, "  [{mark}] {}", v.detail);
        }
        let _ = writeln!(
            out,
            "\n  {} of {} checks passed.",
            self.verdicts.len() - failures.len(),
            self.verdicts.len()
        );

        let _ = writeln!(out, "\n{DATA_MARKER}");
        let _ = writeln!(out, "{FINGERPRINT_MARKER}{}", self.fingerprint);
        for section in &self.sections {
            for row in &section.rows {
                let _ = writeln!(out, "{}\t{}", row.key, row.value);
            }
        }

        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&self.path, out)
            .unwrap_or_else(|e| panic!("could not write {}: {e}", self.path.display()));
        failures
    }
}

/// The previous run's numbers, recovered from the trailer of the file this run
/// is about to overwrite.
#[derive(Default)]
struct Baseline {
    fingerprint: Option<String>,
    values: Vec<(String, f64)>,
}

impl Baseline {
    fn load(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        let Some(body) = text.split_once(DATA_MARKER).map(|(_, rest)| rest) else {
            return Self::default();
        };

        let mut baseline = Self::default();
        for line in body.lines() {
            if let Some(fp) = line.strip_prefix(FINGERPRINT_MARKER) {
                baseline.fingerprint = Some(fp.trim().to_string());
            } else if let Some((key, value)) = line.split_once('\t') {
                if let Ok(value) = value.trim().parse::<f64>() {
                    baseline.values.push((key.to_string(), value));
                }
            }
        }
        baseline
    }

    fn get(&self, key: &str) -> Option<f64> {
        self.values.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }
}

/// Render "how did this move since last time" for one row.
fn describe_delta(kind: Kind, previous: Option<f64>, now: f64) -> String {
    let Some(previous) = previous else {
        return "(new)".into();
    };
    if previous == now {
        return "(unchanged)".into();
    }
    if previous.abs() < f64::EPSILON {
        return "(was 0)".into();
    }

    let change = (now - previous) / previous.abs();
    let pct = change * 100.0;
    if change.abs() <= kind.tolerance() {
        return format!("({pct:+.1}% noise)");
    }
    // Every figure in both reports is a cost, so up is worse without exception.
    let direction = if change > 0.0 { "SLOWER" } else { "faster" };
    format!("({pct:+.1}% {direction})")
}
