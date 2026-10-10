//! Compact Markdown reports and explicit, versionable baselines.
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const TIMING_NOISE: f64 = 0.10;
const BLESS_ENV: &str = "PERF_BLESS";
const MACHINE_ENV: &str = "PERF_MACHINE";
const BASELINE_MARKER: &str = "# baseline-v1";

pub(crate) struct Row {
    pub key: String,
    pub label: String,
    pub value: f64,
    pub display: String,
    pub note: String,
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
        }
    }

    pub fn timed(
        key: impl Into<String>,
        label: impl Into<String>,
        value: f64,
        display: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self::new(key, label, value, display, note)
    }
}

pub(crate) struct Section {
    pub title: String,
    pub blurb: &'static str,
    pub rows: Vec<Row>,
}

pub(crate) struct Verdict {
    pub label: String,
    pub detail: String,
    pub ok: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Cost,
    Timing,
}

impl Kind {
    fn fingerprint(self) -> String {
        match self {
            Self::Cost => "machine-independent fuel accounting".into(),
            Self::Timing => {
                let machine = machine_id().unwrap_or_else(|| "unidentified".into());
                let profile = if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                };
                let cpus = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(0);
                format!(
                    "{machine}; {}-{}; {profile}; {cpus} logical CPUs",
                    std::env::consts::ARCH,
                    std::env::consts::OS,
                )
            }
        }
    }
}

pub(crate) struct Report {
    kind: Kind,
    output_path: PathBuf,
    baseline_path: PathBuf,
    title: &'static str,
    preamble: String,
    fingerprint: String,
    sections: Vec<Section>,
    verdicts: Vec<Verdict>,
}

impl Report {
    pub fn new(kind: Kind, file_name: &str, title: &'static str, preamble: String) -> Self {
        // rsedit's manifest is in `core/`; risp's manifest is at the crate root.
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = if env!("CARGO_PKG_NAME") == "rsedit_core" {
            manifest.parent().unwrap_or(manifest)
        } else {
            manifest
        };
        let output_path = root.join(file_name);
        let baseline_path = root.join(file_name.replace(".txt", ".baseline"));
        Self {
            kind,
            output_path,
            baseline_path,
            title,
            preamble,
            fingerprint: kind.fingerprint(),
            sections: Vec::new(),
            verdicts: Vec::new(),
        }
    }

    pub fn section(&mut self, title: impl Into<String>, blurb: &'static str, rows: Vec<Row>) {
        self.sections.push(Section {
            title: title.into(),
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

    pub fn finish(self) -> Vec<String> {
        let previous = Baseline::load(&self.baseline_path);
        let comparable = match self.kind {
            Kind::Cost => previous.fingerprint.as_deref() == Some(self.fingerprint.as_str()),
            Kind::Timing => {
                machine_id().is_some()
                    && previous.fingerprint.as_deref() == Some(self.fingerprint.as_str())
            }
        };
        let failures: Vec<String> = self
            .verdicts
            .iter()
            .filter(|v| !v.ok)
            .map(|v| format!("{}: {}", v.label, v.detail))
            .collect();
        let bless = std::env::var(BLESS_ENV).is_ok_and(|v| !v.is_empty() && v != "0");

        let mut out = String::new();
        let _ = writeln!(out, "# {}\n", self.title);
        let _ = writeln!(out, "{}\n", self.preamble.trim());
        let _ = writeln!(out, "- Environment: `{}`", self.fingerprint);
        let baseline_status = if bless && failures.is_empty() {
            format!(
                "will save reviewed measurements to `{}`",
                self.baseline_path.display()
            )
        } else if bless {
            "not updated because a behavioral or fuel-accounting check failed".into()
        } else if comparable {
            format!("`{}`", self.baseline_path.display())
        } else if self.kind == Kind::Timing && machine_id().is_none() {
            format!("disabled; set `{MACHINE_ENV}` to a stable machine or runner label")
        } else if let Some(old) = previous.fingerprint.as_deref() {
            format!("different environment (`{old}`); deltas suppressed")
        } else {
            format!("none; set `{BLESS_ENV}=1` to save a reviewed baseline")
        };
        let _ = writeln!(out, "- Baseline: {baseline_status}\n");

        for section in &self.sections {
            let _ = writeln!(out, "## {}\n\n{}\n", section.title, section.blurb);
            let _ = writeln!(
                out,
                "| Measurement | Current value | Change | Interpretation |"
            );
            let _ = writeln!(out, "|---|---:|---:|---|");
            for row in &section.rows {
                let delta = if comparable {
                    describe_delta(self.kind, previous.get(&row.key), row.value)
                } else {
                    "—".into()
                };
                let _ = writeln!(
                    out,
                    "| {} | {} | {} | {} |",
                    row.label, row.display, delta, row.note
                );
            }
            let _ = writeln!(out);
        }

        let _ = writeln!(out, "## Behavioral and accounting checks\n");
        let _ = writeln!(out, "| Result | Check | Detail |");
        let _ = writeln!(out, "|---|---|---|");
        for verdict in &self.verdicts {
            let mark = if verdict.ok { "PASS" } else { "FAIL" };
            let _ = writeln!(out, "| {mark} | {} | {} |", verdict.label, verdict.detail);
        }
        let _ = writeln!(
            out,
            "\n{} of {} checks passed.\n",
            self.verdicts.len() - failures.len(),
            self.verdicts.len()
        );

        fs::write(&self.output_path, &out)
            .unwrap_or_else(|e| panic!("could not write {}: {e}", self.output_path.display()));

        if bless && failures.is_empty() {
            Baseline::save(&self.baseline_path, &self.fingerprint, &self.sections);
        }
        failures
    }
}

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
        if !text.lines().any(|line| line == BASELINE_MARKER) {
            return Self::default();
        }
        let mut baseline = Self::default();
        for line in text.lines() {
            if let Some(fp) = line.strip_prefix("fingerprint\t") {
                baseline.fingerprint = Some(fp.to_string());
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

    fn save(path: &Path, fingerprint: &str, sections: &[Section]) {
        let mut out = format!("{BASELINE_MARKER}\nfingerprint\t{fingerprint}\n");
        for row in sections.iter().flat_map(|s| &s.rows) {
            let _ = writeln!(out, "{}\t{}", row.key, row.value);
        }
        fs::write(path, out).unwrap_or_else(|e| panic!("could not save {}: {e}", path.display()));
    }
}

fn describe_delta(kind: Kind, previous: Option<f64>, now: f64) -> String {
    let Some(previous) = previous else {
        return "new".into();
    };
    if previous == now {
        return "unchanged".into();
    }
    if previous.abs() < f64::EPSILON {
        return "previously zero".into();
    }
    let pct = (now - previous) / previous.abs() * 100.0;
    if kind == Kind::Timing && pct.abs() <= TIMING_NOISE * 100.0 {
        format!("{pct:+.1}% (within 10% noise band)")
    } else {
        format!("{pct:+.1}%")
    }
}

fn machine_id() -> Option<String> {
    std::env::var(MACHINE_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
