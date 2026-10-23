//! The registry: everything running, by number and by name. The WorkManager's
//! answer to what exists, given at once and without asking the executor.
//!
//! Design: `doc/architettura-editor.typ, <registro>`.
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::Sender;
use std::time::Instant;

use super::core::WorkInfo;
use super::executor::Msg;
use super::job::WorkId;
use super::threads::{Dedicated, Stop};

/// Everything running, by number and by name. Lives behind the WorkManager's
/// registry lock.
#[derive(Default)]
pub(super) struct Registry {
    /// Set by `shutdown`: nothing is taken after.
    pub(super) closed: bool,
    /// One namespace for jobs and threads: a name means one thing, and
    /// stopping it by name stops whichever it is.
    pub(super) names: HashMap<String, WorkId>,
    /// The newest work submitted under each name, until it is stopped: kept
    /// after it finishes, unlike `names`. See
    /// [`WorkManager::is_latest`](super::WorkManager::is_latest).
    pub(super) latest: HashMap<String, WorkId>,
    /// The jobs, with what the executor last published about them.
    pub(super) jobs: BTreeMap<WorkId, WorkInfo>,
    pub(super) threads: BTreeMap<WorkId, Dedicated>,
}

/// What taking work out of the registry leaves to do once the lock is given
/// back.
pub(super) enum Withdrawn {
    /// There was nothing to take.
    Nothing,
    /// A job. The executor has been told.
    Job,
    /// A thread, by name, whose stop is to be raised.
    Thread(String, Stop),
}

impl Registry {
    /// Takes the work numbered ID out of the registry. A job leaves at once
    /// and the executor is told; a thread stays listed, as stopping, until it
    /// has finished. After `shutdown` there is nothing to take: everything has
    /// been told to stop already, and the executor must hear nothing more.
    pub(super) fn withdraw<C: 'static>(&mut self, id: WorkId, inbox: &Sender<Msg<C>>) -> Withdrawn {
        if self.closed {
            return Withdrawn::Nothing;
        }
        if let Some(job) = self.jobs.remove(&id) {
            self.forget_name(&job.name, id);
            self.silence(&job.name, id);
            let _ = inbox.send(Msg::Cancel(id));
            return Withdrawn::Job;
        }
        let Some(thread) = self.threads.get_mut(&id) else {
            return Withdrawn::Nothing;
        };
        thread.asked.get_or_insert_with(Instant::now);
        let (name, stop) = (thread.name.clone(), thread.stop.clone());
        self.forget_name(&name, id);
        self.silence(&name, id);
        Withdrawn::Thread(name, stop)
    }

    /// Takes the work called NAME out of the registry, as
    /// [`Self::withdraw`] does.
    pub(super) fn withdraw_named<C: 'static>(
        &mut self,
        name: &str,
        inbox: &Sender<Msg<C>>,
    ) -> Withdrawn {
        match self.names.get(name) {
            Some(&id) => self.withdraw(id, inbox),
            None => Withdrawn::Nothing,
        }
    }

    /// Frees NAME, if it still names ID.
    pub(super) fn forget_name(&mut self, name: &str, id: WorkId) {
        if self.names.get(name) == Some(&id) {
            self.names.remove(name);
        }
    }

    /// Records that ID has been stopped: it is no longer the latest under
    /// NAME, if it was.
    pub(super) fn silence(&mut self, name: &str, id: WorkId) {
        if self.latest.get(name) == Some(&id) {
            self.latest.remove(name);
        }
    }
}
