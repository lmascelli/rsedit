//! The WorkManager and its scheduler: where asynchronous work will run.
//!
//! This replaces `background/` one piece at a time; the design, the decisions
//! behind it and the plan are in `doc/workmanager.typ`. What is here so far is
//! the scheduler's *core* -- the contract a job is written against, the
//! queues, deadlines and topics, the rule that picks the next step, the
//! measurements and reports -- and the [`WorkManager`] built around it, with
//! its executor, its registry and the dedicated threads. Every editor has one;
//! no work runs on it yet. The jobs move to it in the next steps of the plan.
//!
//! # What the core is not
//!
//! It has no thread and no clock of its own. A caller drives it a step at a
//! time ([`Core::run_once`]) and it reads the time from the [`Clock`] it was
//! given. In the editor that is the WorkManager's executor thread and the
//! system clock; in the tests it is the test itself and a [`ManualClock`], so a
//! minute of scheduling runs in microseconds and nothing depends on how loaded
//! the machine is.
//!
//! # The parts
//!
//! - [`job`]: [`Job`], what a step answers ([`Step`]), why a job leaves
//!   ([`Exit`]), and the [`Spec`] a job is submitted with.
//! - [`core`]: [`Core`], which holds the jobs and decides what runs.
//! - [`manager`]: [`WorkManager`], which owns the core and runs it -- on an
//!   executor thread, or by hand in the tests -- and keeps the registry.
//! - [`threads`]: dedicated threads, and the [`Stop`] that asks one to end.
//! - [`topic`]: events as counters, with no data in them.
//! - [`options`]: the values that tune the scheduler while the editor runs.
//! - [`report`]: the `[BKG]` lines, and what keeps them from flooding the log.
//! - [`clock`]: where the time comes from.
pub mod clock;
pub mod core;
pub mod job;
pub mod manager;
pub mod options;
pub mod report;
pub mod threads;
pub mod topic;

pub use self::clock::{Clock, ManualClock, SystemClock};
pub use self::core::{Core, Place, WorkInfo};
pub use self::job::{Class, Exit, Job, JobError, Spec, Start, Step, StepContext, Wake, WorkId};
pub use self::manager::WorkManager;
pub use self::options::{OptionError, Options};
pub use self::threads::{Stop, ThreadInfo};
pub use self::topic::{Topic, Topics};
