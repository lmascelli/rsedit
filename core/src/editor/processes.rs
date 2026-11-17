//! The processes the editor talks with: the table of them by name, what each
//! shares with the threads that read and write it, and how their output
//! reaches Lisp -- through a bounded queue, a few deliveries at a time, on the
//! command thread between one command and the next.
//!
//! Design: `doc/architettura-editor.typ, <processi-dialoganti>`.
use super::*;
use crate::primitives::child_process::Process;
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};

/// The most deliveries a process's queue holds before its readers wait.
pub(crate) const QUEUE_ENTRIES: usize = 256;

/// The most bytes a process's queue holds before its readers wait. An empty
/// queue takes one delivery whatever its size, so that a message longer than
/// this still arrives.
pub(crate) const QUEUE_BYTES: usize = 4 * 1024 * 1024;

/// The most writes queued for a process's input before `process-send` refuses
/// one.
pub(crate) const INPUT_QUEUE: usize = 256;

/// The most deliveries one pass of [`EditorState::run_process_output`] makes.
pub(crate) const PASS_DELIVERIES: usize = 64;

/// How long one pass goes on making deliveries before it leaves the rest for
/// the next: the user's next key comes first.
const PASS_TIME: Duration = Duration::from_millis(10);

/// How a process's output is cut before it is delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Form {
    /// As it is read.
    Chunks,
    /// A line at a time, without its end.
    Lines,
    /// A message at a time, each framed by a `Content-Length` header.
    Messages,
}

/// How a process is, as `process-status` says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Running,
    Exited(i32),
    Killed,
}

/// One thing a process's function is called with.
#[derive(Debug, PartialEq)]
pub(crate) enum Delivery {
    /// A piece, a line or a message of its standard output.
    Output(String),
    /// A line of its standard error, or what went wrong reading it.
    Error(String),
    /// How it ended: always the last.
    Exit(Status),
}

impl Delivery {
    fn size(&self) -> usize {
        match self {
            Self::Output(text) | Self::Error(text) => text.len(),
            Self::Exit(_) => 0,
        }
    }
}

#[derive(Default)]
struct Inbox {
    queue: VecDeque<Delivery>,
    bytes: usize,
    /// The exit has been delivered: nothing more will be, and the name is
    /// free.
    over: bool,
}

/// What the threads of one process share with the editor: the process, the
/// queue its output waits in, the way to its input, and how it is.
///
/// Every lock here is a leaf: none is held while another is taken, or while
/// Lisp runs.
pub(crate) struct Exchange {
    process: Process,
    form: Form,
    inbox: Mutex<Inbox>,
    /// Signalled when a delivery leaves the queue, for the readers waiting
    /// for room.
    room: Condvar,
    /// The writing thread's queue; `None` once the input is closed.
    input: Mutex<Option<SyncSender<Vec<u8>>>>,
    status: Mutex<Status>,
    /// The process was asked to stop: it is said killed however it ends.
    stopped: AtomicBool,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Exchange {
    /// Returns the exchange of PROCESS, running, whose input INPUT writes and
    /// whose output is cut by FORM.
    pub(crate) fn new(process: Process, input: SyncSender<Vec<u8>>, form: Form) -> Self {
        Self {
            process,
            form,
            inbox: Mutex::new(Inbox::default()),
            room: Condvar::new(),
            input: Mutex::new(Some(input)),
            status: Mutex::new(Status::Running),
            stopped: AtomicBool::new(false),
        }
    }

    /// Returns the process.
    pub(crate) fn process(&self) -> &Process {
        &self.process
    }

    /// Returns how the process's output is cut.
    pub(crate) fn form(&self) -> Form {
        self.form
    }

    /// Queues DELIVERY, waiting while the queue is full, and returns whether
    /// it was queued: false when STOP was raised meanwhile, and the delivery
    /// dropped. Called from a reading thread, never from the command thread.
    pub(crate) fn push(&self, delivery: Delivery, stop: &Stop) -> bool {
        let size = delivery.size();
        let mut inbox = locked(&self.inbox);
        loop {
            if stop.is_cancelled() {
                return false;
            }
            let full = inbox.queue.len() >= QUEUE_ENTRIES || inbox.bytes + size > QUEUE_BYTES;
            if inbox.queue.is_empty() || !full {
                break;
            }
            // A timeout, so that a stop raised while it waits is seen.
            inbox = self
                .room
                .wait_timeout(inbox, Duration::from_millis(50))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        inbox.bytes += size;
        inbox.queue.push_back(delivery);
        true
    }

    /// Records how the process ended and queues it, past the queue's limits:
    /// the last delivery, which nothing waits behind.
    pub(crate) fn push_exit(&self, status: Status) {
        let status = if self.stopped.load(Ordering::Relaxed) {
            Status::Killed
        } else {
            status
        };
        *locked(&self.status) = status.clone();
        locked(&self.inbox).queue.push_back(Delivery::Exit(status));
    }

    /// Takes the next delivery off the queue, if there is one.
    fn take(&self) -> Option<Delivery> {
        let mut inbox = locked(&self.inbox);
        let delivery = inbox.queue.pop_front()?;
        inbox.bytes -= delivery.size();
        if matches!(delivery, Delivery::Exit(_)) {
            inbox.over = true;
        }
        drop(inbox);
        self.room.notify_all();
        Some(delivery)
    }

    /// Returns how many deliveries are waiting.
    pub(crate) fn queued(&self) -> usize {
        locked(&self.inbox).queue.len()
    }

    /// Returns whether the exit has been delivered.
    pub(crate) fn is_over(&self) -> bool {
        locked(&self.inbox).over
    }

    /// Queues BYTES for the process's input, and returns whether they were:
    /// false when the input is closed or its queue full. Never waits.
    pub(crate) fn send(&self, bytes: Vec<u8>) -> bool {
        let input = locked(&self.input);
        match input.as_ref().map(|input| input.try_send(bytes)) {
            Some(Ok(())) => true,
            Some(Err(TrySendError::Full(_) | TrySendError::Disconnected(_))) | None => false,
        }
    }

    /// Closes the process's input once what is queued for it is written, and
    /// returns whether it was open.
    pub(crate) fn close_input(&self) -> bool {
        locked(&self.input).take().is_some()
    }

    /// Returns how the process is.
    pub(crate) fn status(&self) -> Status {
        locked(&self.status).clone()
    }

    /// Returns whether the process has been told to finish, its input closed,
    /// and has not ended yet.
    fn is_finishing(&self) -> bool {
        locked(&self.input).is_none() && self.status() == Status::Running
    }

    /// Records that the process has been asked to stop.
    pub(crate) fn mark_stopped(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

/// A process the editor talks with, as the table holds it.
pub(crate) struct Dialogue<B: BufferTrait> {
    pub(crate) exchange: Arc<Exchange>,
    /// What each delivery is passed to.
    function: ELispExp<B>,
    /// The directory of the `with-directory` it was started in, which its
    /// function runs in.
    directory: Option<String>,
    /// The thread that reads its output, once it has started.
    pub(crate) worker: OnceLock<WorkId>,
}

/// The processes the editor talks with, by name.
pub(crate) struct Processes<B: BufferTrait> {
    table: Mutex<BTreeMap<String, Arc<Dialogue<B>>>>,
    /// Which process a pass starts with, so that one never waits behind the
    /// others for ever.
    turn: AtomicUsize,
}

impl<B: BufferTrait> Default for Processes<B> {
    fn default() -> Self {
        Self {
            table: Mutex::new(BTreeMap::new()),
            turn: AtomicUsize::new(0),
        }
    }
}

impl<B: BufferTrait> EditorState<B> {
    /// Returns the process called NAME, running or ended, if any.
    pub(crate) fn dialogue(&self, name: &str) -> Option<Arc<Dialogue<B>>> {
        locked(&self.processes.table).get(name).cloned()
    }

    /// Returns whether NAME is free for a new process: no process has it whose
    /// exit is still to be delivered, and no other work is called so.
    pub(crate) fn dialogue_name_free(&self, name: &str) -> bool {
        match self.dialogue(name) {
            Some(dialogue) => dialogue.exchange.is_over(),
            None => !self.work.is_named(name),
        }
    }

    /// Records EXCHANGE under NAME, with the FUNCTION its deliveries are
    /// passed to, in the directory the calling thread is scoped to; a process
    /// that had the name and is over is forgotten.
    pub(crate) fn add_dialogue(
        &self,
        name: &str,
        exchange: Arc<Exchange>,
        function: ELispExp<B>,
    ) -> Arc<Dialogue<B>> {
        let dialogue = Arc::new(Dialogue {
            exchange,
            function,
            directory: super::scoped_directory(),
            worker: OnceLock::new(),
        });
        locked(&self.processes.table).insert(name.to_string(), dialogue.clone());
        dialogue
    }

    /// Forgets the process called NAME: one whose reading thread never ran.
    pub(crate) fn forget_dialogue(&self, name: &str) {
        locked(&self.processes.table).remove(name);
    }

    /// Returns whether a process the editor talks with has had its input
    /// closed and is still running: one the exit gives time to end by itself.
    pub(crate) fn dialogue_finishing(&self) -> bool {
        locked(&self.processes.table)
            .values()
            .any(|dialogue| dialogue.exchange.is_finishing())
    }

    /// Returns whether a delivery is waiting: what [`Self::callbacks_owed`]
    /// adds to the calls owed.
    pub(crate) fn process_output_waiting(&self) -> bool {
        locked(&self.processes.table)
            .values()
            .any(|dialogue| dialogue.exchange.queued() > 0)
    }

    /// Makes the deliveries waiting, a process at a time in turn, until none
    /// is left, [`PASS_DELIVERIES`] are made or [`PASS_TIME`] has gone; and
    /// returns whether any was made. What is left waits for the next pass.
    ///
    /// Each is a call of the process's function, with its name, `output`,
    /// `error` or `exit`, and the text or the status, made as
    /// [`Self::run_owed_callbacks`] makes one: with a budget of its own, the
    /// locks free, and an error going to the log.
    pub(crate) fn run_process_output(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
        let mut dialogues: Vec<(String, Arc<Dialogue<B>>)> = locked(&self.processes.table)
            .iter()
            .filter(|(_, dialogue)| dialogue.exchange.queued() > 0)
            .map(|(name, dialogue)| (name.clone(), dialogue.clone()))
            .collect();
        if dialogues.is_empty() {
            return false;
        }
        let first = self.processes.turn.fetch_add(1, Ordering::Relaxed) % dialogues.len();
        dialogues.rotate_left(first);
        let began = Instant::now();
        let mut made = 0;
        'pass: loop {
            let mut any = false;
            for (name, dialogue) in &dialogues {
                if made >= PASS_DELIVERIES || began.elapsed() >= PASS_TIME {
                    break 'pass;
                }
                let Some(delivery) = dialogue.exchange.take() else {
                    continue;
                };
                any = true;
                made += 1;
                self.deliver(name, dialogue, delivery, env);
            }
            if !any {
                break;
            }
        }
        made > 0
    }

    fn deliver(
        &self,
        name: &str,
        dialogue: &Dialogue<B>,
        delivery: Delivery,
        env: &Arc<Env<EditorState<B>>>,
    ) {
        let (kind, data) = match delivery {
            Delivery::Output(text) => ("output", ELispExp::string(text)),
            Delivery::Error(text) => ("error", ELispExp::string(text)),
            Delivery::Exit(status) => ("exit", status_value(&status)),
        };
        let args = vec![
            ELispExp::string(name.to_string()),
            ELispExp::symbol(kind.to_string()),
            data,
        ];
        let _command = self.begin_command();
        let _scope = super::DirectoryScope::enter(dialogue.directory.clone());
        if let Err(error) = call_callable(&dialogue.function, &args, env.clone(), self) {
            self.log_diagnostic(&format!(
                "[ERROR] the function of process {name} failed: {error:?}"
            ));
        }
    }
}

/// Returns STATUS as Lisp has it: `run`, `(exit N)` or `killed`.
pub(crate) fn status_value<B: BufferTrait>(status: &Status) -> ELispExp<B> {
    match status {
        Status::Running => ELispExp::symbol("run".to_string()),
        Status::Exited(code) => ELispExp::proper_list(vec![
            ELispExp::symbol("exit".to_string()),
            ELispExp::number(*code as f64),
        ]),
        Status::Killed => ELispExp::symbol("killed".to_string()),
    }
}
