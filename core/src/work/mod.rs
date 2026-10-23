//! Where the editor's asynchronous work runs: the WorkManager, its scheduler
//! and its dedicated threads. Knows nothing of editors: the state a job works
//! on is a type parameter.
//!
//! Design: `doc/architettura-editor.typ, <sfondo>`.
pub mod clock;
pub mod core;
mod executor;
pub mod job;
pub mod manager;
pub mod options;
mod registry;
pub mod report;
pub mod threads;
pub mod topic;
mod watchdog;

pub use self::clock::{Clock, ManualClock, SystemClock};
pub use self::core::{Core, Place, WorkInfo};
pub use self::job::{Class, Exit, Job, JobError, Spec, Start, Step, StepContext, Wake, WorkId};
pub use self::manager::WorkManager;
pub use self::options::{OptionError, Options};
pub use self::threads::{Stop, ThreadInfo};
pub use self::topic::{Topic, Topics};
