//! The editor's state, in compartments: groups of fields under one lock each,
//! changed by operations rather than piece by piece. A compartment reaches no
//! other, hands out no guard, never evaluates Lisp, answers with a verdict
//! rather than acting elsewhere, and does nothing slow under its lock;
//! `EditorState` coordinates them.
//!
//! Design: `doc/architettura-editor.typ, <regole-compartimenti>`.
mod buffers;
mod commands;
mod history;
mod kill_yank;
mod log;
mod macros;
mod modes;
mod runtime;
mod windows;

pub use buffers::{BufferRemoved, BufferRenamed, Buffers};
pub use commands::Commands;
pub(crate) use commands::{prefix_from_raw, raw_prefix};
pub use history::{DEFAULT_HISTORY_LENGTH, History, Recalled};
pub use kill_yank::KillYank;
pub use log::Log;
pub use macros::Macros;
pub use modes::{Binding, BindingSource, Modes};
pub use runtime::Runtime;
pub use windows::{ClickCount, Hit, MouseDrag, Scrolled, WindowRemoved, Windows};
