//! The editor's state, in compartments: groups of fields under one lock each,
//! changed by operations rather than piece by piece. A compartment reaches no
//! other, hands out no guard, never evaluates Lisp, answers with a verdict
//! rather than acting elsewhere, and does nothing slow under its lock;
//! `EditorState` coordinates them.
//!
//! Design: `doc/architettura-editor.typ, <regole-compartimenti>`.
mod buffers;
mod commands;
mod findings;
mod history;
mod kept_values;
mod kill_yank;
mod loader;
mod log;
mod macros;
mod modes;
mod runtime;
mod visits;
mod windows;

pub use buffers::{BufferRemoved, BufferRenamed, Buffers};
pub use commands::Commands;
pub use findings::{Finding, Findings, Found, LineColumn, Placed, Severity};
pub(crate) use commands::{prefix_from_raw, raw_prefix};
pub use history::{DEFAULT_HISTORY_LENGTH, History, Recalled};
pub use kept_values::KeptValues;
pub use kill_yank::KillYank;
pub use loader::{FeatureState, Loader, Plugin};
pub use log::Log;
pub use macros::Macros;
pub use modes::{Binding, BindingSource, Modes};
pub use runtime::Runtime;
#[cfg(test)]
pub(crate) use visits::PLACES_LIMIT;
pub use visits::{DEFAULT_RECENT_FILES_LENGTH, Visits};
pub use windows::{ClickCount, Hit, MouseDrag, Scrolled, WindowRemoved, Windows};
