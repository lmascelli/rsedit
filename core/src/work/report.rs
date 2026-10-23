//! Reports: the `[BKG]` lines, and what keeps them from flooding the log. A
//! report says that a measurement crossed its threshold, and never changes
//! what a job does.
//!
//! Design: `doc/architettura-editor.typ, <misure>`.
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// What a report is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    /// A step took longer than `long-step`.
    LongStep,
    /// A ready job waited longer than `late-high` or `late-normal`.
    LateStart,
    /// An executor was busy for more than `busy-share` of a second.
    Load,
}

impl Rule {
    /// Returns what a summary calls occurrences of this rule: `long steps`.
    fn plural(self) -> &'static str {
        match self {
            Rule::LongStep => "long steps",
            Rule::LateStart => "late starts",
            Rule::Load => "busy seconds",
        }
    }

    /// Returns the unit of the value a summary calls the worst.
    fn unit(self) -> &'static str {
        match self {
            Rule::LongStep | Rule::LateStart => " ms",
            Rule::Load => "%",
        }
    }
}

/// The occurrences of one rule for one job since its window opened.
struct Window {
    opened: Instant,
    count: u64,
    worst: u64,
}

/// Turns occurrences into log lines, with the noise kept down.
///
/// The first occurrence of a rule for a job is written at once, and opens a
/// window of `summary-every`. Further occurrences inside the window are only
/// counted, keeping the worst value. When the window closes, a summary is
/// written if anything was counted, and a new window opens; a window that
/// closes empty ends, so the next occurrence after a quiet spell is written at
/// once again.
///
/// Failures -- an error, a panic, a wait that could never end -- are not
/// limited: a job that fails leaves, so it cannot repeat.
#[derive(Default)]
pub struct Reporter {
    windows: HashMap<(String, Rule), Window>,
    lines: Vec<String>,
}

impl Reporter {
    /// Records an occurrence of RULE for JOB, worth VALUE (milliseconds, or
    /// percent for a busy executor), with LINE to write if it is written at
    /// once.
    pub fn report(&mut self, job: &str, rule: Rule, value: u64, line: String, now: Instant) {
        match self.windows.get_mut(&(job.to_string(), rule)) {
            Some(window) => {
                window.count += 1;
                window.worst = window.worst.max(value);
            }
            None => {
                self.lines.push(line);
                self.windows.insert(
                    (job.to_string(), rule),
                    Window {
                        opened: now,
                        count: 0,
                        worst: 0,
                    },
                );
            }
        }
    }

    /// Writes LINE, whatever came before it.
    pub fn always(&mut self, line: String) {
        self.lines.push(line);
    }

    /// Closes the windows that are EVERY old: a summary for each that counted
    /// something, and an end for each that did not.
    pub fn tick(&mut self, now: Instant, every: Duration) {
        let mut summaries = Vec::new();
        self.windows.retain(|(job, rule), window| {
            if now.saturating_duration_since(window.opened) < every {
                return true;
            }
            if window.count == 0 {
                return false;
            }
            summaries.push(format!(
                "[BKG] summary: {job}, {} more {} in the last {} (worst {}{})",
                window.count,
                rule.plural(),
                span(every),
                window.worst,
                rule.unit(),
            ));
            *window = Window {
                opened: now,
                count: 0,
                worst: 0,
            };
            true
        });
        // Sorted so that two summaries closing together come out in the same
        // order every time.
        summaries.sort();
        self.lines.extend(summaries);
    }

    /// Returns when the earliest open window closes.
    pub fn next_tick(&self, every: Duration) -> Option<Instant> {
        self.windows
            .values()
            .map(|window| window.opened + every)
            .min()
    }

    /// Returns the lines written since the last call.
    pub fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }
}

/// Returns EVERY as a person would say it: seconds when it is whole seconds.
fn span(every: Duration) -> String {
    if every.as_millis().is_multiple_of(1000) {
        format!("{} s", every.as_secs())
    } else {
        format!("{} ms", every.as_millis())
    }
}
