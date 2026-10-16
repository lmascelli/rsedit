//! The WorkManager and its scheduler: where asynchronous work will run.
//!
//! This replaces `background/` one piece at a time; the design, the decisions
//! behind it and the plan are in `doc/workmanager.typ`. What is here so far is
//! the scheduler's *core*: the contract a job is written against, the queues,
//! deadlines and topics, the rule that picks the next step, and the
//! measurements and reports. Nothing in the editor uses it yet -- the next step
//! of the plan builds the WorkManager around it and gives it a thread.
//!
//! # What the core is not
//!
//! It has no thread and no clock of its own. A caller drives it a step at a
//! time ([`Core::run_once`]) and it reads the time from the [`Clock`] it was
//! given. In the editor that will be an executor thread and the system clock;
//! in the tests it is the test itself and a [`ManualClock`], so a minute of
//! scheduling runs in microseconds and nothing depends on how loaded the
//! machine is.
//!
//! # The parts
//!
//! - [`job`]: [`Job`], what a step answers ([`Step`]), why a job leaves
//!   ([`Exit`]), and the [`Spec`] a job is submitted with.
//! - [`core`]: [`Core`], which holds the jobs and decides what runs.
//! - [`topic`]: events as counters, with no data in them.
//! - [`options`]: the values that tune the scheduler while the editor runs.
//! - [`report`]: the `[BKG]` lines, and what keeps them from flooding the log.
//! - [`clock`]: where the time comes from.
pub mod clock;
pub mod core;
pub mod job;
pub mod options;
pub mod report;
pub mod topic;

pub use self::clock::{Clock, ManualClock, SystemClock};
pub use self::core::{Core, Place, WorkInfo};
pub use self::job::{Class, Exit, Job, JobError, Spec, Start, Step, StepContext, Wake, WorkId};
pub use self::options::{OptionError, Options};
pub use self::topic::{Topic, Topics};
